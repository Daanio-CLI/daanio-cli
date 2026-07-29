use crate::bus::{BackgroundTaskProgress, BackgroundTaskProgressEvent, BackgroundTaskStatus};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant as TokioInstant;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecutionTaskMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graceful_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_group_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_start_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub termination_started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub graceful_termination_attempted: bool,
    #[serde(default)]
    pub force_kill_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descendants_observed: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descendants_remaining: Option<usize>,
    #[serde(default)]
    pub output_truncated: bool,
}

impl ExecutionTaskMetadata {
    pub fn from_policy(policy: &crate::execution::EffectiveExecutionPolicy) -> Self {
        let started = Utc::now();
        let lease = policy
            .timeout()
            .min(crate::execution::DEFAULT_BACKGROUND_LEASE);
        let lease_expires_at = started
            + chrono::Duration::from_std(lease).unwrap_or_else(|_| chrono::Duration::hours(1));
        Self {
            state: Some("running".to_string()),
            deadline_at: Some(policy.deadline_at.to_rfc3339()),
            lease_expires_at: Some(lease_expires_at.to_rfc3339()),
            last_progress_at: Some(started.to_rfc3339()),
            effective_timeout_ms: Some(policy.effective_timeout_ms),
            graceful_timeout_ms: Some(policy.graceful_timeout_ms),
            process_container: Some(if cfg!(unix) {
                "process_group".to_string()
            } else {
                "job_object".to_string()
            }),
            ..Self::default()
        }
    }
}

/// Directory for background task output files
pub(super) fn task_dir() -> PathBuf {
    std::env::temp_dir().join("daanio-bg-tasks")
}

pub(super) const EXIT_MARKER_PREFIX: &str = "--- Command finished with exit code: ";
const MAX_EVENT_HISTORY: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskEventKind {
    Progress,
    Checkpoint,
    Completed,
    Failed,
    Superseded,
    Cancelled,
    DeliveryUpdated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundTaskEventRecord {
    pub kind: BackgroundTaskEventKind,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<BackgroundTaskStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<BackgroundTaskProgress>,
}

/// Status file format (written to disk)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatusFile {
    pub task_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub session_id: String,
    pub status: BackgroundTaskStatus,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub duration_secs: Option<f64>,
    #[serde(default)]
    pub pid: Option<u32>,
    /// PID of the process whose in-process future owns (or owned) this task.
    /// `None` for files written by older builds. Used to reconcile phantom
    /// `Running` entries after the owning server crashes or exec-reloads;
    /// files without owner metadata are deliberately never reconciled, so a
    /// task genuinely running in another live process cannot be clobbered.
    #[serde(default)]
    pub owner_pid: Option<u32>,
    /// Per-process random instance token of the owning process. Exec-based
    /// server reloads keep the PID, so PID alone cannot distinguish "this
    /// process, task still bootstrapping" from "same PID after exec, task
    /// future is gone". A fresh token per process image resolves that.
    #[serde(default)]
    pub owner_instance: Option<String>,
    #[serde(default)]
    pub detached: bool,
    #[serde(default = "default_true")]
    pub notify: bool,
    #[serde(default)]
    pub wake: bool,
    #[serde(default)]
    pub progress: Option<BackgroundTaskProgress>,
    #[serde(default)]
    pub event_history: Vec<BackgroundTaskEventRecord>,
    /// Supervision/deadline fields are flattened for API compatibility while
    /// remaining backward-compatible with status files from older releases.
    #[serde(default, flatten)]
    pub execution: ExecutionTaskMetadata,
}

fn default_true() -> bool {
    true
}

/// Random per-process-image instance token for `owner_instance`. Generated
/// once per process image (exec-based reloads get a fresh token even though
/// the PID is unchanged), so status files can tell "this process, task still
/// alive" apart from "same PID after exec, task future is gone".
pub fn process_instance_token() -> &'static str {
    use std::sync::OnceLock;
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
}

pub(super) fn normalize_delivery(notify: bool, wake: bool) -> (bool, bool) {
    (notify || wake, wake)
}

pub(super) fn push_task_event(status: &mut TaskStatusFile, event: BackgroundTaskEventRecord) {
    status.event_history.push(event);
    let overflow = status.event_history.len().saturating_sub(MAX_EVENT_HISTORY);
    if overflow > 0 {
        status.event_history.drain(0..overflow);
    }
}

pub(super) fn progress_event_record(
    kind: BackgroundTaskEventKind,
    progress: BackgroundTaskProgress,
) -> BackgroundTaskEventRecord {
    BackgroundTaskEventRecord {
        kind,
        timestamp: Utc::now().to_rfc3339(),
        message: progress.message.clone(),
        status: Some(BackgroundTaskStatus::Running),
        exit_code: None,
        progress: Some(progress),
    }
}

fn terminal_event_kind(
    status: &BackgroundTaskStatus,
    error: Option<&str>,
) -> BackgroundTaskEventKind {
    match status {
        BackgroundTaskStatus::Completed => BackgroundTaskEventKind::Completed,
        BackgroundTaskStatus::Superseded => BackgroundTaskEventKind::Superseded,
        BackgroundTaskStatus::Failed if error == Some("Cancelled by user") => {
            BackgroundTaskEventKind::Cancelled
        }
        BackgroundTaskStatus::Failed => BackgroundTaskEventKind::Failed,
        BackgroundTaskStatus::Running => BackgroundTaskEventKind::Progress,
    }
}

pub(super) fn terminal_event_record(
    status: BackgroundTaskStatus,
    exit_code: Option<i32>,
    error: Option<&str>,
) -> BackgroundTaskEventRecord {
    BackgroundTaskEventRecord {
        kind: terminal_event_kind(&status, error),
        timestamp: Utc::now().to_rfc3339(),
        message: error.map(ToString::to_string),
        status: Some(status),
        exit_code,
        progress: None,
    }
}

pub(super) fn progress_wait_reason(
    event: Option<&BackgroundTaskEventRecord>,
) -> BackgroundTaskWaitReason {
    match event.map(|event| &event.kind) {
        Some(BackgroundTaskEventKind::Checkpoint) => BackgroundTaskWaitReason::Checkpoint,
        _ => BackgroundTaskWaitReason::Progress,
    }
}

// Progress-display formatting now lives in `daanio-background-types` (pure
// functions over BackgroundTaskProgress); re-export for existing callers.
pub use daanio_background_types::{
    format_progress_display, format_progress_summary, render_progress_bar,
};

pub(super) fn progress_equivalent(a: &BackgroundTaskProgress, b: &BackgroundTaskProgress) -> bool {
    a.kind == b.kind
        && a.percent == b.percent
        && a.message == b.message
        && a.current == b.current
        && a.total == b.total
        && a.unit == b.unit
        && a.eta_seconds == b.eta_seconds
        && a.source == b.source
}

#[derive(Debug, Clone, Default)]
pub struct RunningBackgroundProgress {
    pub task_id: String,
    pub tool_name: String,
    pub label: String,
    pub detail: Option<String>,
}

/// Information returned when a background task is started
#[derive(Debug, Clone, Serialize)]
pub struct BackgroundTaskInfo {
    pub task_id: String,
    pub output_file: PathBuf,
    pub status_file: PathBuf,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundTaskWaitReason {
    AlreadyFinished,
    Finished,
    Progress,
    Checkpoint,
    Timeout,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundTaskWaitResult {
    pub reason: BackgroundTaskWaitReason,
    pub task: TaskStatusFile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_event: Option<BackgroundTaskProgressEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_record: Option<BackgroundTaskEventRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackgroundCleanupResult {
    pub matched_files: usize,
    pub removed_files: usize,
    pub skipped_running_files: usize,
}

/// Internal tracking for a running task
pub(super) struct RunningTask {
    pub(super) task_id: String,
    pub(super) tool_name: String,
    pub(super) display_name: Option<String>,
    pub(super) session_id: String,
    pub(super) status_path: PathBuf,
    pub(super) started_at: Instant,
    pub(super) started_at_rfc3339: String,
    pub(super) delivery_flags: watch::Sender<(bool, bool)>,
    pub(super) lease_deadline: watch::Sender<TokioInstant>,
    pub(super) absolute_deadline: TokioInstant,
    pub(super) handle: JoinHandle<Result<TaskResult>>,
}

/// Result from a background task execution
pub struct TaskResult {
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub status: Option<BackgroundTaskStatus>,
    pub execution_state: Option<String>,
    pub termination_report: Option<crate::execution::TerminationReport>,
}

impl TaskResult {
    pub fn completed(exit_code: Option<i32>) -> Self {
        Self {
            exit_code,
            error: None,
            status: Some(BackgroundTaskStatus::Completed),
            execution_state: Some("completed".to_string()),
            termination_report: None,
        }
    }

    pub fn failed(exit_code: Option<i32>, error: impl Into<String>) -> Self {
        Self {
            exit_code,
            error: Some(error.into()),
            status: Some(BackgroundTaskStatus::Failed),
            execution_state: Some("failed".to_string()),
            termination_report: None,
        }
    }

    pub fn superseded(exit_code: Option<i32>, detail: impl Into<String>) -> Self {
        Self {
            exit_code,
            error: Some(detail.into()),
            status: Some(BackgroundTaskStatus::Superseded),
            execution_state: Some("superseded".to_string()),
            termination_report: None,
        }
    }

    pub fn timed_out(
        exit_code: Option<i32>,
        detail: impl Into<String>,
        report: crate::execution::TerminationReport,
    ) -> Self {
        Self {
            exit_code,
            error: Some(detail.into()),
            status: Some(BackgroundTaskStatus::Failed),
            execution_state: Some(if report.cleanup_verified {
                "timed_out".to_string()
            } else {
                "kill_failed".to_string()
            }),
            termination_report: Some(report),
        }
    }
}
