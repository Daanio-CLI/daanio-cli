//! Shared runtime process supervision primitives.
//!
//! Ordinary tool processes must have a bounded lifetime and must be contained
//! before user code can create descendants.  This module deliberately keeps
//! policy normalization and process-tree termination below the CLI/server
//! boundary so dropping an RPC client cannot disable enforcement.

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

pub const DEFAULT_FOREGROUND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub const DEFAULT_NETWORK_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_BROWSER_TIMEOUT: Duration = Duration::from_secs(2 * 60);
pub const DEFAULT_LONG_FOREGROUND_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const DEFAULT_BACKGROUND_LEASE: Duration = Duration::from_secs(60 * 60);
pub const DEFAULT_BACKGROUND_MAX_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
pub const DEFAULT_GRACEFUL_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_FORCE_VERIFY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    Foreground,
    NetworkProbe,
    BrowserAction,
    LongForeground,
    Background,
}

impl ExecutionClass {
    pub const fn default_timeout(self) -> Duration {
        match self {
            Self::Foreground => DEFAULT_FOREGROUND_TIMEOUT,
            Self::NetworkProbe => DEFAULT_NETWORK_TIMEOUT,
            Self::BrowserAction => DEFAULT_BROWSER_TIMEOUT,
            Self::LongForeground => DEFAULT_LONG_FOREGROUND_TIMEOUT,
            // Background work has a renewable one-hour lease, but its hard
            // absolute deadline is the independent 24-hour maximum.
            Self::Background => DEFAULT_BACKGROUND_MAX_LIFETIME,
        }
    }

    pub const fn maximum_timeout(self) -> Duration {
        match self {
            Self::Background => DEFAULT_BACKGROUND_MAX_LIFETIME,
            _ => DEFAULT_BACKGROUND_MAX_LIFETIME,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectiveExecutionPolicy {
    pub execution_class: ExecutionClass,
    pub effective_timeout_ms: u64,
    pub graceful_timeout_ms: u64,
    pub force_verify_timeout_ms: u64,
    pub deadline_at: DateTime<Utc>,
    pub timeout_was_normalized: bool,
}

impl EffectiveExecutionPolicy {
    /// Normalize caller input into a mandatory, bounded timeout.
    ///
    /// Missing, zero, and values that cannot be represented safely all become
    /// the class default. Values above the class maximum are clamped.
    pub fn normalize(
        execution_class: ExecutionClass,
        requested_timeout_ms: Option<u64>,
        requested_graceful_timeout_ms: Option<u64>,
    ) -> Self {
        let default = execution_class.default_timeout();
        let maximum = execution_class.maximum_timeout();
        let requested = requested_timeout_ms
            .filter(|value| *value > 0)
            .map(Duration::from_millis);
        let effective = requested.unwrap_or(default).min(maximum);
        let graceful = requested_graceful_timeout_ms
            .filter(|value| *value > 0)
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_GRACEFUL_TIMEOUT)
            .min(Duration::from_secs(30));
        let effective_timeout_ms = u64::try_from(effective.as_millis()).unwrap_or(u64::MAX);
        let graceful_timeout_ms = u64::try_from(graceful.as_millis()).unwrap_or(u64::MAX);
        let force_verify_timeout_ms =
            u64::try_from(DEFAULT_FORCE_VERIFY_TIMEOUT.as_millis()).unwrap_or(u64::MAX);
        let deadline_at = Utc::now()
            + ChronoDuration::from_std(effective).unwrap_or_else(|_| ChronoDuration::hours(24));

        Self {
            execution_class,
            effective_timeout_ms,
            graceful_timeout_ms,
            force_verify_timeout_ms,
            deadline_at,
            timeout_was_normalized: requested.map(|value| value != effective).unwrap_or(true),
        }
    }

    pub const fn timeout(&self) -> Duration {
        Duration::from_millis(self.effective_timeout_ms)
    }

    pub const fn graceful_timeout(&self) -> Duration {
        Duration::from_millis(self.graceful_timeout_ms)
    }

    pub const fn force_verify_timeout(&self) -> Duration {
        Duration::from_millis(self.force_verify_timeout_ms)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessContainerKind {
    ProcessGroup,
    #[serde(rename = "job_object")]
    WindowsJobObject,
    WindowsProcessTree,
}

#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
struct WindowsJob {
    handle: usize,
}

#[cfg(windows)]
impl WindowsJob {
    fn handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle as windows_sys::Win32::Foundation::HANDLE
    }

    fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle(), 1);
        }
    }

    fn active_process_count(&self) -> usize {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };
        let mut info = unsafe {
            std::mem::MaybeUninit::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>::zeroed().assume_init()
        };
        let ok = unsafe {
            QueryInformationJobObject(
                self.handle(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                u32::try_from(std::mem::size_of_val(&info)).unwrap_or(u32::MAX),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            0
        } else {
            info.ActiveProcesses as usize
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle());
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessContainer {
    pub kind: ProcessContainerKind,
    pub root_pid: u32,
    pub process_group_id: Option<u32>,
    pub process_start_token: Option<String>,
    #[cfg(windows)]
    #[serde(skip)]
    job: Option<std::sync::Arc<WindowsJob>>,
}

/// Synchronous last-resort cleanup for cancellation or future abortion.
///
/// Normal completion must call [`ProcessTreeGuard::disarm`] after the async
/// termination/reaping path has verified cleanup. If the owning future is
/// dropped (for example backend cancellation), this guard prevents descendants
/// from escaping merely because `tokio::process::Child` only kills the root.
pub struct ProcessTreeGuard {
    container: Option<ProcessContainer>,
}

impl ProcessTreeGuard {
    pub fn new(container: ProcessContainer) -> Self {
        Self {
            container: Some(container),
        }
    }

    pub fn disarm(&mut self) {
        self.container = None;
    }
}

impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        if let Some(container) = self.container.as_ref() {
            container.force_kill_sync();
        }
    }
}

impl ProcessContainer {
    pub fn from_persisted(
        kind: ProcessContainerKind,
        root_pid: u32,
        process_group_id: Option<u32>,
        process_start_token: Option<String>,
    ) -> Self {
        Self {
            kind,
            root_pid,
            process_group_id,
            process_start_token,
            #[cfg(windows)]
            job: None,
        }
    }

    pub fn from_pid(root_pid: u32) -> Self {
        Self::from_persisted(
            if cfg!(unix) {
                ProcessContainerKind::ProcessGroup
            } else {
                ProcessContainerKind::WindowsProcessTree
            },
            root_pid,
            cfg!(unix).then_some(root_pid),
            process_start_token(root_pid),
        )
    }

    pub fn from_child(child: &Child) -> std::io::Result<Self> {
        let root_pid = child
            .id()
            .ok_or_else(|| std::io::Error::other("spawned process has no PID"))?;
        let mut container = Self::from_pid(root_pid);
        #[cfg(windows)]
        {
            let job = attach_suspended_child_to_job(child)?;
            container.kind = ProcessContainerKind::WindowsJobObject;
            container.job = Some(std::sync::Arc::new(job));
        }
        Ok(container)
    }

    pub fn from_std_child(child: &std::process::Child) -> std::io::Result<Self> {
        let root_pid = child.id();
        let mut container = Self::from_pid(root_pid);
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            let job = attach_process_handle_to_job(child.as_raw_handle() as _)?;
            container.kind = ProcessContainerKind::WindowsJobObject;
            container.job = Some(std::sync::Arc::new(job));
        }
        Ok(container)
    }

    pub fn member_count(&self) -> usize {
        #[cfg(windows)]
        if let Some(job) = self.job.as_ref() {
            return job.active_process_count();
        }
        process_group_member_count(self.process_group_id.unwrap_or(self.root_pid))
    }

    pub fn root_identity_matches(&self) -> bool {
        process_identity_matches(self.root_pid, self.process_start_token.as_deref())
    }

    pub fn is_alive(&self) -> bool {
        #[cfg(unix)]
        {
            process_group_is_alive(self.process_group_id.unwrap_or(self.root_pid))
        }
        #[cfg(windows)]
        {
            self.job
                .as_ref()
                .map(|job| job.active_process_count() > 0)
                .unwrap_or_else(|| crate::platform::is_process_running(self.root_pid))
        }
    }

    pub fn force_kill_sync(&self) {
        #[cfg(unix)]
        {
            let _ = crate::platform::signal_detached_process_group(
                self.process_group_id.unwrap_or(self.root_pid),
                libc::SIGKILL,
            );
        }
        #[cfg(windows)]
        {
            if let Some(job) = self.job.as_ref() {
                job.terminate();
            } else {
                let _ = crate::platform::signal_detached_process_group(self.root_pid, 0);
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminationReport {
    pub graceful_termination_attempted: bool,
    pub force_kill_required: bool,
    pub cleanup_verified: bool,
    pub descendants_observed: usize,
    pub descendants_remaining: usize,
    pub signal: Option<String>,
}

pub struct SupervisedOutput {
    pub status: Option<ExitStatus>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub output_truncated: bool,
    pub policy: EffectiveExecutionPolicy,
    pub termination: Option<TerminationReport>,
}

pub struct SupervisedStatus {
    pub status: Option<ExitStatus>,
    pub policy: EffectiveExecutionPolicy,
    pub termination: Option<TerminationReport>,
}

/// Backend-owned subprocess runner for non-interactive helpers and probes.
pub struct ExecutionSupervisor;

/// Emit a structured, secret-free supervisor lifecycle event.
///
/// Command text, arguments, environment variables, and captured output are
/// intentionally excluded.
pub fn record_event(
    event: &str,
    task_id: Option<&str>,
    policy: Option<&EffectiveExecutionPolicy>,
    container: Option<&ProcessContainer>,
    reason: Option<&str>,
    report: Option<&TerminationReport>,
) {
    let payload = serde_json::json!({
        "component": "execution_supervisor",
        "event": event,
        "task_id": task_id,
        "execution_class": policy.map(|value| value.execution_class),
        "effective_timeout_ms": policy.map(|value| value.effective_timeout_ms),
        "deadline_at": policy.map(|value| value.deadline_at),
        "process_container": container.map(|value| value.kind),
        "root_pid": container.map(|value| value.root_pid),
        "process_group_id": container.and_then(|value| value.process_group_id),
        "reason": reason,
        "graceful_termination_attempted": report.map(|value| value.graceful_termination_attempted),
        "force_kill_required": report.map(|value| value.force_kill_required),
        "descendants_observed": report.map(|value| value.descendants_observed),
        "descendants_remaining": report.map(|value| value.descendants_remaining),
        "cleanup_verified": report.map(|value| value.cleanup_verified),
    });
    crate::logging::info(&payload.to_string());
}

impl ExecutionSupervisor {
    pub fn run_status_blocking(
        mut command: std::process::Command,
        policy: EffectiveExecutionPolicy,
    ) -> anyhow::Result<SupervisedStatus> {
        record_event("spawn_requested", None, Some(&policy), None, None, None);
        configure_std_command(&mut command);
        let mut child = command.spawn()?;
        let container = ProcessContainer::from_std_child(&child)?;
        let mut guard = ProcessTreeGuard::new(container.clone());
        record_event(
            "spawn_succeeded",
            None,
            Some(&policy),
            Some(&container),
            None,
            None,
        );

        let deadline = Instant::now() + policy.timeout();
        let mut status = None;
        while Instant::now() < deadline {
            if let Some(exit) = child.try_wait()? {
                status = Some(exit);
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let termination = if status.is_none() {
            record_event(
                "deadline_expired",
                None,
                Some(&policy),
                Some(&container),
                Some("absolute_deadline_exceeded"),
                None,
            );
            Some(terminate_std_process_tree(
                &mut child,
                &container,
                policy.graceful_timeout(),
                policy.force_verify_timeout(),
            ))
        } else if container.is_alive() {
            Some(terminate_std_process_tree(
                &mut child,
                &container,
                policy.graceful_timeout(),
                policy.force_verify_timeout(),
            ))
        } else {
            None
        };
        if termination
            .as_ref()
            .is_none_or(|report| report.cleanup_verified)
        {
            guard.disarm();
        }
        Ok(SupervisedStatus {
            status,
            policy,
            termination,
        })
    }

    pub async fn run_to_output(
        mut command: Command,
        policy: EffectiveExecutionPolicy,
        max_bytes_per_stream: usize,
    ) -> anyhow::Result<SupervisedOutput> {
        record_event("spawn_requested", None, Some(&policy), None, None, None);
        record_event(
            "effective_deadline_assigned",
            None,
            Some(&policy),
            None,
            None,
            None,
        );
        configure_command(&mut command);
        command
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let container = ProcessContainer::from_child(&child)?;
        record_event(
            "spawn_succeeded",
            None,
            Some(&policy),
            Some(&container),
            None,
            None,
        );
        let mut process_guard = ProcessTreeGuard::new(container.clone());
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_task = tokio::spawn(async move {
            match stdout {
                Some(stream) => drain_stream(stream, max_bytes_per_stream).await,
                None => (Vec::new(), false),
            }
        });
        let stderr_task = tokio::spawn(async move {
            match stderr {
                Some(stream) => drain_stream(stream, max_bytes_per_stream).await,
                None => (Vec::new(), false),
            }
        });

        let deadline = tokio::time::sleep(policy.timeout());
        tokio::pin!(deadline);
        let (status, termination) = tokio::select! {
            status = child.wait() => {
                let status = status?;
                record_event("process_exit_observed", None, Some(&policy), Some(&container), None, None);
                (Some(status), None)
            },
            _ = &mut deadline => {
                record_event("deadline_expired", None, Some(&policy), Some(&container), Some("absolute_deadline_exceeded"), None);
                let report = terminate_process_tree(
                    &mut child,
                    &container,
                    policy.graceful_timeout(),
                    policy.force_verify_timeout(),
                ).await;
                record_event("termination_completed", None, Some(&policy), Some(&container), Some("absolute_deadline_exceeded"), Some(&report));
                (None, Some(report))
            }
        };

        let completion_cleanup = if status.is_some() && container.is_alive() {
            Some(
                terminate_process_tree(
                    &mut child,
                    &container,
                    policy.graceful_timeout(),
                    policy.force_verify_timeout(),
                )
                .await,
            )
        } else {
            None
        };
        if completion_cleanup
            .as_ref()
            .is_some_and(|report| !report.cleanup_verified)
        {
            anyhow::bail!("helper exited but descendant cleanup verification failed");
        }

        let cleanup_verified = termination
            .as_ref()
            .or(completion_cleanup.as_ref())
            .is_none_or(|report| report.cleanup_verified);
        if cleanup_verified {
            process_guard.disarm();
        }

        let (stdout, stdout_truncated) =
            finish_drain(stdout_task, policy.force_verify_timeout()).await;
        let (stderr, stderr_truncated) =
            finish_drain(stderr_task, policy.force_verify_timeout()).await;
        Ok(SupervisedOutput {
            status,
            stdout,
            stderr,
            output_truncated: stdout_truncated || stderr_truncated,
            policy,
            termination,
        })
    }
}

async fn finish_drain(
    mut task: tokio::task::JoinHandle<(Vec<u8>, bool)>,
    timeout: Duration,
) -> (Vec<u8>, bool) {
    match tokio::time::timeout(timeout, &mut task).await {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => (Vec::new(), true),
        Err(_) => {
            task.abort();
            (Vec::new(), true)
        }
    }
}

async fn drain_stream<R>(mut reader: R, limit: usize) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut chunk = [0u8; 16 * 1024];
    let mut truncated = false;
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let keep = limit.saturating_sub(retained.len()).min(read);
                retained.extend_from_slice(&chunk[..keep]);
                truncated |= keep < read;
            }
        }
    }
    (retained, truncated)
}

/// Configure containment before spawning user code.
pub fn configure_command(command: &mut Command) {
    #[cfg(target_os = "linux")]
    ensure_linux_subreaper();

    #[cfg(unix)]
    {
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED};
        // Suspension closes the spawn/assignment race: ProcessContainer::from_child
        // assigns the process to a kill-on-close Job Object before resuming it.
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
    }
}

pub fn configure_std_command(command: &mut std::process::Command) {
    #[cfg(target_os = "linux")]
    ensure_linux_subreaper();

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED};
        command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
    }
}

#[cfg(target_os = "linux")]
fn ensure_linux_subreaper() {
    static SUBREAPER: std::sync::Once = std::sync::Once::new();
    SUBREAPER.call_once(|| {
        let result = unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) };
        if result != 0 {
            crate::logging::warn(&format!(
                "execution supervisor could not enable Linux subreaper mode: {}",
                std::io::Error::last_os_error()
            ));
        }
    });
}

#[cfg(windows)]
fn attach_suspended_child_to_job(child: &Child) -> std::io::Result<WindowsJob> {
    use windows_sys::Win32::Foundation::HANDLE;
    attach_process_handle_to_job(
        child
            .raw_handle()
            .map(|handle| handle as HANDLE)
            .ok_or_else(|| std::io::Error::other("spawned process has no Windows handle"))?,
    )
}

#[cfg(windows)]
fn attach_process_handle_to_job(
    process_handle: windows_sys::Win32::Foundation::HANDLE,
) -> std::io::Result<WindowsJob> {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject,
    };

    unsafe extern "system" {
        fn NtResumeProcess(process_handle: HANDLE) -> i32;
    }

    let raw_job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw_job.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let job = WindowsJob {
        handle: raw_job as usize,
    };

    let mut limits = unsafe {
        std::mem::MaybeUninit::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>::zeroed().assume_init()
    };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let configured = unsafe {
        SetInformationJobObject(
            job.handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            u32::try_from(std::mem::size_of_val(&limits)).unwrap_or(u32::MAX),
        )
    };
    if configured == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let assigned = unsafe { AssignProcessToJobObject(job.handle(), process_handle) };
    if assigned == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let resume_status = unsafe { NtResumeProcess(process_handle) };
    if resume_status < 0 {
        unsafe {
            TerminateJobObject(job.handle(), 1);
        }
        return Err(std::io::Error::other(format!(
            "NtResumeProcess failed with NTSTATUS {resume_status:#x}"
        )));
    }
    Ok(job)
}

/// Idempotently terminate and verify the complete recorded process container.
pub async fn terminate_process_tree(
    child: &mut Child,
    container: &ProcessContainer,
    graceful_timeout: Duration,
    force_verify_timeout: Duration,
) -> TerminationReport {
    let observed = container.member_count();
    let mut report = TerminationReport {
        graceful_termination_attempted: true,
        force_kill_required: false,
        cleanup_verified: false,
        descendants_observed: observed.saturating_sub(1),
        descendants_remaining: 0,
        signal: None,
    };
    if container.is_alive() {
        record_event(
            "graceful_termination_sent",
            None,
            None,
            Some(container),
            None,
            None,
        );
        #[cfg(unix)]
        {
            let _ = crate::platform::signal_detached_process_group(
                container.process_group_id.unwrap_or(container.root_pid),
                libc::SIGTERM,
            );
            report.signal = Some("SIGTERM".to_string());
        }
        #[cfg(windows)]
        {
            send_windows_graceful_signal(container.root_pid);
        }
        wait_until_container_stops_and_reap(child, container, graceful_timeout).await;
    }
    if container.is_alive() {
        report.force_kill_required = true;
        record_event("force_kill_sent", None, None, Some(container), None, None);
        container.force_kill_sync();
        report.signal = Some(
            if cfg!(unix) {
                "SIGKILL"
            } else {
                "TerminateProcess"
            }
            .to_string(),
        );
        wait_until_container_stops_and_reap(child, container, force_verify_timeout).await;
    }
    let _ = child.try_wait();
    let remaining = container.member_count();
    report.descendants_remaining = remaining.saturating_sub(usize::from(
        process_identity_matches(container.root_pid, container.process_start_token.as_deref())
            && crate::platform::is_process_running(container.root_pid),
    ));
    report.cleanup_verified = !container.is_alive();
    record_event(
        if report.cleanup_verified {
            "cleanup_verification_succeeded"
        } else {
            "cleanup_verification_failed"
        },
        None,
        None,
        Some(container),
        None,
        Some(&report),
    );
    report
}

fn terminate_std_process_tree(
    child: &mut std::process::Child,
    container: &ProcessContainer,
    graceful_timeout: Duration,
    force_verify_timeout: Duration,
) -> TerminationReport {
    let observed = container.member_count();
    let mut report = TerminationReport {
        graceful_termination_attempted: true,
        force_kill_required: false,
        cleanup_verified: false,
        descendants_observed: observed.saturating_sub(1),
        descendants_remaining: 0,
        signal: None,
    };
    if container.is_alive() {
        record_event(
            "graceful_termination_sent",
            None,
            None,
            Some(container),
            None,
            None,
        );
        #[cfg(unix)]
        {
            let _ = crate::platform::signal_detached_process_group(
                container.process_group_id.unwrap_or(container.root_pid),
                libc::SIGTERM,
            );
            report.signal = Some("SIGTERM".to_string());
        }
        #[cfg(windows)]
        send_windows_graceful_signal(container.root_pid);
        wait_until_std_container_stops_and_reap(child, container, graceful_timeout);
    }
    if container.is_alive() {
        report.force_kill_required = true;
        record_event("force_kill_sent", None, None, Some(container), None, None);
        container.force_kill_sync();
        report.signal = Some(
            if cfg!(unix) {
                "SIGKILL"
            } else {
                "TerminateJobObject"
            }
            .to_string(),
        );
        wait_until_std_container_stops_and_reap(child, container, force_verify_timeout);
    }
    let _ = child.try_wait();
    let remaining = container.member_count();
    report.descendants_remaining = remaining.saturating_sub(usize::from(
        process_identity_matches(container.root_pid, container.process_start_token.as_deref())
            && crate::platform::is_process_running(container.root_pid),
    ));
    report.cleanup_verified = !container.is_alive();
    record_event(
        if report.cleanup_verified {
            "cleanup_verification_succeeded"
        } else {
            "cleanup_verification_failed"
        },
        None,
        None,
        Some(container),
        None,
        Some(&report),
    );
    report
}

fn wait_until_std_container_stops_and_reap(
    child: &mut std::process::Child,
    container: &ProcessContainer,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    while container.is_alive() && Instant::now() < deadline {
        let root_reaped = child.try_wait().ok().flatten().is_some();
        if root_reaped {
            reap_adopted_group_members(container);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let root_reaped = child.try_wait().ok().flatten().is_some();
    if root_reaped {
        reap_adopted_group_members(container);
    }
}

async fn wait_until_container_stops_and_reap(
    child: &mut Child,
    container: &ProcessContainer,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    while container.is_alive() && Instant::now() < deadline {
        let root_reaped = child.try_wait().ok().flatten().is_some();
        if root_reaped {
            reap_adopted_group_members(container);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let root_reaped = child.try_wait().ok().flatten().is_some();
    if root_reaped {
        reap_adopted_group_members(container);
    }
}

#[cfg(target_os = "linux")]
fn reap_adopted_group_members(container: &ProcessContainer) {
    let pgid = container.process_group_id.unwrap_or(container.root_pid) as i32;
    loop {
        let result = unsafe { libc::waitpid(-pgid, std::ptr::null_mut(), libc::WNOHANG) };
        if result <= 0 {
            break;
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn reap_adopted_group_members(_container: &ProcessContainer) {}

/// Terminate a persisted process container when the caller no longer owns a
/// `Child` handle (for example cancellation after a client disconnect).
pub async fn terminate_process_container(
    container: &ProcessContainer,
    graceful_timeout: Duration,
    force_verify_timeout: Duration,
) -> TerminationReport {
    let observed = container.member_count();
    let mut report = TerminationReport {
        graceful_termination_attempted: true,
        force_kill_required: false,
        cleanup_verified: false,
        descendants_observed: observed.saturating_sub(1),
        descendants_remaining: 0,
        signal: None,
    };

    match process_identity_state(container.root_pid, container.process_start_token.as_deref()) {
        ProcessIdentityState::Matches | ProcessIdentityState::Exited => {}
        ProcessIdentityState::Reused | ProcessIdentityState::Unverifiable => {
            if !container.is_alive() {
                report.cleanup_verified = true;
                report.descendants_remaining = 0;
                return report;
            }
            // A live group whose recorded root identity was reused or cannot
            // be verified may belong to a later unrelated task.
            report.descendants_remaining = observed;
            return report;
        }
    }

    if container.is_alive() {
        record_event(
            "graceful_termination_sent",
            None,
            None,
            Some(container),
            None,
            None,
        );
        #[cfg(unix)]
        {
            let _ = crate::platform::signal_detached_process_group(
                container.process_group_id.unwrap_or(container.root_pid),
                libc::SIGTERM,
            );
            report.signal = Some("SIGTERM".to_string());
        }
        #[cfg(windows)]
        {
            send_windows_graceful_signal(container.root_pid);
        }

        wait_until_container_stops(container, graceful_timeout).await;
    }

    if container.is_alive() {
        report.force_kill_required = true;
        record_event("force_kill_sent", None, None, Some(container), None, None);
        container.force_kill_sync();
        report.signal = Some(
            if cfg!(unix) {
                "SIGKILL"
            } else {
                "TerminateProcess"
            }
            .to_string(),
        );
        wait_until_container_stops(container, force_verify_timeout).await;
    }

    let remaining = container.member_count();
    report.descendants_remaining = remaining.saturating_sub(usize::from(
        process_identity_matches(container.root_pid, container.process_start_token.as_deref())
            && crate::platform::is_process_running(container.root_pid),
    ));
    report.cleanup_verified = !container.is_alive();
    record_event(
        if report.cleanup_verified {
            "cleanup_verification_succeeded"
        } else {
            "cleanup_verification_failed"
        },
        None,
        None,
        Some(container),
        None,
        Some(&report),
    );
    report
}

async fn wait_until_container_stops(container: &ProcessContainer, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while container.is_alive() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[cfg(windows)]
fn send_windows_graceful_signal(process_group_id: u32) {
    unsafe {
        // CREATE_NEW_PROCESS_GROUP makes the root PID the console process-group
        // identifier. Delivery may fail for GUI or detached-console children;
        // the bounded grace period then escalates through TerminateJobObject.
        let _ = windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent(
            windows_sys::Win32::System::Console::CTRL_BREAK_EVENT,
            process_group_id,
        );
    }
}

fn process_identity_matches(pid: u32, expected: Option<&str>) -> bool {
    matches!(
        process_identity_state(pid, expected),
        ProcessIdentityState::Matches
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessIdentityState {
    Matches,
    Exited,
    Reused,
    Unverifiable,
}

fn process_identity_state(pid: u32, expected: Option<&str>) -> ProcessIdentityState {
    match (expected, process_start_token(pid)) {
        (Some(expected), Some(actual)) if actual == expected => ProcessIdentityState::Matches,
        (Some(_), Some(_)) => ProcessIdentityState::Reused,
        (Some(_), None) => ProcessIdentityState::Exited,
        (None, _) => ProcessIdentityState::Unverifiable,
    }
}

#[cfg(target_os = "linux")]
fn process_start_token(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 2..)?;
    // Fields after comm begin at field 3; starttime is field 22.
    rest.split_whitespace().nth(19).map(str::to_string)
}

#[cfg(windows)]
fn process_start_token(pid: u32) -> Option<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut created = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exited = created;
        let mut kernel = created;
        let mut user = created;
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user);
        CloseHandle(handle);
        (ok != 0).then(|| {
            format!(
                "{:08x}{:08x}",
                created.dwHighDateTime, created.dwLowDateTime
            )
        })
    }
}

#[cfg(target_os = "macos")]
fn process_start_token(pid: u32) -> Option<String> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = i32::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).ok()?;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if read != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    Some(format!(
        "{}:{:06}",
        info.pbi_start_tvsec, info.pbi_start_tvusec
    ))
}

#[cfg(all(not(target_os = "linux"), not(target_os = "macos"), not(windows)))]
fn process_start_token(_pid: u32) -> Option<String> {
    None
}

#[cfg(unix)]
fn process_group_is_alive(pgid: u32) -> bool {
    let rc = unsafe { libc::kill(-(pgid as i32), 0) };
    if rc == 0 {
        return true;
    }
    !matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(code) if code == libc::ESRCH
    )
}

#[cfg(target_os = "linux")]
fn process_group_member_count(pgid: u32) -> usize {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return usize::from(process_group_is_alive(pgid));
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return false;
            };
            let Some(close) = stat.rfind(')') else {
                return false;
            };
            stat.get(close + 2..)
                .and_then(|rest| rest.split_whitespace().nth(2))
                .and_then(|value| value.parse::<u32>().ok())
                == Some(pgid)
        })
        .count()
}

#[cfg(target_os = "macos")]
fn process_group_member_count(pgid: u32) -> usize {
    let count = unsafe { libc::proc_listpgrppids(pgid as i32, std::ptr::null_mut(), 0) };
    if count <= 0 {
        return usize::from(process_group_is_alive(pgid));
    }
    // The sizing call returns bytes needed.
    let slots = (count as usize).div_ceil(std::mem::size_of::<libc::pid_t>());
    let mut pids = vec![0 as libc::pid_t; slots];
    let bytes = unsafe {
        libc::proc_listpgrppids(
            pgid as i32,
            pids.as_mut_ptr().cast(),
            i32::try_from(pids.len() * std::mem::size_of::<libc::pid_t>()).unwrap_or(i32::MAX),
        )
    };
    if bytes <= 0 {
        return usize::from(process_group_is_alive(pgid));
    }
    (bytes as usize / std::mem::size_of::<libc::pid_t>()).min(pids.len())
}

#[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
fn process_group_member_count(pgid: u32) -> usize {
    usize::from(process_group_is_alive(pgid))
}

#[cfg(windows)]
fn process_group_member_count(pid: u32) -> usize {
    usize::from(crate::platform::is_process_running(pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_and_zero_timeouts_are_bounded() {
        let omitted = EffectiveExecutionPolicy::normalize(ExecutionClass::Foreground, None, None);
        let zero = EffectiveExecutionPolicy::normalize(ExecutionClass::Foreground, Some(0), None);
        assert_eq!(
            omitted.effective_timeout_ms,
            DEFAULT_FOREGROUND_TIMEOUT.as_millis() as u64
        );
        assert_eq!(zero.effective_timeout_ms, omitted.effective_timeout_ms);
        assert!(omitted.timeout_was_normalized);
        assert!(zero.timeout_was_normalized);
    }

    #[test]
    fn background_timeout_is_clamped_to_maximum() {
        let policy =
            EffectiveExecutionPolicy::normalize(ExecutionClass::Background, Some(u64::MAX), None);
        assert_eq!(
            policy.effective_timeout_ms,
            DEFAULT_BACKGROUND_MAX_LIFETIME.as_millis() as u64
        );
        assert!(policy.timeout_was_normalized);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silent_process_timeout_removes_process_group() {
        let mut command = Command::new("bash");
        command.arg("-c").arg("sleep 60");
        configure_command(&mut command);
        let mut child = command.spawn().expect("spawn");
        let container = ProcessContainer::from_child(&child).expect("container");
        tokio::time::sleep(Duration::from_millis(50)).await;
        let report = terminate_process_tree(
            &mut child,
            &container,
            Duration::from_millis(100),
            Duration::from_secs(2),
        )
        .await;
        assert!(report.cleanup_verified);
        assert_eq!(report.descendants_remaining, 0);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stale_start_token_does_not_signal_reused_pid() {
        let mut command = Command::new("bash");
        command.arg("-c").arg("sleep 60");
        configure_command(&mut command);
        let mut child = command.spawn().expect("spawn");
        let real = ProcessContainer::from_child(&child).expect("container");
        let mut stale = real.clone();
        stale.process_start_token = Some("definitely-not-the-real-start-time".to_string());

        let report = terminate_process_container(
            &stale,
            Duration::from_millis(50),
            Duration::from_millis(50),
        )
        .await;
        assert!(!report.cleanup_verified);
        assert!(
            real.is_alive(),
            "identity mismatch must preserve unrelated process"
        );

        let cleanup = terminate_process_tree(
            &mut child,
            &real,
            Duration::from_millis(50),
            Duration::from_secs(2),
        )
        .await;
        assert!(cleanup.cleanup_verified);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn exited_root_does_not_hide_owned_process_group() {
        let mut command = Command::new("bash");
        command.arg("-c").arg("sleep 60 & exit 0");
        configure_command(&mut command);
        let mut child = command.spawn().expect("spawn");
        let container = ProcessContainer::from_child(&child).expect("container");
        child.wait().await.expect("root exits");
        assert!(
            container.is_alive(),
            "the descendant should keep the recorded process group alive"
        );

        let report = terminate_process_container(
            &container,
            Duration::from_millis(100),
            Duration::from_secs(2),
        )
        .await;
        assert!(report.cleanup_verified);
        assert_eq!(report.descendants_remaining, 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn signal_ignoring_process_tree_is_force_killed() {
        let mut command = Command::new("bash");
        command.arg("-c").arg(
            "trap '' TERM; (trap '' TERM; while :; do sleep 1; done) & while :; do sleep 1; done",
        );
        configure_command(&mut command);
        let mut child = command.spawn().expect("spawn");
        let container = ProcessContainer::from_child(&child).expect("container");
        tokio::time::sleep(Duration::from_millis(100)).await;
        let report = terminate_process_tree(
            &mut child,
            &container,
            Duration::from_millis(100),
            Duration::from_secs(2),
        )
        .await;
        assert!(report.graceful_termination_attempted);
        assert!(report.force_kill_required);
        assert!(report.cleanup_verified);
        assert_eq!(report.descendants_remaining, 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn full_stdout_and_stderr_are_drained_and_bounded() {
        let mut command = Command::new("bash");
        command
            .arg("-c")
            .arg("while :; do printf 'stdout-flood\\n'; printf 'stderr-flood\\n' >&2; done");
        let policy =
            EffectiveExecutionPolicy::normalize(ExecutionClass::Foreground, Some(200), Some(50));
        let output = ExecutionSupervisor::run_to_output(command, policy, 4096)
            .await
            .expect("supervised flood");
        assert!(output.termination.is_some());
        assert!(output.output_truncated);
        assert!(output.stdout.len() <= 4096);
        assert!(output.stderr.len() <= 4096);
        assert!(
            output
                .termination
                .as_ref()
                .is_some_and(|report| report.cleanup_verified)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hung_network_reader_is_stopped_by_outer_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let port = listener.local_addr().expect("address").port();
        let server = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.expect("accept");
            std::future::pending::<()>().await;
        });

        let mut command = Command::new("bash");
        command.arg("-c").arg(format!(
            "exec 3<>/dev/tcp/127.0.0.1/{port}; read -r response <&3"
        ));
        let policy =
            EffectiveExecutionPolicy::normalize(ExecutionClass::NetworkProbe, Some(250), Some(50));
        let output = ExecutionSupervisor::run_to_output(command, policy, 1024)
            .await
            .expect("supervised network reader");
        server.abort();

        let report = output.termination.expect("outer deadline should fire");
        assert!(report.cleanup_verified);
        assert_eq!(report.descendants_remaining, 0);
    }
}
