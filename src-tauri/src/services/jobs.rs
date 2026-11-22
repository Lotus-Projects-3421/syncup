//! Crash-safe, cancelable in-memory job registry with typed progress events.
//!
//! Jobs are process-local: nothing here is persisted, so an application exit
//! discards every job record (PLAN.md §8, Phase 4). Workflow services in Phases
//! 5-8 will register a job, drive progress from their own tasks, and call
//! [`JobHandle::finish`] exactly once.
//!
//! Progress events are emitted on the `job://progress` Tauri event at most once
//! per 100 ms per job. Status transitions always bypass that limit so the UI can
//! never miss a terminal state.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::ipc::error::IpcError;

/// Tauri event name carrying [`JobProgressEvent`] payloads.
pub const JOB_PROGRESS_EVENT: &str = "job://progress";

/// Minimum spacing between progress-only events for a single job (10 events/sec).
const MIN_EMIT_INTERVAL: Duration = Duration::from_millis(100);

/// Stable identifier for a registered job.
///
/// Serialized as a hyphenated UUID string; the [`Type`] implementation maps it to
/// `string` so the generated TypeScript surface matches that representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(Uuid);

impl JobId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl Type for JobId {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        <String as Type>::definition(types)
    }
}

/// Operation family a job belongs to.
///
/// Every variant is reserved for a command scheduled in PLAN.md §8; no workflow
/// command exists yet in Phase 4c.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    /// `start_backup` — PLAN.md §8 Phase 5 (Linux MTP) and Phase 6 (Wi-Fi).
    Backup,
    /// `start_wifi_session` — PLAN.md §8 Phase 6.
    Wifi,
    /// `start_organizer` — PLAN.md §8 Phase 7.
    Organizer,
    /// `scan_duplicates` / `move_duplicates` / `undo_duplicates` — PLAN.md §8 Phase 8.
    Dedup,
    /// `run_migration` — PLAN.md §4, scheduled within §8 Phases 5-10.
    Migration,
}

/// Lifecycle state of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl JobStatus {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Counted progress for a job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobProgress {
    // Counts are bounded well below 2^53, so the JSON number is exact.
    #[specta(type = specta_typescript::Number)]
    pub current: u64,
    #[specta(type = Option<specta_typescript::Number>)]
    pub total: Option<u64>,
    pub message: Option<String>,
}

impl JobProgress {
    /// Fractional progress in `0.0..=1.0`, or `None` when the total is unknown or zero.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        match self.total {
            Some(total) if total > 0 => Some(self.current as f64 / total as f64),
            _ => None,
        }
    }
}

/// Public snapshot of a job.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: JobId,
    pub kind: JobKind,
    pub status: JobStatus,
    pub progress: JobProgress,
    /// Unix epoch seconds. Bounded below 2^53, so the JSON number is exact.
    #[specta(type = specta_typescript::Number)]
    pub started_at: u64,
    /// Unix epoch seconds; `None` until the job reaches a terminal state.
    #[specta(type = Option<specta_typescript::Number>)]
    pub finished_at: Option<u64>,
    pub last_error: Option<String>,
}

/// Payload of the `job://progress` event.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JobProgressEvent {
    pub job_id: JobId,
    pub status: JobStatus,
    pub progress: JobProgress,
}

/// Handle used by a workflow task to report progress and finish its job.
#[derive(Clone)]
pub struct JobHandle {
    inner: Arc<Mutex<Job>>,
    cancel: CancellationToken,
    app: Option<AppHandle>,
    last_emit: Arc<Mutex<Option<Instant>>>,
}

impl JobHandle {
    fn new(id: JobId, kind: JobKind, app: Option<AppHandle>) -> Self {
        let job = Job {
            id,
            kind,
            status: JobStatus::Pending,
            progress: JobProgress::default(),
            started_at: now_epoch_seconds(),
            finished_at: None,
            last_error: None,
        };

        Self {
            inner: Arc::new(Mutex::new(job)),
            cancel: CancellationToken::new(),
            app,
            last_emit: Arc::new(Mutex::new(None)),
        }
    }

    /// Replace the current progress snapshot.
    ///
    /// The first call transitions a pending job to running and always emits a
    /// status event. Later calls are coalesced to at most one event per 100 ms.
    /// Updates after a terminal state are ignored.
    pub fn update_progress(&self, progress: JobProgress) {
        let (snapshot, transitioned) = {
            let mut job = lock_mutex(&self.inner);
            if job.status.is_terminal() {
                return;
            }

            let transitioned = job.status == JobStatus::Pending;
            job.status = JobStatus::Running;
            job.progress = progress;
            (job.clone(), transitioned)
        };

        self.emit(&snapshot, transitioned);
    }

    /// Move the job into a terminal state exactly once.
    ///
    /// A second call is ignored so the first terminal outcome wins.
    pub fn finish(&self, status: JobStatus, error: Option<String>) {
        let snapshot = {
            let mut job = lock_mutex(&self.inner);
            if job.status.is_terminal() {
                return;
            }

            job.status = status;
            job.finished_at = Some(now_epoch_seconds());
            job.last_error = error;
            job.clone()
        };

        self.emit(&snapshot, true);
    }

    #[must_use]
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    fn snapshot(&self) -> Job {
        lock_mutex(&self.inner).clone()
    }

    fn emit(&self, job: &Job, force: bool) {
        let Some(app) = self.app.as_ref() else {
            return;
        };

        // Forced (transition) events also reset the rate-limit window.
        let allowed = record_emit(&self.last_emit);
        if !force && !allowed {
            return;
        }

        let payload = JobProgressEvent {
            job_id: job.id,
            status: job.status,
            progress: job.progress.clone(),
        };

        if let Err(error) = app.emit(JOB_PROGRESS_EVENT, payload) {
            tracing::warn!(
                job_id = %job.id,
                error = %error,
                "failed to emit job progress event"
            );
        }
    }
}

/// In-memory registry of active and finished jobs.
#[derive(Clone)]
pub struct JobRegistry {
    jobs: Arc<RwLock<HashMap<JobId, JobHandle>>>,
    app: Option<AppHandle>,
}

impl JobRegistry {
    /// Create a registry.
    ///
    /// `app` enables `job://progress` event emission. `None` keeps the registry
    /// fully functional without an event sink (used by unit tests).
    #[must_use]
    pub fn new(app: Option<AppHandle>) -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            app,
        }
    }

    /// Register a pending job and return its id, handle, and cancellation token.
    pub fn register(&self, kind: JobKind) -> (JobId, JobHandle, CancellationToken) {
        let id = JobId::new();
        let handle = JobHandle::new(id, kind, self.app.clone());
        let cancel = handle.cancel_token();

        lock_rw_write(&self.jobs).insert(id, handle.clone());

        (id, handle, cancel)
    }

    /// Snapshot every job, ordered by start time for deterministic UI output.
    #[must_use]
    pub fn list(&self) -> Vec<Job> {
        let jobs = lock_rw_read(&self.jobs);
        let mut snapshot: Vec<Job> = jobs.values().map(JobHandle::snapshot).collect();
        snapshot.sort_by_key(|job| (job.started_at, job.id));
        snapshot
    }

    #[must_use]
    pub fn get(&self, id: &JobId) -> Option<Job> {
        lock_rw_read(&self.jobs).get(id).map(JobHandle::snapshot)
    }

    /// Request cooperative cancellation of a job.
    ///
    /// The job stays in its current state until its workflow observes the token
    /// and calls [`JobHandle::finish`] with [`JobStatus::Cancelled`].
    pub fn cancel(&self, id: &JobId) -> Result<(), IpcError> {
        let jobs = lock_rw_read(&self.jobs);
        let handle = jobs.get(id).ok_or_else(|| job_not_found(id))?;
        handle.cancel_token().cancel();
        Ok(())
    }
}

/// Structured validation error for an unknown job id.
pub(crate) fn job_not_found(id: &JobId) -> IpcError {
    IpcError::validation(&format!("Job {id} was not found"))
}

fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_rw_read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn lock_rw_write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn record_emit(last_emit: &Mutex<Option<Instant>>) -> bool {
    let now = Instant::now();
    let mut last = lock_mutex(last_emit);
    let allowed = should_emit(*last, now);

    if allowed {
        *last = Some(now);
    }

    allowed
}

fn should_emit(last: Option<Instant>, now: Instant) -> bool {
    match last {
        Some(previous) => now.duration_since(previous) >= MIN_EMIT_INTERVAL,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::error::VALIDATION_ERROR;

    fn registry() -> JobRegistry {
        JobRegistry::new(None)
    }

    #[test]
    fn fraction_is_none_without_a_known_non_zero_total() {
        let unknown = JobProgress {
            current: 3,
            total: None,
            message: None,
        };
        assert_eq!(unknown.fraction(), None);

        let zero = JobProgress {
            current: 0,
            total: Some(0),
            message: None,
        };
        assert_eq!(zero.fraction(), None);

        let half = JobProgress {
            current: 5,
            total: Some(10),
            message: None,
        };
        assert_eq!(half.fraction(), Some(0.5));
    }

    #[test]
    fn register_starts_pending_with_a_unique_id() {
        let registry = registry();
        let (first, first_handle, _first_cancel) = registry.register(JobKind::Backup);
        let (second, _second_handle, _second_cancel) = registry.register(JobKind::Dedup);

        assert_ne!(first, second);

        let job = registry.get(&first);
        assert!(job.is_some(), "registered job should be retrievable");
        if let Some(job) = job {
            assert_eq!(job.kind, JobKind::Backup);
            assert_eq!(job.status, JobStatus::Pending);
            assert_eq!(job.progress, JobProgress::default());
            assert!(job.finished_at.is_none());
            assert!(job.last_error.is_none());
        }

        first_handle.finish(JobStatus::Completed, None);
    }

    #[test]
    fn first_progress_update_transitions_to_running() {
        let registry = registry();
        let (id, handle, _cancel) = registry.register(JobKind::Organizer);

        handle.update_progress(JobProgress {
            current: 4,
            total: Some(10),
            message: Some("copying".to_owned()),
        });

        let job = registry.get(&id);
        assert!(job.is_some(), "registered job should exist");
        if let Some(job) = job {
            assert_eq!(job.status, JobStatus::Running);
            assert_eq!(job.progress.current, 4);
            assert_eq!(job.progress.total, Some(10));
            assert_eq!(job.progress.message.as_deref(), Some("copying"));
            assert_eq!(job.progress.fraction(), Some(0.4));
        }
    }

    #[test]
    fn terminal_jobs_ignore_late_progress_and_second_finish() {
        let registry = registry();
        let (id, handle, _cancel) = registry.register(JobKind::Migration);

        handle.finish(JobStatus::Failed, Some("boom".to_owned()));
        handle.update_progress(JobProgress {
            current: 99,
            total: Some(100),
            message: None,
        });
        handle.finish(JobStatus::Completed, None);

        let job = registry.get(&id);
        assert!(job.is_some(), "registered job should exist");
        if let Some(job) = job {
            assert_eq!(job.status, JobStatus::Failed);
            assert_eq!(job.progress.current, 0);
            assert_eq!(job.last_error.as_deref(), Some("boom"));
            assert!(job.finished_at.is_some());
        }
    }

    #[test]
    fn cancel_cancels_the_jobs_token() {
        let registry = registry();
        let (id, _handle, cancel) = registry.register(JobKind::Wifi);

        assert!(!cancel.is_cancelled());
        assert!(registry.cancel(&id).is_ok());
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn cancel_unknown_job_reports_validation_error_with_id() {
        let registry = registry();
        let missing = JobId::new();

        let result = registry.cancel(&missing);
        assert!(result.is_err(), "unknown job must not cancel successfully");
        if let Err(error) = result {
            assert_eq!(error.code, VALIDATION_ERROR);
            assert!(error.message.contains(&missing.to_string()));
        }
    }

    #[test]
    fn list_returns_a_snapshot_of_all_jobs() {
        let registry = registry();
        let (first, first_handle, _first_cancel) = registry.register(JobKind::Backup);
        let (second, _second_handle, _second_cancel) = registry.register(JobKind::Dedup);

        first_handle.update_progress(JobProgress {
            current: 1,
            total: Some(2),
            message: None,
        });

        let jobs = registry.list();
        assert_eq!(jobs.len(), 2);
        assert!(jobs
            .iter()
            .any(|job| job.id == first && job.status == JobStatus::Running));
        assert!(jobs
            .iter()
            .any(|job| job.id == second && job.status == JobStatus::Pending));
    }

    #[test]
    fn emit_predicate_allows_first_event_and_enforces_interval() {
        let start = Instant::now();

        assert!(should_emit(None, start));
        assert!(!should_emit(Some(start), start + Duration::from_millis(99)));
        assert!(should_emit(Some(start), start + Duration::from_millis(100)));
    }
}
