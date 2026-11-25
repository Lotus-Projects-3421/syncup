use crate::domain::dedup::quick_hash;
use crate::ipc::error::IpcError;
use crate::ipc::models::{
    DuplicateFile, DuplicateGroup, ManifestEntry, OperationStatus, ScanConfig,
};
use crate::services::fileops::{compute_file_hash, move_verified, Destination, VerifyMode};
use crate::services::jobs::{JobHandle, JobProgress, JobStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
    pub fn get_session(&self, id: &str) -> Option<ScanSession> {
        self.inner.read().ok()?.get(id).cloned()
    }
    pub fn remove(&self, id: &str) {
        if let Ok(mut map) = self.inner.write() {
            map.remove(id);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub manifest_id: String,
    pub scan_job_id: String,
    pub created_at: String,
    pub ops: Vec<ManifestEntry>,
}

fn write_manifest_atomic(manifest_path: &Path, manifest: &Manifest) -> Result<(), IpcError> {
    let parent = manifest_path
        .parent()
        .ok_or_else(|| IpcError::validation("Invalid manifest path"))?;
    fs::create_dir_all(parent).map_err(|e| IpcError::io(&e.to_string()))?;

    let mut temp_file = tempfile::Builder::new()
        .prefix(".manifest-")
        .tempfile_in(parent)
        .map_err(|e| IpcError::io(&e.to_string()))?;

    serde_json::to_writer_pretty(&temp_file, manifest).map_err(|e| IpcError::io(&e.to_string()))?;
    temp_file
        .flush()
        .map_err(|e| IpcError::io(&e.to_string()))?;
    temp_file
        .as_file()
        .sync_all()
        .map_err(|e| IpcError::io(&e.to_string()))?;

    let temp_path = temp_file.path().to_path_buf();
    fs::rename(&temp_path, manifest_path).map_err(|e| IpcError::io(&e.to_string()))?;

    let dir_file = File::open(parent).map_err(|e| IpcError::io(&e.to_string()))?;
    dir_file
        .sync_all()
        .map_err(|e| IpcError::io(&e.to_string()))?;

    Ok(())
}

fn quarantine_path(
    data_dir: &Path,
    manifest_id: &str,
    root_idx: usize,
    rel_path: &Path,
) -> PathBuf {
    data_dir
        .join("dedup")
        .join(manifest_id)
        .join(format!("root-{}", root_idx))
        .join(rel_path)
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
    scan_job_id: String,
    confirmed: bool,
    handle: &JobHandle,
    store: &ScanResultStore,
    data_dir: &Path,
    cancel: CancellationToken,
) -> Result<(), IpcError> {
    if !confirmed {
        return Err(IpcError::validation("Move not confirmed"));
    }

    let session = store
        .get_session(&scan_job_id)
        .ok_or_else(|| IpcError::validation("Scan results not found"))?;

    let manifest_id = Uuid::new_v4().to_string();
    let mut ops = Vec::new();

    for group in &session.groups {
        let keep_path = PathBuf::from(&group.keep.path);
        let keep_hash = compute_file_hash(&keep_path)?;
        let keep_hash_hex = hex::encode(keep_hash);

        if keep_hash_hex != group.hash {
            return Err(IpcError::validation("Retained file modified since scan"));
        }

        for dup in &group.duplicates {
            let dup_path = PathBuf::from(&dup.path);
            let mut root_idx = 0;
            let mut rel_path = dup_path.clone();
            for (i, root) in session.canonical_roots.iter().enumerate() {
                if let Ok(rel) = dup_path.strip_prefix(root) {
                    root_idx = i;
                    rel_path = rel.to_path_buf();
                    break;
                }
            }

            let dest = quarantine_path(data_dir, &manifest_id, root_idx, &rel_path);
            ops.push(ManifestEntry {
                op_id: Uuid::new_v4().to_string(),
                source: dup.path.clone(),
                dest: dest.to_string_lossy().to_string(),
                expected_size: dup.size,
                expected_hash: group.hash.clone(),
                status: OperationStatus::Pending,
            });
        }
    }

    let manifest = Manifest {
        version: 1,
        manifest_id: manifest_id.clone(),
        scan_job_id: scan_job_id.clone(),
        created_at: chrono::Utc::now().to_rfc3339(),
        ops: ops.clone(),
    };

    let manifest_path = data_dir.join("dedup").join(format!("{}.json", manifest_id));
    write_manifest_atomic(&manifest_path, &manifest)?;

    let mut current_manifest = manifest;
    let total = current_manifest.ops.len();

    for i in 0..total {
        if cancel.is_cancelled() {
            handle.finish(JobStatus::Cancelled, None);
            return Ok(());
        }

        let file_label = Path::new(&current_manifest.ops[i].source)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        handle.update_progress(JobProgress {
            current: i as u64,
            total: Some(total as u64),
            message: Some(format!("Moving {}", file_label)),
        });

        let source_path = PathBuf::from(&current_manifest.ops[i].source);
        let dest_path = PathBuf::from(&current_manifest.ops[i].dest);
        let expected_hash_str = current_manifest.ops[i].expected_hash.clone();
        let expected_size = current_manifest.ops[i].expected_size;

        let source_hash = match compute_file_hash(&source_path) {
            Ok(h) => hex::encode(h),
            Err(_) => {
                current_manifest.ops[i].status =
                    OperationStatus::Failed("Source missing or unreadable".to_string());
                write_manifest_atomic(&manifest_path, &current_manifest)?;
                continue;
            }
        };

        if source_hash != expected_hash_str {
            current_manifest.ops[i].status = OperationStatus::Failed("Source modified".to_string());
            write_manifest_atomic(&manifest_path, &current_manifest)?;
            continue;
        }

        let dest = Destination {
            final_path: dest_path.clone(),
            conflict_mode: crate::domain::conflict::ConflictMode::Skip,
            expected_size: Some(expected_size),
            verify: VerifyMode::SizeAndHash,
        };

        let expected_hash_bytes =
            hex::decode(&expected_hash_str).map_err(|_| IpcError::validation("Invalid hash"))?;
        let mut hash_arr = [0u8; 64];
        hash_arr.copy_from_slice(&expected_hash_bytes);

        match move_verified(source_path, dest, hash_arr, cancel.clone()).await {
            Ok(_) => {
                current_manifest.ops[i].status = OperationStatus::Completed;
            }
            Err(e) => {
                current_manifest.ops[i].status = OperationStatus::Failed(e.to_string());
            }
        }

        write_manifest_atomic(&manifest_path, &current_manifest)?;
    }

    store.remove(&scan_job_id);
    handle.finish(JobStatus::Completed, None);
    Ok(())
}

pub async fn run_undo(
    manifest_id: String,
    handle: &JobHandle,
    data_dir: &Path,
    cancel: CancellationToken,
) -> Result<(), IpcError> {
    let manifest_path = data_dir.join("dedup").join(format!("{}.json", manifest_id));
    let content = fs::read_to_string(&manifest_path).map_err(|e| IpcError::io(&e.to_string()))?;
    let mut manifest: Manifest =
        serde_json::from_str(&content).map_err(|e| IpcError::validation(&e.to_string()))?;

    let mut indices_to_undo: Vec<usize> = manifest
        .ops
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op.status, OperationStatus::Completed))
        .map(|(i, _)| i)
        .collect();

    indices_to_undo.reverse();
    let total = indices_to_undo.len();

    for (progress_idx, &op_idx) in indices_to_undo.iter().enumerate() {
        if cancel.is_cancelled() {
            handle.finish(JobStatus::Cancelled, None);
            return Ok(());
        }

        let file_label = Path::new(&manifest.ops[op_idx].source)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        handle.update_progress(JobProgress {
            current: progress_idx as u64,
            total: Some(total as u64),
            message: Some(format!("Restoring {}", file_label)),
        });

        let source_path = PathBuf::from(&manifest.ops[op_idx].source);
        let dest_path = PathBuf::from(&manifest.ops[op_idx].dest);
        let expected_hash_str = manifest.ops[op_idx].expected_hash.clone();
        let expected_size = manifest.ops[op_idx].expected_size;

        if source_path.exists() {
            manifest.ops[op_idx].status =
                OperationStatus::UndoBlocked("Original path occupied".to_string());
            write_manifest_atomic(&manifest_path, &manifest)?;
            continue;
        }

        let quarantine_hash = match compute_file_hash(&dest_path) {
            Ok(h) => hex::encode(h),
            Err(_) => {
                manifest.ops[op_idx].status =
                    OperationStatus::Failed("Quarantine file missing".to_string());
                write_manifest_atomic(&manifest_path, &manifest)?;
                continue;
            }
        };

        if quarantine_hash != expected_hash_str {
            manifest.ops[op_idx].status =
                OperationStatus::UndoBlocked("Quarantine file modified".to_string());
            write_manifest_atomic(&manifest_path, &manifest)?;
            continue;
        }

        manifest.ops[op_idx].status = OperationStatus::UndoPending;
        write_manifest_atomic(&manifest_path, &manifest)?;

        let dest = Destination {
            final_path: source_path.clone(),
            conflict_mode: crate::domain::conflict::ConflictMode::Skip,
            expected_size: Some(expected_size),
            verify: VerifyMode::SizeAndHash,
        };

        let expected_hash_bytes =
            hex::decode(&expected_hash_str).map_err(|_| IpcError::validation("Invalid hash"))?;
        let mut hash_arr = [0u8; 64];
        hash_arr.copy_from_slice(&expected_hash_bytes);

        match move_verified(dest_path, dest, hash_arr, cancel.clone()).await {
            Ok(_) => {
                manifest.ops[op_idx].status = OperationStatus::Undone;
            }
            Err(e) => {
                manifest.ops[op_idx].status = OperationStatus::Failed(e.to_string());
            }
        }

        write_manifest_atomic(&manifest_path, &manifest)?;
    }

    handle.finish(JobStatus::Completed, None);
    Ok(())
}

pub fn reconcile_manifests_on_startup(data_dir: &Path) -> Result<(), IpcError> {
    let dedup_dir = data_dir.join("dedup");
    if !dedup_dir.exists() {
        return Ok(());
    }

    let corrupt_dir = dedup_dir.join(".corrupt");

    for entry in fs::read_dir(&dedup_dir).map_err(|e| IpcError::io(&e.to_string()))? {
        let entry = entry.map_err(|e| IpcError::io(&e.to_string()))?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut manifest: Manifest = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(_) => {
                fs::create_dir_all(&corrupt_dir).ok();
                let dest = corrupt_dir.join(path.file_name().unwrap_or_default());
                fs::rename(&path, &dest).ok();
                continue;
            }
        };

        let mut changed = false;

        for op in &mut manifest.ops {
            match op.status {
                OperationStatus::Pending => {
                    let source_exists = Path::new(&op.source).exists();
                    let dest_exists = Path::new(&op.dest).exists();

                    if source_exists && !dest_exists {
                        op.status = OperationStatus::Failed("Not moved".to_string());
                        changed = true;
                    } else if !source_exists && dest_exists {
                        if let Ok(hash) = compute_file_hash(Path::new(&op.dest)) {
                            if hex::encode(hash) == op.expected_hash {
                                op.status = OperationStatus::Completed;
                                changed = true;
                            } else {
                                op.status = OperationStatus::Ambiguous;
                                changed = true;
                            }
                        } else {
                            op.status = OperationStatus::Ambiguous;
                            changed = true;
                        }
                    } else if source_exists && dest_exists {
                        op.status = OperationStatus::Ambiguous;
                        changed = true;
                    } else if !source_exists && !dest_exists {
                        op.status = OperationStatus::Lost;
                        changed = true;
                    }
                }
                OperationStatus::UndoPending => {
                    let source_exists = Path::new(&op.source).exists();
                    let dest_exists = Path::new(&op.dest).exists();

                    if !dest_exists && source_exists {
                        op.status = OperationStatus::Undone;
                        changed = true;
                    } else if dest_exists && source_exists {
                        op.status = OperationStatus::Ambiguous;
                        changed = true;
                    } else if !dest_exists && !source_exists {
                        op.status = OperationStatus::Lost;
                        changed = true;
                    }
                }
                _ => {}
            }
        }

        if changed {
            write_manifest_atomic(&path, &manifest)?;
        }
    }

    Ok(())
}
