use crate::ipc::error::IpcError;
use crate::ipc::models::{DuplicateGroup, ScanConfig};
use crate::services::jobs::{JobHandle, JobStatus};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Default)]
pub struct ScanResultStore {
    inner: Arc<RwLock<HashMap<String, Vec<DuplicateGroup>>>>,
}

impl ScanResultStore {
    pub fn insert(&self, id: String, groups: Vec<DuplicateGroup>) {
        if let Ok(mut map) = self.inner.write() {
            map.insert(id, groups);
        }
    }
    pub fn get(&self, id: &str) -> Option<Vec<DuplicateGroup>> {
        self.inner.read().ok()?.get(id).cloned()
    }
    pub fn remove(&self, id: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(id);
        }
    }
}

pub async fn run_scan(
    _config: ScanConfig,
    job_id: String,
    handle: &JobHandle,
    store: &ScanResultStore,
    _cancel: CancellationToken,
) -> Result<(), IpcError> {
    let groups: Vec<DuplicateGroup> = vec![];
    store.insert(job_id, groups);
    handle.finish(JobStatus::Completed, None);
    Ok(())
}

pub async fn run_move(
    scan_job_id: String,
    confirmed: bool,
    handle: &JobHandle,
    store: &ScanResultStore,
    _data_dir: &Path,
    _cancel: CancellationToken,
) -> Result<(), IpcError> {
    if !confirmed {
        return Err(IpcError::validation("Move not confirmed"));
    }
    let _groups = store
        .get(&scan_job_id)
        .ok_or_else(|| IpcError::validation("Scan results not found"))?;
    store.remove(&scan_job_id);
    handle.finish(JobStatus::Completed, None);
    Ok(())
}

pub async fn run_undo(
    _manifest_id: String,
    handle: &JobHandle,
    _data_dir: &Path,
    _cancel: CancellationToken,
) -> Result<(), IpcError> {
    handle.finish(JobStatus::Completed, None);
    Ok(())
}

pub fn reconcile_manifests_on_startup(_data_dir: &Path) -> Result<(), IpcError> {
    Ok(())
}
