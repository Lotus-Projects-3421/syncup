use crate::domain::dedup::quick_hash;
use crate::ipc::error::IpcError;
use crate::ipc::models::{DuplicateFile, DuplicateGroup, ScanConfig};
use crate::services::fileops::compute_file_hash;
use crate::services::jobs::{JobHandle, JobStatus};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ScanSession {
    pub canonical_roots: Vec<PathBuf>,
    pub groups: Vec<DuplicateGroup>,
}

#[derive(Clone, Default)]
pub struct ScanResultStore {
    inner: Arc<RwLock<HashMap<String, ScanSession>>>,
}

impl ScanResultStore {
    pub fn insert(&self, id: String, session: ScanSession) {
        if let Ok(mut map) = self.inner.write() {
            map.insert(id, session);
        }
    }
    pub fn get(&self, id: &str) -> Option<Vec<DuplicateGroup>> {
        self.inner.read().ok()?.get(id).map(|s| s.groups.clone())
    }
    #[allow(dead_code)]
    pub fn get_session(&self, id: &str) -> Option<ScanSession> {
        self.inner.read().ok()?.get(id).cloned()
    }
    pub fn remove(&self, id: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(id);
        }
    }
}

fn find_best_root_depth(path: &Path, roots: &[PathBuf]) -> usize {
    let mut best_depth = usize::MAX;
    for root in roots {
        if let Ok(rel) = path.strip_prefix(root) {
            let depth = rel.components().count();
            if depth < best_depth {
                best_depth = depth;
            }
        }
    }
    best_depth
}

pub async fn run_scan(
    config: ScanConfig,
    job_id: String,
    handle: &JobHandle,
    store: &ScanResultStore,
    cancel: CancellationToken,
) -> Result<(), IpcError> {
    let mut canonical_roots = Vec::new();
    for root_str in &config.scan_roots {
        let root = PathBuf::from(root_str);
        let meta = match fs::symlink_metadata(&root) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(root = %root.display(), error = %e, "Root inaccessible, skipping");
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            tracing::warn!(root = %root.display(), "Root is a symlink, skipping");
            continue;
        }
        if !meta.is_dir() {
            tracing::warn!(root = %root.display(), "Root is not a directory, skipping");
            continue;
        }
        match fs::canonicalize(&root) {
            Ok(canon) => {
                if !canonical_roots.contains(&canon) {
                    canonical_roots.push(canon);
                }
            }
            Err(e) => {
                tracing::warn!(root = %root.display(), error = %e, "Cannot canonicalize root, skipping");
            }
        }
    }

    if canonical_roots.is_empty() {
        return Err(IpcError::validation("No valid scan roots provided"));
    }

    let mut seen_files: HashSet<PathBuf> = HashSet::new();
    let mut records: Vec<(PathBuf, u64, i64)> = Vec::new();

    for root in &canonical_roots {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Scan cancelled"));
        }
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            if cancel.is_cancelled() {
                return Err(IpcError::cancelled("Scan cancelled"));
            }
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(dir = %dir.display(), error = %e, "Cannot read directory");
                    continue;
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = entry.path();
                let meta = match fs::symlink_metadata(&path) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if meta.file_type().is_symlink() {
                    continue;
                }

                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if name_str.starts_with('.') || name_str.contains(".syncup-part-") {
                    continue;
                }

                let mut excluded = false;
                for pattern in &config.exclude_patterns {
                    if !pattern.is_empty() && name_str.contains(pattern.as_str()) {
                        excluded = true;
                        break;
                    }
                }
                if excluded {
                    continue;
                }

                if meta.is_dir() {
                    if name_str.starts_with("_Duplicates_") {
                        continue;
                    }
                    stack.push(path);
                } else if meta.is_file() && meta.len() > 0 {
                    let canon = match fs::canonicalize(&path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    if seen_files.insert(canon.clone()) {
                        let mtime_ns = meta
                            .modified()
                            .map(|t| {
                                t.duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_nanos() as i64
                            })
                            .unwrap_or(0);
                        records.push((canon, meta.len(), mtime_ns));
                    }
                }
            }
        }
    }

    if records.is_empty() {
        store.insert(
            job_id,
            ScanSession {
                canonical_roots,
                groups: Vec::new(),
            },
        );
        handle.finish(JobStatus::Completed, None);
        return Ok(());
    }

    let mut by_size: HashMap<u64, Vec<(PathBuf, u64, i64)>> = HashMap::new();
    for record in records {
        by_size.entry(record.1).or_default().push(record);
    }

    let size_groups: Vec<Vec<(PathBuf, u64, i64)>> =
        by_size.into_values().filter(|g| g.len() > 1).collect();
    if size_groups.is_empty() {
        store.insert(
            job_id,
            ScanSession {
                canonical_roots,
                groups: Vec::new(),
            },
        );
        handle.finish(JobStatus::Completed, None);
        return Ok(());
    }

    #[allow(clippy::type_complexity)]
    let mut quick_groups: HashMap<(u64, [u8; 16]), Vec<(PathBuf, u64, i64)>> = HashMap::new();
    for group in size_groups {
        for record in group {
            if cancel.is_cancelled() {
                return Err(IpcError::cancelled("Scan cancelled"));
            }
            match quick_hash(&record.0, record.1) {
                Ok(qh) => {
                    quick_groups.entry((record.1, qh)).or_default().push(record);
                }
                Err(e) => {
                    tracing::warn!(file = %record.0.display(), error = %e, "Quick hash failed");
                }
            }
        }
    }

    let candidates: Vec<Vec<(PathBuf, u64, i64)>> =
        quick_groups.into_values().filter(|g| g.len() > 1).collect();
    if candidates.is_empty() {
        store.insert(
            job_id,
            ScanSession {
                canonical_roots,
                groups: Vec::new(),
            },
        );
        handle.finish(JobStatus::Completed, None);
        return Ok(());
    }

    #[allow(clippy::type_complexity)]
    let mut full_groups: HashMap<(u64, [u8; 64]), Vec<(PathBuf, u64, i64)>> = HashMap::new();
    for group in candidates {
        for record in group {
            if cancel.is_cancelled() {
                return Err(IpcError::cancelled("Scan cancelled"));
            }
            match compute_file_hash(&record.0) {
                Ok(fh) => {
                    full_groups.entry((record.1, fh)).or_default().push(record);
                }
                Err(e) => {
                    tracing::warn!(file = %record.0.display(), error = %e, "Full hash failed");
                }
            }
        }
    }

    let mut groups: Vec<DuplicateGroup> = Vec::new();
    for ((size, hash), mut group) in full_groups {
        if group.len() < 2 {
            continue;
        }
        group.sort_by(|a, b| {
            let depth_a = find_best_root_depth(&a.0, &canonical_roots);
            let depth_b = find_best_root_depth(&b.0, &canonical_roots);
            let key_a = (depth_a, a.2, a.0.to_string_lossy().to_string());
            let key_b = (depth_b, b.2, b.0.to_string_lossy().to_string());
            key_a.cmp(&key_b)
        });

        let keep_record = &group[0];
        let keep = DuplicateFile {
            path: keep_record.0.to_string_lossy().to_string(),
            size: keep_record.1,
            mtime_ns: keep_record.2,
        };
        let duplicates: Vec<DuplicateFile> = group
            .into_iter()
            .skip(1)
            .map(|r| DuplicateFile {
                path: r.0.to_string_lossy().to_string(),
                size: r.1,
                mtime_ns: r.2,
            })
            .collect();

        groups.push(DuplicateGroup {
            size,
            hash: hex::encode(hash),
            keep,
            duplicates,
        });
    }

    store.insert(
        job_id,
        ScanSession {
            canonical_roots,
            groups,
        },
    );
    handle.finish(JobStatus::Completed, None);
    Ok(())
}

pub async fn run_move(
    _scan_job_id: String,
    _confirmed: bool,
    handle: &JobHandle,
    _store: &ScanResultStore,
    _data_dir: &Path,
    _cancel: CancellationToken,
) -> Result<(), IpcError> {
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
