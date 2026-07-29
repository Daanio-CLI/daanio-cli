//! Background task execution manager
//!
//! Allows tools to run in the background and notify the agent when complete.
//! Uses file-based storage for crash resilience + event channel for real-time notifications.

use crate::bus::{
    BackgroundTaskCompleted, BackgroundTaskProgress, BackgroundTaskProgressEvent,
    BackgroundTaskProgressSource, BackgroundTaskStatus, Bus, BusEvent,
};
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::sync::{RwLock, watch};
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant as TokioInstant, MissedTickBehavior};

mod model;

pub use model::{
    BackgroundCleanupResult, BackgroundTaskEventKind, BackgroundTaskEventRecord,
    BackgroundTaskInfo, BackgroundTaskWaitReason, BackgroundTaskWaitResult, ExecutionTaskMetadata,
    RunningBackgroundProgress, TaskResult, TaskStatusFile, format_progress_display,
    format_progress_summary, render_progress_bar,
};
use model::{
    EXIT_MARKER_PREFIX, RunningTask, normalize_delivery, progress_equivalent,
    progress_event_record, progress_wait_reason, push_task_event, task_dir, terminal_event_record,
};

/// Manages background task execution
pub struct BackgroundTaskManager {
    tasks: Arc<RwLock<HashMap<String, RunningTask>>>,
    output_dir: PathBuf,
}

impl BackgroundTaskManager {
    fn process_container_from_status(
        status: &TaskStatusFile,
    ) -> Option<crate::execution::ProcessContainer> {
        let root_pid = status.execution.root_pid.or(status.pid)?;
        let kind = match status.execution.process_container.as_deref() {
            Some("process_group") => crate::execution::ProcessContainerKind::ProcessGroup,
            Some("job_object") => crate::execution::ProcessContainerKind::WindowsJobObject,
            Some("windows_process_tree") => {
                crate::execution::ProcessContainerKind::WindowsProcessTree
            }
            _ if cfg!(unix) => crate::execution::ProcessContainerKind::ProcessGroup,
            _ => crate::execution::ProcessContainerKind::WindowsProcessTree,
        };
        let process_group_id = status.execution.process_group_id.or_else(|| {
            matches!(&kind, crate::execution::ProcessContainerKind::ProcessGroup)
                .then_some(root_pid)
        });
        Some(crate::execution::ProcessContainer::from_persisted(
            kind,
            root_pid,
            process_group_id,
            status.execution.process_start_token.clone(),
        ))
    }

    fn launch_deadline_watchdog(
        &self,
        task_id: String,
        mut lease_rx: watch::Receiver<TokioInstant>,
        absolute_deadline: TokioInstant,
    ) {
        let manager = Self {
            tasks: Arc::clone(&self.tasks),
            output_dir: self.output_dir.clone(),
        };
        tokio::spawn(async move {
            loop {
                let lease_deadline = *lease_rx.borrow();
                let next_deadline = lease_deadline.min(absolute_deadline);
                tokio::select! {
                    _ = tokio::time::sleep_until(next_deadline) => {
                        let now = TokioInstant::now();
                        let reason = if now >= absolute_deadline {
                            "absolute_deadline_exceeded"
                        } else {
                            "lease_expired"
                        };
                        if manager
                            .cancel_with_grace(
                                &task_id,
                                crate::execution::DEFAULT_GRACEFUL_TIMEOUT,
                            )
                            .await
                            .unwrap_or(false)
                            && let Some(mut status) = manager.status(&task_id).await
                        {
                            status.execution.state = Some(if status
                                .execution
                                .descendants_remaining
                                .unwrap_or(0)
                                == 0
                            {
                                "timed_out".to_string()
                            } else {
                                "kill_failed".to_string()
                            });
                            status.execution.reason = Some(reason.to_string());
                            status.error = Some(match reason {
                                "lease_expired" => {
                                    "Background task lease expired; process tree terminated"
                                        .to_string()
                                }
                                _ => "Absolute execution deadline exceeded; process tree terminated"
                                    .to_string(),
                            });
                            manager
                                .write_status_file(&manager.status_path_for(&task_id), &status)
                                .await;
                        }
                        break;
                    }
                    changed = lease_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }

    fn schedule_kill_reconciliation(&self, task_id: String) {
        let manager = Self {
            tasks: Arc::clone(&self.tasks),
            output_dir: self.output_dir.clone(),
        };
        tokio::spawn(async move {
            for delay in [1_u64, 5, 30] {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                let status_path = manager.status_path_for(&task_id);
                let Some(mut status) = manager.read_status_file(&status_path).await else {
                    return;
                };
                if status.execution.state.as_deref() != Some("kill_failed") {
                    return;
                }
                let Some(container) = Self::process_container_from_status(&status) else {
                    return;
                };
                let report = crate::execution::terminate_process_container(
                    &container,
                    crate::execution::DEFAULT_GRACEFUL_TIMEOUT,
                    crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
                )
                .await;
                if report.cleanup_verified {
                    status.execution.state = Some(
                        match status.execution.reason.as_deref() {
                            Some("user_cancelled") => "cancelled",
                            Some("absolute_deadline_exceeded") | Some("lease_expired") => {
                                "timed_out"
                            }
                            Some("backend_interrupted") => "orphan_reaped",
                            _ => "killed",
                        }
                        .to_string(),
                    );
                    status.execution.finished_at = Some(Utc::now().to_rfc3339());
                    status.execution.graceful_termination_attempted =
                        report.graceful_termination_attempted;
                    status.execution.force_kill_required = report.force_kill_required;
                    status.execution.descendants_observed = Some(report.descendants_observed);
                    status.execution.descendants_remaining = Some(0);
                    manager.write_status_file(&status_path, &status).await;
                    crate::execution::record_event(
                        "reconciliation_succeeded",
                        Some(&task_id),
                        None,
                        Some(&container),
                        status.execution.reason.as_deref(),
                        Some(&report),
                    );
                    return;
                }
            }
            crate::execution::record_event(
                "reconciliation_exhausted",
                Some(&task_id),
                None,
                None,
                Some("cleanup_verification_failed"),
                None,
            );
        });
    }

    /// Create a manager rooted at a specific output directory.
    ///
    /// Primarily for tests; production code should use [`global`].
    pub fn with_output_dir(output_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&output_dir).ok();
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            output_dir,
        }
    }

    /// Create a new background task manager
    pub fn new() -> Self {
        let output_dir = task_dir();
        Self::with_output_dir(output_dir)
    }

    /// Generate a short, unique task ID
    fn generate_task_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        const TASK_ID_ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        // Use last 6 digits of timestamp + 4 random chars
        let rand_part: String = (0..4)
            .map(|_| {
                let idx = (rand::random::<u8>() as usize) % TASK_ID_ALPHABET.len();
                TASK_ID_ALPHABET[idx] as char
            })
            .collect();
        format!(
            "{}{}",
            &timestamp.to_string()[timestamp.to_string().len().saturating_sub(6)..],
            rand_part
        )
    }

    pub fn output_path_for(&self, task_id: &str) -> PathBuf {
        self.output_dir.join(format!("{}.output", task_id))
    }

    pub fn status_path_for(&self, task_id: &str) -> PathBuf {
        self.output_dir.join(format!("{}.status.json", task_id))
    }

    fn publish_task_started_activity(
        task_id: &str,
        tool_name: &str,
        display_name: Option<&str>,
        session_id: &str,
        notify: bool,
    ) {
        if !notify {
            return;
        }
        let label = crate::message::background_task_display_label(tool_name, display_name);
        let safe_label = label.replace('`', "'");
        Bus::global().publish(BusEvent::UiActivity(crate::bus::UiActivity::background(
            Some(session_id.to_string()),
            format!(
                "**Background task started** `{}` · `{}`\n\nDaanio is running this in the background. Progress, checkpoints, and completion will appear here.",
                task_id, safe_label
            ),
            Some(format!("Background task started · {}", label)),
        )));
    }

    fn status_duration_secs(started_at: &str, completed_at: DateTime<Utc>) -> Option<f64> {
        DateTime::parse_from_rfc3339(started_at)
            .ok()
            .and_then(|started| (completed_at - started.with_timezone(&Utc)).to_std().ok())
            .map(|duration| duration.as_secs_f64())
    }

    fn parse_exit_code_from_output(output: &str) -> Option<i32> {
        output.lines().rev().find_map(|line| {
            let trimmed = line.trim();
            let suffix = trimmed.strip_prefix(EXIT_MARKER_PREFIX)?;
            let suffix = suffix.strip_suffix(" ---")?;
            suffix.trim().parse::<i32>().ok()
        })
    }

    async fn read_status_file(&self, path: &std::path::Path) -> Option<TaskStatusFile> {
        let content = fs::read_to_string(path).await.ok()?;
        serde_json::from_str(&content).ok()
    }

    async fn write_status_file(&self, path: &std::path::Path, status: &TaskStatusFile) {
        if let Ok(json) = serde_json::to_string_pretty(status) {
            let _ = fs::write(path, json).await;
        }
    }

    async fn finalize_detached_status_if_needed(
        &self,
        mut status: TaskStatusFile,
        status_path: &std::path::Path,
    ) -> TaskStatusFile {
        if status.status != BackgroundTaskStatus::Running || !status.detached {
            return status;
        }

        let Some(pid) = status.pid else {
            return status;
        };

        let reaped_exit = crate::platform::try_reap_child_process(pid).ok().flatten();

        if reaped_exit.is_none() && crate::platform::is_process_running(pid) {
            return status;
        }

        let output_path = self.output_path_for(&status.task_id);
        let output = fs::read_to_string(&output_path).await.unwrap_or_default();
        let exit_code = reaped_exit.or_else(|| Self::parse_exit_code_from_output(&output));
        let completed_at = Utc::now();
        let duration_secs = Self::status_duration_secs(&status.started_at, completed_at);
        let final_status = if matches!(exit_code, Some(0)) {
            BackgroundTaskStatus::Completed
        } else {
            BackgroundTaskStatus::Failed
        };
        let final_error = if matches!(final_status, BackgroundTaskStatus::Failed) {
            Some(match exit_code {
                Some(code) => format!("Command exited with code {}", code),
                None => "Detached command exited without a readable exit code".to_string(),
            })
        } else {
            None
        };

        status.status = final_status.clone();
        status.exit_code = exit_code;
        status.error = final_error.clone();
        status.completed_at = Some(completed_at.to_rfc3339());
        status.duration_secs = duration_secs;
        status.pid = Some(pid);
        push_task_event(
            &mut status,
            terminal_event_record(final_status.clone(), exit_code, final_error.as_deref()),
        );

        self.write_status_file(status_path, &status).await;

        let output_preview = if output.len() > 500 {
            format!("{}...", crate::util::truncate_str(&output, 500))
        } else {
            output
        };
        Bus::global().publish(BusEvent::BackgroundTaskCompleted(BackgroundTaskCompleted {
            task_id: status.task_id.clone(),
            tool_name: status.tool_name.clone(),
            display_name: status.display_name.clone(),
            session_id: status.session_id.clone(),
            status: final_status,
            exit_code,
            output_preview,
            output_file: output_path,
            duration_secs: duration_secs.unwrap_or_default(),
            notify: status.notify,
            wake: status.wake,
        }));

        status
    }

    /// True when a non-detached `Running` status file provably belongs to a
    /// process image that no longer exists, so no future can ever finalize it.
    ///
    /// Rules, deliberately conservative because the task dir is shared by
    /// every daanio process on the machine:
    /// - Terminal, detached, or pid-bearing files are never orphans here
    ///   (detached reconciliation is `finalize_detached_status_if_needed`).
    /// - Files owned by this exact process image are never orphans: the
    ///   initial status file is written before the task lands in the live
    ///   map, so "Running + not in map + my instance" can simply mean the
    ///   task is still bootstrapping.
    /// - Files owned by this PID but a different instance token are orphans:
    ///   an exec-based reload replaced the process image, so the owning
    ///   future is gone even though the PID matches.
    /// - Files owned by another PID are orphans only once that process is
    ///   dead.
    /// - Files without owner metadata (written by older builds) are left
    ///   alone; only the explicit startup sweep in
    ///   [`Self::reconcile_orphaned_tasks`] handles those.
    fn status_is_reconcilable_orphan(status: &TaskStatusFile) -> bool {
        if status.status != BackgroundTaskStatus::Running || status.detached {
            return false;
        }
        Self::owner_image_is_interrupted(status)
    }

    fn owner_image_is_interrupted(status: &TaskStatusFile) -> bool {
        let Some(owner_pid) = status.owner_pid else {
            return false;
        };
        if status.owner_instance.as_deref() == Some(model::process_instance_token()) {
            return false;
        }
        if owner_pid == std::process::id() {
            return true;
        }
        !crate::platform::is_process_running(owner_pid)
    }

    /// Finalize an orphaned non-detached `Running` status file as `Failed`.
    ///
    /// The owning process's task future died with the process (crash or
    /// exec-based server reload), so without this the file reads `Running`
    /// forever: `bg list`/`bg status` show a phantom task and `bg wait`
    /// blocks until its timeout.
    async fn finalize_orphaned_status_if_needed(
        &self,
        mut status: TaskStatusFile,
        status_path: &std::path::Path,
    ) -> TaskStatusFile {
        if !Self::status_is_reconcilable_orphan(&status) {
            return status;
        }
        // Belt and braces: never rewrite a task this process is executing.
        if self.is_live_task(&status.task_id) {
            return status;
        }

        let termination_started_at = Utc::now();
        let termination_report =
            if let Some(container) = Self::process_container_from_status(&status) {
                Some(
                    crate::execution::terminate_process_container(
                        &container,
                        crate::execution::DEFAULT_GRACEFUL_TIMEOUT,
                        crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
                    )
                    .await,
                )
            } else {
                None
            };
        let cleanup_verified = termination_report
            .as_ref()
            .map(|report| report.cleanup_verified)
            .unwrap_or(false);
        let completed_at = Utc::now();
        let duration_secs = Self::status_duration_secs(&status.started_at, completed_at);
        let error = if cleanup_verified {
            "Task orphaned because the backend reloaded or was interrupted; owned process tree was reaped"
                .to_string()
        } else if termination_report.is_some() {
            "Task orphaned because the backend reloaded or was interrupted; process-tree cleanup could not be verified".to_string()
        } else {
            "Task orphaned because the backend reloaded or was interrupted; no process identity was persisted, so safe cleanup could not be verified".to_string()
        };
        status.status = BackgroundTaskStatus::Failed;
        status.exit_code = None;
        status.error = Some(error.clone());
        status.completed_at = Some(completed_at.to_rfc3339());
        status.duration_secs = duration_secs;
        status.execution.state = Some(if cleanup_verified {
            "orphan_reaped".to_string()
        } else {
            "kill_failed".to_string()
        });
        status.execution.reason = Some("backend_interrupted".to_string());
        status.execution.termination_started_at = Some(termination_started_at.to_rfc3339());
        status.execution.finished_at = Some(completed_at.to_rfc3339());
        if let Some(report) = termination_report.as_ref() {
            status.execution.graceful_termination_attempted = report.graceful_termination_attempted;
            status.execution.force_kill_required = report.force_kill_required;
            status.execution.descendants_observed = Some(report.descendants_observed);
            status.execution.descendants_remaining = Some(report.descendants_remaining);
        }
        push_task_event(
            &mut status,
            terminal_event_record(BackgroundTaskStatus::Failed, None, Some(&error)),
        );
        self.write_status_file(status_path, &status).await;
        crate::execution::record_event(
            "startup_reconciliation",
            Some(&status.task_id),
            None,
            Self::process_container_from_status(&status).as_ref(),
            Some("backend_interrupted"),
            termination_report.as_ref(),
        );

        let output_path = self.output_path_for(&status.task_id);
        let output = fs::read_to_string(&output_path).await.unwrap_or_default();
        let output_preview = if output.len() > 500 {
            format!("{}...", crate::util::truncate_str(&output, 500))
        } else {
            output
        };
        Bus::global().publish(BusEvent::BackgroundTaskCompleted(BackgroundTaskCompleted {
            task_id: status.task_id.clone(),
            tool_name: status.tool_name.clone(),
            display_name: status.display_name.clone(),
            session_id: status.session_id.clone(),
            status: BackgroundTaskStatus::Failed,
            exit_code: None,
            output_preview,
            output_file: output_path,
            duration_secs: duration_secs.unwrap_or_default(),
            notify: status.notify,
            wake: status.wake,
        }));

        status
    }

    /// Startup/reload sweep: mark orphaned non-detached `Running` status
    /// files as `Failed` with a "server reloaded" note.
    ///
    /// Only owner-tagged files are considered, using the liveness rules of
    /// [`Self::status_is_reconcilable_orphan`]. Files without owner metadata
    /// (written by older builds, or by processes that legitimately still run
    /// them) are left untouched: the task dir is shared machine-wide, so
    /// without owner metadata there is no safe way to distinguish a phantom
    /// from another live process's task. Returns how many files were
    /// reconciled.
    pub async fn reconcile_orphaned_tasks(&self) -> usize {
        let mut reconciled = 0;
        let Ok(mut entries) = fs::read_dir(&self.output_dir).await else {
            return reconciled;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(status) = self.read_status_file(&path).await else {
                continue;
            };
            if status.status == BackgroundTaskStatus::Running
                && status.detached
                && Self::owner_image_is_interrupted(&status)
            {
                let mut recovered = status;
                let termination_started_at = Utc::now();
                let report =
                    Self::process_container_from_status(&recovered).map(|container| async move {
                        crate::execution::terminate_process_container(
                            &container,
                            crate::execution::DEFAULT_GRACEFUL_TIMEOUT,
                            crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
                        )
                        .await
                    });
                let report = match report {
                    Some(future) => Some(future.await),
                    None => None,
                };
                let cleanup_verified = report
                    .as_ref()
                    .map(|report| report.cleanup_verified)
                    .unwrap_or(false);
                let finished_at = Utc::now();
                recovered.status = BackgroundTaskStatus::Failed;
                recovered.error = Some(if cleanup_verified {
                    "Detached ordinary task was reaped during backend startup recovery".to_string()
                } else {
                    "Detached task cleanup could not be verified during backend startup recovery"
                        .to_string()
                });
                recovered.completed_at = Some(finished_at.to_rfc3339());
                recovered.duration_secs =
                    Self::status_duration_secs(&recovered.started_at, finished_at);
                recovered.execution.state = Some(if cleanup_verified {
                    "orphan_reaped".to_string()
                } else {
                    "kill_failed".to_string()
                });
                recovered.execution.reason = Some("backend_interrupted".to_string());
                recovered.execution.termination_started_at =
                    Some(termination_started_at.to_rfc3339());
                recovered.execution.finished_at = Some(finished_at.to_rfc3339());
                if let Some(report) = report {
                    recovered.execution.graceful_termination_attempted =
                        report.graceful_termination_attempted;
                    recovered.execution.force_kill_required = report.force_kill_required;
                    recovered.execution.descendants_observed = Some(report.descendants_observed);
                    recovered.execution.descendants_remaining = Some(report.descendants_remaining);
                }
                self.write_status_file(&path, &recovered).await;
                crate::execution::record_event(
                    "startup_reconciliation",
                    Some(&recovered.task_id),
                    None,
                    Self::process_container_from_status(&recovered).as_ref(),
                    Some("backend_interrupted"),
                    None,
                );
                reconciled += 1;
                continue;
            }
            if !Self::status_is_reconcilable_orphan(&status) {
                continue;
            }
            if self.tasks.read().await.contains_key(&status.task_id) {
                continue;
            }
            self.finalize_orphaned_status_if_needed(status, &path).await;
            reconciled += 1;
        }
        reconciled
    }

    pub fn reserve_task_info(&self) -> BackgroundTaskInfo {
        let task_id = Self::generate_task_id();
        let output_file = self.output_path_for(&task_id);
        let status_file = self.status_path_for(&task_id);
        BackgroundTaskInfo {
            task_id,
            output_file,
            status_file,
        }
    }

    /// Persist the OS process-container identity immediately after spawn.
    ///
    /// This record lets cancellation and startup reconciliation target only the
    /// task-owned group and reject a later process that merely reused the PID.
    pub async fn register_process_container(
        &self,
        task_id: &str,
        container: &crate::execution::ProcessContainer,
    ) -> Result<()> {
        let status_path = self.status_path_for(task_id);
        let Some(mut status) = self.read_status_file(&status_path).await else {
            anyhow::bail!("background task status disappeared before process registration");
        };
        if status.status != BackgroundTaskStatus::Running {
            anyhow::bail!("cannot register a process container for a terminal task");
        }
        status.pid = Some(container.root_pid);
        status.execution.root_pid = Some(container.root_pid);
        status.execution.process_group_id = container.process_group_id;
        status.execution.process_start_token = container.process_start_token.clone();
        status.execution.process_container = Some(
            match container.kind {
                crate::execution::ProcessContainerKind::ProcessGroup => "process_group",
                crate::execution::ProcessContainerKind::WindowsJobObject => "job_object",
                crate::execution::ProcessContainerKind::WindowsProcessTree => {
                    "windows_process_tree"
                }
            }
            .to_string(),
        );
        self.write_status_file(&status_path, &status).await;
        crate::execution::record_event(
            "spawn_succeeded",
            Some(task_id),
            None,
            Some(container),
            None,
            None,
        );
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Detached task registration mirrors persisted status fields and existing call sites"
    )]
    pub async fn register_detached_task(
        &self,
        info: &BackgroundTaskInfo,
        tool_name: &str,
        display_name: Option<String>,
        session_id: &str,
        pid: u32,
        started_at: &str,
        notify: bool,
        wake: bool,
    ) {
        let (notify, wake) = normalize_delivery(notify, wake);
        let policy = crate::execution::EffectiveExecutionPolicy::normalize(
            crate::execution::ExecutionClass::Background,
            None,
            None,
        );
        let container = crate::execution::ProcessContainer::from_pid(pid);
        let mut execution = ExecutionTaskMetadata::from_policy(&policy);
        execution.root_pid = Some(pid);
        execution.process_group_id = container.process_group_id;
        execution.process_start_token = container.process_start_token;
        let status = TaskStatusFile {
            task_id: info.task_id.clone(),
            tool_name: tool_name.to_string(),
            display_name,
            session_id: session_id.to_string(),
            status: BackgroundTaskStatus::Running,
            exit_code: None,
            error: None,
            started_at: started_at.to_string(),
            completed_at: None,
            duration_secs: None,
            pid: Some(pid),
            // The backend, not the initiating CLI connection, owns the task.
            // Persist the exact backend process image so a later backend can
            // reconcile it without touching another live instance's work.
            owner_pid: Some(std::process::id()),
            owner_instance: Some(model::process_instance_token().to_string()),
            detached: true,
            notify,
            wake,
            progress: None,
            event_history: Vec::new(),
            execution,
        };
        self.write_status_file(&info.status_file, &status).await;
        Self::publish_task_started_activity(
            &info.task_id,
            tool_name,
            status.display_name.as_deref(),
            session_id,
            notify,
        );

        // Detached does not mean unbounded. Keep the deadline in the backend
        // even when the initiating CLI disconnects.
        let watchdog = Self {
            tasks: Arc::clone(&self.tasks),
            output_dir: self.output_dir.clone(),
        };
        let watchdog_task_id = info.task_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(policy.timeout()).await;
            if watchdog
                .status(&watchdog_task_id)
                .await
                .is_some_and(|status| status.status == BackgroundTaskStatus::Running)
            {
                let _ = watchdog
                    .cancel_with_grace(
                        &watchdog_task_id,
                        crate::execution::DEFAULT_GRACEFUL_TIMEOUT,
                    )
                    .await;
            }
        });
    }

    /// Spawn a background task
    ///
    /// The `execute_fn` receives the output file path and should write output there.
    /// It returns a TaskResult with exit code and optional error.
    pub async fn spawn<F, Fut>(
        &self,
        tool_name: &str,
        session_id: &str,
        execute_fn: F,
    ) -> BackgroundTaskInfo
    where
        F: FnOnce(PathBuf) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<TaskResult>> + Send,
    {
        self.spawn_with_notify(tool_name, None, session_id, true, false, execute_fn)
            .await
    }

    /// Spawn a background task with explicit notify flag
    pub async fn spawn_with_notify<F, Fut>(
        &self,
        tool_name: &str,
        display_name: Option<String>,
        session_id: &str,
        notify: bool,
        wake: bool,
        execute_fn: F,
    ) -> BackgroundTaskInfo
    where
        F: FnOnce(PathBuf) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<TaskResult>> + Send,
    {
        let policy = crate::execution::EffectiveExecutionPolicy::normalize(
            crate::execution::ExecutionClass::Background,
            None,
            None,
        );
        self.spawn_with_notify_and_policy(
            tool_name,
            display_name,
            session_id,
            notify,
            wake,
            policy,
            execute_fn,
        )
        .await
    }

    /// Spawn a background task with a backend-normalized execution policy.
    ///
    /// The effective deadline is persisted before `execute_fn` runs so clients
    /// can disconnect without losing the backend's ownership record.
    #[expect(
        clippy::too_many_arguments,
        reason = "task identity, delivery, policy, and execution closure are independent inputs"
    )]
    pub async fn spawn_with_notify_and_policy<F, Fut>(
        &self,
        tool_name: &str,
        display_name: Option<String>,
        session_id: &str,
        notify: bool,
        wake: bool,
        policy: crate::execution::EffectiveExecutionPolicy,
        execute_fn: F,
    ) -> BackgroundTaskInfo
    where
        F: FnOnce(PathBuf) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<TaskResult>> + Send,
    {
        let (notify, wake) = normalize_delivery(notify, wake);
        let task_id = Self::generate_task_id();
        let output_path = self.output_dir.join(format!("{}.output", task_id));
        let status_path = self.output_dir.join(format!("{}.status.json", task_id));
        let started_at_rfc3339 = chrono::Utc::now().to_rfc3339();

        // Write initial status file
        let initial_status = TaskStatusFile {
            task_id: task_id.clone(),
            tool_name: tool_name.to_string(),
            display_name: display_name.clone(),
            session_id: session_id.to_string(),
            status: BackgroundTaskStatus::Running,
            exit_code: None,
            error: None,
            started_at: started_at_rfc3339.clone(),
            completed_at: None,
            duration_secs: None,
            pid: None,
            owner_pid: Some(std::process::id()),
            owner_instance: Some(model::process_instance_token().to_string()),
            detached: false,
            notify,
            wake,
            progress: None,
            event_history: Vec::new(),
            execution: ExecutionTaskMetadata::from_policy(&policy),
        };
        if let Ok(json) = serde_json::to_string_pretty(&initial_status) {
            let _ = std::fs::write(&status_path, json);
        }
        crate::execution::record_event(
            "spawn_requested",
            Some(&task_id),
            Some(&policy),
            None,
            None,
            None,
        );
        crate::execution::record_event(
            "effective_deadline_assigned",
            Some(&task_id),
            Some(&policy),
            None,
            None,
            None,
        );
        Self::publish_task_started_activity(
            &task_id,
            tool_name,
            display_name.as_deref(),
            session_id,
            notify,
        );

        let output_path_clone = output_path.clone();
        let status_path_clone = status_path.clone();
        let task_id_clone = task_id.clone();
        let tool_name_owned = tool_name.to_string();
        let display_name_owned = display_name.clone();
        let session_id_owned = session_id.to_string();
        let started_at = Instant::now();
        let started_at_rfc3339_for_task = started_at_rfc3339.clone();
        let (delivery_flags_tx, delivery_flags_rx) = watch::channel((notify, wake));
        let absolute_deadline = TokioInstant::now() + policy.timeout();
        let initial_lease_deadline = TokioInstant::now()
            + policy
                .timeout()
                .min(crate::execution::DEFAULT_BACKGROUND_LEASE);
        let (lease_deadline_tx, lease_deadline_rx) = watch::channel(initial_lease_deadline);
        let tasks_for_prune = Arc::clone(&self.tasks);
        let tasks_for_reconcile = Arc::clone(&self.tasks);
        let output_dir_for_reconcile = self.output_dir.clone();
        let (registered_tx, registered_rx) = tokio::sync::oneshot::channel::<()>();

        // Spawn the background task
        let handle = tokio::spawn(async move {
            let result = execute_fn(output_path_clone.clone()).await;

            let duration_secs = started_at.elapsed().as_secs_f64();
            let (status, exit_code, error, execution_state, termination_report) = match &result {
                Ok(task_result) => {
                    let status = task_result.status.clone().unwrap_or_else(|| {
                        if task_result.error.is_some() {
                            BackgroundTaskStatus::Failed
                        } else {
                            BackgroundTaskStatus::Completed
                        }
                    });
                    (
                        status,
                        task_result.exit_code,
                        task_result.error.clone(),
                        task_result.execution_state.clone(),
                        task_result.termination_report.clone(),
                    )
                }
                Err(e) => (
                    BackgroundTaskStatus::Failed,
                    None,
                    Some(e.to_string()),
                    Some("failed".to_string()),
                    None,
                ),
            };

            let (notify_flag, wake_flag) = *delivery_flags_rx.borrow();
            let prior_status = tokio::fs::read_to_string(&status_path_clone)
                .await
                .ok()
                .and_then(|content| serde_json::from_str::<TaskStatusFile>(&content).ok());
            let prior_progress = prior_status
                .as_ref()
                .and_then(|status| status.progress.clone());
            let prior_event_history = prior_status
                .as_ref()
                .map(|status| status.event_history.clone())
                .unwrap_or_default();
            let prior_execution = prior_status
                .as_ref()
                .map(|status| status.execution.clone())
                .unwrap_or_default();
            let mut final_execution = prior_execution;
            final_execution.state = execution_state;
            final_execution.finished_at = Some(chrono::Utc::now().to_rfc3339());
            final_execution.reason = match final_execution.state.as_deref() {
                Some("timed_out") => Some("absolute_deadline_exceeded".to_string()),
                Some("kill_failed") => Some("cleanup_verification_failed".to_string()),
                _ => final_execution.reason,
            };
            if let Some(report) = termination_report {
                final_execution.graceful_termination_attempted =
                    report.graceful_termination_attempted;
                final_execution.force_kill_required = report.force_kill_required;
                final_execution.descendants_observed = Some(report.descendants_observed);
                final_execution.descendants_remaining = Some(report.descendants_remaining);
            }

            // Update status file
            let mut final_status = TaskStatusFile {
                task_id: task_id_clone.clone(),
                tool_name: tool_name_owned.clone(),
                display_name: display_name_owned.clone(),
                session_id: session_id_owned.clone(),
                status: status.clone(),
                exit_code,
                error: error.clone(),
                started_at: started_at_rfc3339_for_task,
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_secs: Some(duration_secs),
                pid: None,
                owner_pid: Some(std::process::id()),
                owner_instance: Some(model::process_instance_token().to_string()),
                detached: false,
                notify: notify_flag,
                wake: wake_flag,
                progress: prior_progress,
                event_history: prior_event_history,
                execution: final_execution,
            };
            push_task_event(
                &mut final_status,
                terminal_event_record(status.clone(), exit_code, error.as_deref()),
            );
            if let Ok(json) = serde_json::to_string_pretty(&final_status) {
                let _ = tokio::fs::write(&status_path_clone, json).await;
            }
            if final_status.execution.state.as_deref() == Some("kill_failed") {
                Self {
                    tasks: tasks_for_reconcile,
                    output_dir: output_dir_for_reconcile,
                }
                .schedule_kill_reconciliation(task_id_clone.clone());
            }

            // Drop this task from the live map now that its terminal status is
            // persisted. Order matters: pruning only after the status-file
            // write keeps "in the live map while the status file says Running"
            // equivalent to "a task future is actually executing", which the
            // run_plan duplicate-driver guard and self-dev build reconciliation
            // rely on. Awaiting registration first means a task that finishes
            // instantly cannot race the insert below and leave a permanent
            // phantom entry in the map.
            let _ = registered_rx.await;
            tasks_for_prune.write().await.remove(&task_id_clone);

            // Read output preview for notification
            let output_preview = tokio::fs::read_to_string(&output_path_clone)
                .await
                .map(|s| {
                    if s.len() > 500 {
                        format!("{}...", crate::util::truncate_str(&s, 500))
                    } else {
                        s
                    }
                })
                .unwrap_or_default();

            // Publish completion event to the bus
            Bus::global().publish(BusEvent::BackgroundTaskCompleted(BackgroundTaskCompleted {
                task_id: task_id_clone,
                tool_name: tool_name_owned,
                display_name: display_name_owned,
                session_id: session_id_owned,
                status,
                exit_code,
                output_preview,
                output_file: output_path_clone,
                duration_secs,
                notify: notify_flag,
                wake: wake_flag,
            }));

            result
        });

        // Track the running task
        let running_task = RunningTask {
            task_id: task_id.clone(),
            tool_name: tool_name.to_string(),
            display_name,
            session_id: session_id.to_string(),
            status_path: status_path.clone(),
            started_at,
            started_at_rfc3339,
            delivery_flags: delivery_flags_tx,
            lease_deadline: lease_deadline_tx,
            absolute_deadline,
            handle,
        };

        self.tasks
            .write()
            .await
            .insert(task_id.clone(), running_task);
        let _ = registered_tx.send(());
        self.launch_deadline_watchdog(task_id.clone(), lease_deadline_rx, absolute_deadline);

        BackgroundTaskInfo {
            task_id,
            output_file: output_path,
            status_file: status_path,
        }
    }

    /// Adopt an already-spawned task as a background task.
    /// Used when the user moves a currently-executing tool to background via Alt+B.
    /// The `handle` is an already-running tokio task; we just register it for tracking
    /// and wire up completion notifications.
    pub async fn adopt(
        &self,
        tool_name: &str,
        session_id: &str,
        handle: JoinHandle<Result<daanio_tool_types::ToolOutput>>,
    ) -> BackgroundTaskInfo {
        self.adopt_with_options(tool_name, None, session_id, true, false, handle)
            .await
    }

    /// Adopt an already-spawned task as a background task, with explicit display
    /// name and delivery flags. Used both for user-initiated handoff (Alt+B) and
    /// for promoting a foreground command that exceeded its timeout but is still
    /// running, so it keeps running and surfaces as a background-task card.
    pub async fn adopt_with_options(
        &self,
        tool_name: &str,
        display_name: Option<String>,
        session_id: &str,
        notify: bool,
        wake: bool,
        handle: JoinHandle<Result<daanio_tool_types::ToolOutput>>,
    ) -> BackgroundTaskInfo {
        let (notify, wake) = normalize_delivery(notify, wake);
        let policy = crate::execution::EffectiveExecutionPolicy::normalize(
            crate::execution::ExecutionClass::Background,
            None,
            None,
        );
        let task_id = Self::generate_task_id();
        let output_path = self.output_dir.join(format!("{}.output", task_id));
        let status_path = self.output_dir.join(format!("{}.status.json", task_id));

        let initial_status = TaskStatusFile {
            task_id: task_id.clone(),
            tool_name: tool_name.to_string(),
            display_name: display_name.clone(),
            session_id: session_id.to_string(),
            status: BackgroundTaskStatus::Running,
            exit_code: None,
            error: None,
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: None,
            duration_secs: None,
            pid: None,
            owner_pid: Some(std::process::id()),
            owner_instance: Some(model::process_instance_token().to_string()),
            detached: false,
            notify,
            wake,
            progress: None,
            event_history: Vec::new(),
            execution: ExecutionTaskMetadata::from_policy(&policy),
        };
        if let Ok(json) = serde_json::to_string_pretty(&initial_status) {
            let _ = std::fs::write(&status_path, json);
        }
        Self::publish_task_started_activity(
            &task_id,
            tool_name,
            display_name.as_deref(),
            session_id,
            notify,
        );

        let output_path_clone = output_path.clone();
        let status_path_clone = status_path.clone();
        let task_id_clone = task_id.clone();
        let tool_name_owned = tool_name.to_string();
        let session_id_owned = session_id.to_string();
        let started_at = Instant::now();
        let started_at_rfc3339 = initial_status.started_at.clone();
        let display_name_owned = initial_status.display_name.clone();
        let (delivery_flags_tx, delivery_flags_rx) = watch::channel((notify, wake));
        let absolute_deadline = TokioInstant::now() + policy.timeout();
        let initial_lease_deadline = TokioInstant::now()
            + policy
                .timeout()
                .min(crate::execution::DEFAULT_BACKGROUND_LEASE);
        let (lease_deadline_tx, lease_deadline_rx) = watch::channel(initial_lease_deadline);
        let tasks_for_prune = Arc::clone(&self.tasks);
        let (registered_tx, registered_rx) = tokio::sync::oneshot::channel::<()>();

        let wrapper_handle = tokio::spawn(async move {
            let tool_result = handle.await;
            let duration_secs = started_at.elapsed().as_secs_f64();

            let (status, exit_code, error, output_text) = match tool_result {
                Ok(Ok(output)) => (
                    BackgroundTaskStatus::Completed,
                    Some(0),
                    None,
                    output.output,
                ),
                Ok(Err(e)) => (
                    BackgroundTaskStatus::Failed,
                    None,
                    Some(e.to_string()),
                    e.to_string(),
                ),
                Err(e) => (
                    BackgroundTaskStatus::Failed,
                    None,
                    Some(e.to_string()),
                    format!("Task panicked: {}", e),
                ),
            };

            if let Ok(mut file) = File::create(&output_path_clone).await {
                let _ = file.write_all(output_text.as_bytes()).await;
            }

            let (notify_flag, wake_flag) = *delivery_flags_rx.borrow();
            let prior_status = tokio::fs::read_to_string(&status_path_clone)
                .await
                .ok()
                .and_then(|content| serde_json::from_str::<TaskStatusFile>(&content).ok());
            let prior_progress = prior_status
                .as_ref()
                .and_then(|status| status.progress.clone());
            let prior_event_history = prior_status
                .as_ref()
                .map(|status| status.event_history.clone())
                .unwrap_or_default();
            let prior_execution = prior_status
                .as_ref()
                .map(|status| status.execution.clone())
                .unwrap_or_default();

            let mut final_status = TaskStatusFile {
                task_id: task_id_clone.clone(),
                tool_name: tool_name_owned.clone(),
                display_name: display_name_owned.clone(),
                session_id: session_id_owned.clone(),
                status: status.clone(),
                exit_code,
                error: error.clone(),
                started_at: started_at_rfc3339,
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_secs: Some(duration_secs),
                pid: None,
                owner_pid: Some(std::process::id()),
                owner_instance: Some(model::process_instance_token().to_string()),
                detached: false,
                notify: notify_flag,
                wake: wake_flag,
                progress: prior_progress,
                event_history: prior_event_history,
                execution: prior_execution,
            };
            push_task_event(
                &mut final_status,
                terminal_event_record(status.clone(), exit_code, error.as_deref()),
            );
            if let Ok(json) = serde_json::to_string_pretty(&final_status) {
                let _ = tokio::fs::write(&status_path_clone, json).await;
            }

            // Prune the live-map entry only after the terminal status file is
            // persisted (and after registration below, so instant completions
            // cannot race the insert and leave a phantom entry).
            let _ = registered_rx.await;
            tasks_for_prune.write().await.remove(&task_id_clone);

            let output_preview = if output_text.len() > 500 {
                format!("{}...", crate::util::truncate_str(&output_text, 500))
            } else {
                output_text
            };

            Bus::global().publish(BusEvent::BackgroundTaskCompleted(BackgroundTaskCompleted {
                task_id: task_id_clone,
                tool_name: tool_name_owned,
                display_name: display_name_owned,
                session_id: session_id_owned,
                status: status.clone(),
                exit_code,
                output_preview,
                output_file: output_path_clone,
                duration_secs,
                notify: notify_flag,
                wake: wake_flag,
            }));

            Ok(TaskResult {
                exit_code,
                error,
                status: Some(status),
                execution_state: Some(if exit_code == Some(0) {
                    "completed".to_string()
                } else {
                    "failed".to_string()
                }),
                termination_report: None,
            })
        });

        let running_task = RunningTask {
            task_id: task_id.clone(),
            tool_name: tool_name.to_string(),
            display_name: None,
            session_id: session_id.to_string(),
            status_path: status_path.clone(),
            started_at,
            started_at_rfc3339: initial_status.started_at.clone(),
            delivery_flags: delivery_flags_tx,
            lease_deadline: lease_deadline_tx,
            absolute_deadline,
            handle: wrapper_handle,
        };

        self.tasks
            .write()
            .await
            .insert(task_id.clone(), running_task);
        let _ = registered_tx.send(());
        self.launch_deadline_watchdog(task_id.clone(), lease_deadline_rx, absolute_deadline);

        BackgroundTaskInfo {
            task_id,
            output_file: output_path,
            status_file: status_path,
        }
    }

    /// List all tasks (both running and completed from disk)
    pub async fn list(&self) -> Vec<TaskStatusFile> {
        let mut results = Vec::new();

        // Read all status files from disk
        if let Ok(mut entries) = fs::read_dir(&self.output_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false)
                    && let Some(status) = self.read_status_file(&path).await
                {
                    let reconciled = self.finalize_detached_status_if_needed(status, &path).await;
                    let reconciled = self
                        .finalize_orphaned_status_if_needed(reconciled, &path)
                        .await;
                    results.push(reconciled);
                }
            }
        }

        // Sort by task_id (which includes timestamp)
        results.sort_by(|a, b| b.task_id.cmp(&a.task_id));
        results
    }

    /// Get status of a specific task
    pub async fn status(&self, task_id: &str) -> Option<TaskStatusFile> {
        let status_path = self.status_path_for(task_id);
        let status = self.read_status_file(&status_path).await?;
        let status = self
            .finalize_detached_status_if_needed(status, &status_path)
            .await;
        Some(
            self.finalize_orphaned_status_if_needed(status, &status_path)
                .await,
        )
    }

    /// Best-effort synchronous check for whether a task is still live in this process.
    pub fn is_live_task(&self, task_id: &str) -> bool {
        let Ok(tasks) = self.tasks.try_read() else {
            return false;
        };
        tasks.contains_key(task_id)
    }

    /// Get full output of a task
    pub async fn output(&self, task_id: &str) -> Option<String> {
        let output_path = self.output_path_for(task_id);
        fs::read_to_string(&output_path).await.ok()
    }

    /// Wait for a task to finish, emit progress, or reach the caller's maximum wait.
    ///
    /// This combines bus-driven wakeups with a light periodic status reconciliation so
    /// detached tasks, missed broadcast messages, or crash/reload edges still return no
    /// later than `max_wait` and can notice completion without active polling by the agent.
    pub async fn wait(
        &self,
        task_id: &str,
        max_wait: Duration,
        return_on_progress: bool,
    ) -> Option<BackgroundTaskWaitResult> {
        let mut bus_rx = Bus::global().subscribe();
        let initial = self.status(task_id).await?;
        if initial.status != BackgroundTaskStatus::Running {
            return Some(BackgroundTaskWaitResult {
                reason: BackgroundTaskWaitReason::AlreadyFinished,
                task: initial,
                progress_event: None,
                event_record: None,
            });
        }
        if max_wait.is_zero() {
            return Some(BackgroundTaskWaitResult {
                reason: BackgroundTaskWaitReason::Timeout,
                task: initial,
                progress_event: None,
                event_record: None,
            });
        }

        let mut last_progress = initial.progress.clone();
        let deadline = TokioInstant::now() + max_wait;
        let timeout = tokio::time::sleep_until(deadline);
        tokio::pin!(timeout);
        let mut poll = tokio::time::interval(Duration::from_secs(1));
        poll.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    let task = self.status(task_id).await?;
                    let reason = if task.status == BackgroundTaskStatus::Running {
                        BackgroundTaskWaitReason::Timeout
                    } else {
                        BackgroundTaskWaitReason::Finished
                    };
                    return Some(BackgroundTaskWaitResult {
                        reason,
                        task,
                        progress_event: None,
                        event_record: None,
                    });
                }
                _ = poll.tick() => {
                    let task = self.status(task_id).await?;
                    if task.status != BackgroundTaskStatus::Running {
                        return Some(BackgroundTaskWaitResult {
                            reason: BackgroundTaskWaitReason::Finished,
                            task,
                            progress_event: None,
                            event_record: None,
                        });
                    }
                    if return_on_progress && task.progress != last_progress {
                        let event_record = task.event_history.last().cloned();
                        return Some(BackgroundTaskWaitResult {
                            reason: progress_wait_reason(event_record.as_ref()),
                            progress_event: None,
                            task,
                            event_record,
                        });
                    }
                    last_progress = task.progress.clone();
                }
                event = bus_rx.recv() => {
                    match event {
                        Ok(BusEvent::BackgroundTaskCompleted(event)) if event.task_id == task_id => {
                            let task = self.status(task_id).await?;
                            return Some(BackgroundTaskWaitResult {
                                reason: BackgroundTaskWaitReason::Finished,
                                task,
                                progress_event: None,
                                event_record: None,
                            });
                        }
                        Ok(BusEvent::BackgroundTaskProgress(event)) if event.task_id == task_id => {
                            if return_on_progress {
                                let task = self.status(task_id).await?;
                                let event_record = task.event_history.last().cloned();
                                return Some(BackgroundTaskWaitResult {
                                    reason: progress_wait_reason(event_record.as_ref()),
                                    task,
                                    progress_event: Some(event),
                                    event_record,
                                });
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            let task = self.status(task_id).await?;
                            if task.status != BackgroundTaskStatus::Running {
                                return Some(BackgroundTaskWaitResult {
                                    reason: BackgroundTaskWaitReason::Finished,
                                    task,
                                    progress_event: None,
                                    event_record: None,
                                });
                            }
                            if return_on_progress && task.progress != last_progress {
                                let event_record = task.event_history.last().cloned();
                                return Some(BackgroundTaskWaitResult {
                                    reason: progress_wait_reason(event_record.as_ref()),
                                    progress_event: None,
                                    task,
                                    event_record,
                                });
                            }
                            last_progress = task.progress.clone();
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            let task = self.status(task_id).await?;
                            let reason = if task.status == BackgroundTaskStatus::Running {
                                BackgroundTaskWaitReason::Timeout
                            } else {
                                BackgroundTaskWaitReason::Finished
                            };
                            return Some(BackgroundTaskWaitResult {
                                reason,
                                task,
                                progress_event: None,
                                event_record: None,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    /// Update progress for an existing background task.
    pub async fn update_progress(
        &self,
        task_id: &str,
        progress: BackgroundTaskProgress,
    ) -> Result<Option<TaskStatusFile>> {
        self.update_progress_with_event_kind(task_id, progress, BackgroundTaskEventKind::Progress)
            .await
    }

    /// Record an explicit checkpoint for an existing background task.
    pub async fn update_checkpoint(
        &self,
        task_id: &str,
        progress: BackgroundTaskProgress,
    ) -> Result<Option<TaskStatusFile>> {
        self.update_progress_with_event_kind(task_id, progress, BackgroundTaskEventKind::Checkpoint)
            .await
    }

    async fn update_progress_with_event_kind(
        &self,
        task_id: &str,
        progress: BackgroundTaskProgress,
        event_kind: BackgroundTaskEventKind,
    ) -> Result<Option<TaskStatusFile>> {
        let status_path = self.status_path_for(task_id);
        let Some(mut status) = self.read_status_file(&status_path).await else {
            return Ok(None);
        };

        let progress = progress.normalize();
        if let Some(existing) = status.progress.as_ref() {
            if progress_equivalent(existing, &progress) {
                return Ok(Some(status));
            }

            let existing_is_more_determinate = existing.percent.is_some()
                || matches!((existing.current, existing.total), (_, Some(total)) if total > 0);
            let new_is_less_determinate = progress.percent.is_none()
                && !matches!((progress.current, progress.total), (_, Some(total)) if total > 0);
            if existing_is_more_determinate
                && new_is_less_determinate
                && matches!(progress.source, BackgroundTaskProgressSource::ParsedOutput)
            {
                return Ok(Some(status));
            }
        }

        status.progress = Some(progress.clone());
        let progress_at = Utc::now();
        status.execution.last_progress_at = Some(progress_at.to_rfc3339());
        if let Some(deadline) = status
            .execution
            .deadline_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc))
        {
            let proposed = progress_at
                + chrono::Duration::from_std(crate::execution::DEFAULT_BACKGROUND_LEASE)
                    .unwrap_or_else(|_| chrono::Duration::hours(1));
            status.execution.lease_expires_at = Some(proposed.min(deadline).to_rfc3339());
        }
        push_task_event(
            &mut status,
            progress_event_record(event_kind, progress.clone()),
        );
        self.write_status_file(&status_path, &status).await;
        crate::execution::record_event(
            "progress_received",
            Some(task_id),
            None,
            Self::process_container_from_status(&status).as_ref(),
            None,
            None,
        );

        Bus::global().publish(BusEvent::BackgroundTaskProgress(
            BackgroundTaskProgressEvent {
                task_id: status.task_id.clone(),
                tool_name: status.tool_name.clone(),
                display_name: status.display_name.clone(),
                session_id: status.session_id.clone(),
                progress,
            },
        ));

        if let Some(task) = self.tasks.read().await.get(task_id) {
            let renewed = (TokioInstant::now() + crate::execution::DEFAULT_BACKGROUND_LEASE)
                .min(task.absolute_deadline);
            let _ = task.lease_deadline.send(renewed);
        }

        Ok(Some(status))
    }

    /// Update delivery behavior for an existing background task.
    ///
    /// This supports retroactively enabling notify/wake after the task was already started.
    pub async fn update_delivery(
        &self,
        task_id: &str,
        notify: bool,
        wake: bool,
    ) -> Result<Option<TaskStatusFile>> {
        let (notify, wake) = normalize_delivery(notify, wake);
        let status_path = self.status_path_for(task_id);
        let Some(mut status) = self.read_status_file(&status_path).await else {
            return Ok(None);
        };
        status.notify = notify;
        status.wake = wake;
        let event_status = status.status.clone();
        let event_exit_code = status.exit_code;
        let event_progress = status.progress.clone();
        push_task_event(
            &mut status,
            BackgroundTaskEventRecord {
                kind: BackgroundTaskEventKind::DeliveryUpdated,
                timestamp: Utc::now().to_rfc3339(),
                message: Some(format!("notify={}, wake={}", notify, wake)),
                status: Some(event_status),
                exit_code: event_exit_code,
                progress: event_progress,
            },
        );
        self.write_status_file(&status_path, &status).await;

        if let Some(task) = self.tasks.read().await.get(task_id) {
            let _ = task.delivery_flags.send((notify, wake));
        }

        Ok(Some(status))
    }

    /// Cancel a running task
    pub async fn cancel(&self, task_id: &str) -> Result<bool> {
        self.cancel_with_grace(task_id, crate::execution::DEFAULT_GRACEFUL_TIMEOUT)
            .await
    }

    /// Cancel a running task, allowing detached processes a configurable grace period
    /// between TERM and KILL on Unix.
    pub async fn cancel_with_grace(
        &self,
        task_id: &str,
        _graceful_timeout: std::time::Duration,
    ) -> Result<bool> {
        crate::execution::record_event(
            "cancellation_requested",
            Some(task_id),
            None,
            None,
            Some("user_cancelled"),
            None,
        );
        let mut tasks = self.tasks.write().await;
        if let Some(mut task) = tasks.remove(task_id) {
            drop(tasks);
            let prior_status = self.read_status_file(&task.status_path).await;
            let process_container = prior_status
                .as_ref()
                .and_then(Self::process_container_from_status);
            let termination_started_at = Utc::now();

            let termination_report = if let Some(container) = process_container.as_ref() {
                let report = crate::execution::terminate_process_container(
                    container,
                    _graceful_timeout,
                    crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
                )
                .await;
                // Let the owning future observe process exit and reap the root.
                if tokio::time::timeout(
                    crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
                    &mut task.handle,
                )
                .await
                .is_err()
                {
                    task.handle.abort();
                    let _ = tokio::time::timeout(Duration::from_secs(1), &mut task.handle).await;
                }
                Some(report)
            } else {
                // Pure in-process background work has no OS process container.
                task.handle.abort();
                let _ = tokio::time::timeout(Duration::from_secs(2), &mut task.handle).await;
                None
            };

            // Update status file
            let (notify_flag, wake_flag) = *task.delivery_flags.borrow();
            let cleanup_verified = termination_report
                .as_ref()
                .map(|report| report.cleanup_verified)
                .unwrap_or(true);
            let final_error = if cleanup_verified {
                "Cancelled by user".to_string()
            } else {
                "Cancellation requested, but process-tree cleanup verification failed".to_string()
            };
            let mut execution = prior_status
                .as_ref()
                .map(|status| status.execution.clone())
                .unwrap_or_default();
            execution.state = Some(if cleanup_verified {
                "cancelled".to_string()
            } else {
                "kill_failed".to_string()
            });
            execution.reason = Some("user_cancelled".to_string());
            execution.termination_started_at = Some(termination_started_at.to_rfc3339());
            execution.finished_at = Some(Utc::now().to_rfc3339());
            if let Some(report) = termination_report {
                execution.graceful_termination_attempted = report.graceful_termination_attempted;
                execution.force_kill_required = report.force_kill_required;
                execution.descendants_observed = Some(report.descendants_observed);
                execution.descendants_remaining = Some(report.descendants_remaining);
            }
            let mut final_status = TaskStatusFile {
                task_id: task.task_id,
                tool_name: task.tool_name,
                display_name: task.display_name,
                session_id: task.session_id,
                status: BackgroundTaskStatus::Failed,
                exit_code: None,
                error: Some(final_error),
                started_at: task.started_at_rfc3339,
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_secs: Some(task.started_at.elapsed().as_secs_f64()),
                pid: None,
                owner_pid: Some(std::process::id()),
                owner_instance: Some(model::process_instance_token().to_string()),
                detached: false,
                notify: notify_flag,
                wake: wake_flag,
                progress: prior_status
                    .as_ref()
                    .and_then(|status| status.progress.clone()),
                event_history: prior_status
                    .as_ref()
                    .map(|status| status.event_history.clone())
                    .unwrap_or_default(),
                execution,
            };
            let event_status = final_status.status.clone();
            let event_exit_code = final_status.exit_code;
            let event_error = final_status.error.clone();
            push_task_event(
                &mut final_status,
                terminal_event_record(event_status, event_exit_code, event_error.as_deref()),
            );
            if let Ok(json) = serde_json::to_string_pretty(&final_status) {
                let _ = fs::write(&task.status_path, json).await;
            }
            if !cleanup_verified {
                self.schedule_kill_reconciliation(task_id.to_string());
            }

            Ok(true)
        } else {
            drop(tasks);

            let status_path = self.status_path_for(task_id);
            let Some(mut status) = self.read_status_file(&status_path).await else {
                return Ok(false);
            };
            status = self
                .finalize_detached_status_if_needed(status, &status_path)
                .await;
            if status.status != BackgroundTaskStatus::Running || !status.detached {
                return Ok(false);
            }

            let Some(pid) = status.pid else {
                return Ok(false);
            };
            let Some(container) = Self::process_container_from_status(&status) else {
                return Ok(false);
            };
            let termination_started_at = Utc::now();
            let report = crate::execution::terminate_process_container(
                &container,
                _graceful_timeout,
                crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
            )
            .await;
            let _ = crate::platform::try_reap_child_process(pid);

            let completed_at = Utc::now();
            status.status = BackgroundTaskStatus::Failed;
            status.exit_code = None;
            status.error = Some(if report.cleanup_verified {
                "Cancelled by user".to_string()
            } else {
                "Cancellation requested, but process-tree cleanup verification failed".to_string()
            });
            status.completed_at = Some(completed_at.to_rfc3339());
            status.duration_secs = Self::status_duration_secs(&status.started_at, completed_at);
            status.execution.state = Some(if report.cleanup_verified {
                "cancelled".to_string()
            } else {
                "kill_failed".to_string()
            });
            status.execution.reason = Some("user_cancelled".to_string());
            status.execution.termination_started_at = Some(termination_started_at.to_rfc3339());
            status.execution.finished_at = Some(completed_at.to_rfc3339());
            status.execution.graceful_termination_attempted = report.graceful_termination_attempted;
            status.execution.force_kill_required = report.force_kill_required;
            status.execution.descendants_observed = Some(report.descendants_observed);
            status.execution.descendants_remaining = Some(report.descendants_remaining);
            let event_status = status.status.clone();
            let event_exit_code = status.exit_code;
            let event_error = status.error.clone();
            push_task_event(
                &mut status,
                terminal_event_record(event_status, event_exit_code, event_error.as_deref()),
            );
            self.write_status_file(&status_path, &status).await;
            if !report.cleanup_verified {
                self.schedule_kill_reconciliation(task_id.to_string());
            }
            Ok(true)
        }
    }

    /// Abort every live in-process task before an exec-based server reload.
    ///
    /// `exec` replaces the process image without running destructors, so
    /// without this the spawned task futures simply vanish: their
    /// `kill_on_drop` children (e.g. cargo builds) are never killed and keep
    /// running orphaned, and their status files stay `Running` until the next
    /// process's reconcile sweep happens to notice. Aborting the handles here
    /// drops the futures (killing children) and persisting a terminal
    /// `Failed` status makes the interruption deterministic and immediately
    /// visible to `bg wait`/`bg status` and self-dev queue reconciliation.
    ///
    /// Returns the number of tasks finalized.
    pub async fn abort_live_tasks_for_reload(&self) -> usize {
        let tasks: Vec<RunningTask> = {
            let mut map = self.tasks.write().await;
            map.drain().map(|(_, task)| task).collect()
        };
        let mut finalized = 0;

        for mut task in tasks {
            let (notify_flag, wake_flag) = *task.delivery_flags.borrow();
            let prior_status = self.read_status_file(&task.status_path).await;
            // If the task won the race and finished naturally, keep its real
            // terminal status instead of stamping it as interrupted.
            if prior_status
                .as_ref()
                .is_some_and(|status| status.status != BackgroundTaskStatus::Running)
            {
                continue;
            }
            let termination_report = if let Some(container) = prior_status
                .as_ref()
                .and_then(Self::process_container_from_status)
            {
                let report = crate::execution::terminate_process_container(
                    &container,
                    crate::execution::DEFAULT_GRACEFUL_TIMEOUT,
                    crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
                )
                .await;
                if tokio::time::timeout(
                    crate::execution::DEFAULT_FORCE_VERIFY_TIMEOUT,
                    &mut task.handle,
                )
                .await
                .is_err()
                {
                    task.handle.abort();
                    let _ = tokio::time::timeout(Duration::from_secs(1), &mut task.handle).await;
                }
                Some(report)
            } else {
                task.handle.abort();
                let _ = tokio::time::timeout(Duration::from_secs(2), &mut task.handle).await;
                None
            };
            let error = "Interrupted by server reload: the owning server process was replaced before the task finished".to_string();
            let mut execution = prior_status
                .as_ref()
                .map(|status| status.execution.clone())
                .unwrap_or_default();
            let cleanup_verified = termination_report
                .as_ref()
                .map(|report| report.cleanup_verified)
                .unwrap_or(true);
            execution.state = Some(if cleanup_verified {
                "backend_interrupted".to_string()
            } else {
                "kill_failed".to_string()
            });
            execution.reason = Some("backend_shutdown".to_string());
            execution.finished_at = Some(Utc::now().to_rfc3339());
            if let Some(report) = termination_report {
                execution.graceful_termination_attempted = report.graceful_termination_attempted;
                execution.force_kill_required = report.force_kill_required;
                execution.descendants_observed = Some(report.descendants_observed);
                execution.descendants_remaining = Some(report.descendants_remaining);
            }
            let mut final_status = TaskStatusFile {
                task_id: task.task_id,
                tool_name: task.tool_name,
                display_name: prior_status
                    .as_ref()
                    .and_then(|status| status.display_name.clone())
                    .or(task.display_name),
                session_id: task.session_id,
                status: BackgroundTaskStatus::Failed,
                exit_code: None,
                error: Some(error.clone()),
                started_at: task.started_at_rfc3339,
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                duration_secs: Some(task.started_at.elapsed().as_secs_f64()),
                pid: None,
                owner_pid: Some(std::process::id()),
                owner_instance: Some(model::process_instance_token().to_string()),
                detached: false,
                notify: notify_flag,
                wake: wake_flag,
                progress: prior_status
                    .as_ref()
                    .and_then(|status| status.progress.clone()),
                event_history: prior_status
                    .as_ref()
                    .map(|status| status.event_history.clone())
                    .unwrap_or_default(),
                execution,
            };
            push_task_event(
                &mut final_status,
                terminal_event_record(BackgroundTaskStatus::Failed, None, Some(&error)),
            );
            self.write_status_file(&task.status_path, &final_status)
                .await;
            finalized += 1;
        }

        finalized
    }

    /// Clean up old task files (older than specified hours)
    pub async fn cleanup(&self, max_age_hours: u64) -> Result<usize> {
        Ok(self
            .cleanup_filtered(max_age_hours, &std::collections::HashSet::new(), false)
            .await?
            .removed_files)
    }

    /// Clean up old task files, skipping running tasks and optionally filtering by status.
    pub async fn cleanup_filtered(
        &self,
        max_age_hours: u64,
        status_filter: &std::collections::HashSet<&str>,
        dry_run: bool,
    ) -> Result<BackgroundCleanupResult> {
        let mut result = BackgroundCleanupResult {
            matched_files: 0,
            removed_files: 0,
            skipped_running_files: 0,
        };
        let cutoff =
            std::time::SystemTime::now() - std::time::Duration::from_secs(max_age_hours * 3600);

        if let Ok(mut entries) = fs::read_dir(&self.output_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let Ok(metadata) = fs::metadata(&path).await else {
                    continue;
                };
                let Ok(modified) = metadata.modified() else {
                    continue;
                };
                if modified >= cutoff {
                    continue;
                }

                let mut associated_status = None;
                if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                    associated_status = self.read_status_file(&path).await;
                } else if path.extension().and_then(|ext| ext.to_str()) == Some("output")
                    && let Some(task_id) = path.file_stem().and_then(|stem| stem.to_str())
                {
                    associated_status = self.status(task_id).await;
                }

                if let Some(status) = associated_status.as_ref() {
                    if status.status == BackgroundTaskStatus::Running {
                        result.skipped_running_files += 1;
                        continue;
                    }
                    let status_label = match status.status {
                        BackgroundTaskStatus::Running => "running",
                        BackgroundTaskStatus::Completed => "completed",
                        BackgroundTaskStatus::Superseded => "superseded",
                        BackgroundTaskStatus::Failed => "failed",
                    };
                    if !status_filter.is_empty() && !status_filter.contains(status_label) {
                        continue;
                    }
                } else if !status_filter.is_empty() {
                    continue;
                }

                result.matched_files += 1;
                if !dry_run {
                    let _ = fs::remove_file(&path).await;
                    result.removed_files += 1;
                }
            }
        }

        if dry_run {
            result.removed_files = result.matched_files;
        }

        Ok(result)
    }

    /// Best-effort synchronous snapshot of currently running tasks.
    /// This avoids async calls in render paths.
    pub fn running_snapshot(&self) -> (usize, Vec<String>, Option<RunningBackgroundProgress>) {
        let Ok(tasks) = self.tasks.try_read() else {
            return (0, Vec::new(), None);
        };

        let mut rows: Vec<RunningBackgroundProgress> = Vec::new();
        for task in tasks.values() {
            let status = std::fs::read_to_string(&task.status_path)
                .ok()
                .and_then(|content| serde_json::from_str::<TaskStatusFile>(&content).ok());
            let progress = status.as_ref().and_then(|status| status.progress.clone());
            let label = status
                .as_ref()
                .and_then(|status| status.display_name.clone())
                .or_else(|| task.display_name.clone())
                .unwrap_or_else(|| task.tool_name.clone());

            rows.push(RunningBackgroundProgress {
                task_id: task.task_id.clone(),
                tool_name: task.tool_name.clone(),
                label,
                detail: progress.map(|progress| format_progress_display(&progress, 10)),
            });
        }

        rows.sort_by(|a, b| b.task_id.cmp(&a.task_id));
        let latest = rows.iter().find(|row| row.detail.is_some()).cloned();

        (
            tasks.len(),
            rows.iter().map(|row| row.label.clone()).collect(),
            latest,
        )
    }

    /// Best-effort synchronous lookup of detached tasks that are still running
    /// for a specific session.
    ///
    /// This is primarily used during self-dev reload recovery, where the new
    /// process needs to remind the agent that a previous `bash` command was
    /// persisted into the background instead of being interrupted.
    pub fn persisted_detached_running_tasks_for_session(
        &self,
        session_id: &str,
    ) -> Vec<TaskStatusFile> {
        let mut matches = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.output_dir) else {
            return matches;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }

            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(status) = serde_json::from_str::<TaskStatusFile>(&content) else {
                continue;
            };

            if status.session_id != session_id
                || status.status != BackgroundTaskStatus::Running
                || !status.detached
            {
                continue;
            }

            let Some(pid) = status.pid else {
                continue;
            };

            if crate::platform::is_process_running(pid) {
                matches.push(status);
            }
        }

        matches.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        matches
    }
}

impl Default for BackgroundTaskManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Global singleton for background task manager
static BACKGROUND_MANAGER: std::sync::OnceLock<BackgroundTaskManager> = std::sync::OnceLock::new();

/// Get the global background task manager
pub fn global() -> &'static BackgroundTaskManager {
    BACKGROUND_MANAGER.get_or_init(BackgroundTaskManager::new)
}

#[cfg(test)]
mod tests;
