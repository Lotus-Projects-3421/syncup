//! Phase 7 organizer service — oracle-parity implementation.
//!
//! Behaviour preserved from `core/takeout_engine.py` and `core/naming.py`:
//! sidecar candidates `<name>.json` / `<name>.supplemental-metadata.json`,
//! `photoTakenTime` then `creationTime` timestamps, date precedence
//! (Takeout: JSON > filename > EXIF > mtime; phone: filename > EXIF > mtime),
//! idempotent skip on same size + mtime, protected same-run targets, sidecar
//! preservation, hidden/partial-file pruning and nested ZIP discovery.
//!
//! Documented deviations: hash-fallback idempotency (same size + same content
//! hash also skips, so a published file whose mtime could not be set never
//! duplicates on rerun); ZIP members get a bounded 2 MiB EXIF head read in a
//! pre-pass and a fresh full stream for the copy (C7 precedence chain);
//! corrupt ZIPs fail with a clear validation error. All ZIP and filesystem
//! work runs in blocking tasks; copies use the Phase 4 verified engine.

use crate::domain::conflict::ConflictMode;
use crate::domain::constants::{is_media, MAX_ARCHIVE_UNCOMPRESSED, MAX_FILES_PER_JOB};
use crate::domain::naming::{
    build_target_path, date_with_source, exif_datetime_from_bytes, exif_supported,
    naive_from_unix_timestamp, parse_datetime_from_name, read_head, system_time_to_naive, SortMode,
    EXIF_MAX_READ,
};
use crate::ipc::error::IpcError;
use crate::services::fileops::{compute_file_hash, copy_verified, Destination, Source, VerifyMode};
use crate::services::jobs::{JobHandle, JobProgress};
use blake2::{Blake2b512, Digest};
use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, FileTimes, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tokio_util::sync::CancellationToken;
use zip::ZipArchive;

const SIDECAR_MAX_READ: usize = 1024 * 1024;

/// Test-only fault injection: fails the next mtime preservation exactly once.
/// Compiled only for unit tests; production builds never reference it.
#[cfg(test)]
static SIMULATE_MTIME_FAIL_ONCE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Arm the one-shot mtime failure hook (unit tests only).
#[cfg(test)]
pub(crate) fn arm_mtime_failure_once() {
    SIMULATE_MTIME_FAIL_ONCE.store(true, std::sync::atomic::Ordering::SeqCst);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SourceType {
    Takeout,
    PhoneFolder,
}

fn default_sort_mode() -> SortMode {
    SortMode::Year
}

#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OrganizerConfig {
    pub inputs: Vec<PathBuf>,
    pub dest_dir: PathBuf,
    pub conflict_mode: ConflictMode,
    pub source_type: SourceType,
    #[serde(default = "default_sort_mode")]
    pub sort_mode: SortMode,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct OrganizerOutcome {
    pub processed: usize,
    pub skipped: usize,
    pub errors: usize,
}

enum FileOutcome {
    Processed,
    Skipped,
    SidecarError,
}

#[derive(Clone, Copy)]
enum TargetAction {
    Skip,
    RepairMtime,
    Copy(ConflictMode),
}

#[derive(Clone)]
struct ProtectedTargets(Arc<Mutex<HashSet<PathBuf>>>);

impl ProtectedTargets {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(HashSet::new())))
    }

    fn contains(&self, path: &Path) -> bool {
        let set = self.0.lock().unwrap_or_else(|e| e.into_inner());
        set.contains(path)
    }

    fn insert(&self, path: PathBuf) {
        let mut set = self.0.lock().unwrap_or_else(|e| e.into_inner());
        set.insert(path);
    }
}

struct Discovered {
    loose: Vec<PathBuf>,
    zips: Vec<PathBuf>,
    zip_media: usize,
    scan_errors: usize,
}

struct LoosePlan {
    action: TargetAction,
    target: PathBuf,
    size: u64,
    mtime: Option<SystemTime>,
    atime: Option<SystemTime>,
    sidecar: Option<PathBuf>,
}

struct ZipMemberPlan {
    index: usize,
    name: String,
    file_name: String,
    size: u64,
    mtime: Option<SystemTime>,
    date: NaiveDateTime,
    sidecar_index: Option<usize>,
}

struct ZipJob {
    zip_path: PathBuf,
    config: OrganizerConfig,
    dest_canon: PathBuf,
    protected: ProtectedTargets,
    cancel: CancellationToken,
    handle: Option<JobHandle>,
    done: usize,
    total: usize,
}

struct ZipResult {
    outcome: OrganizerOutcome,
    done: usize,
}

pub async fn run_organizer(
    config: OrganizerConfig,
    cancel: CancellationToken,
    handle: Option<JobHandle>,
) -> Result<OrganizerOutcome, IpcError> {
    if config.inputs.is_empty() {
        return Err(IpcError::validation("No organizer inputs were provided"));
    }
    let dest_dir = config.dest_dir.clone();
    let dest_canon = tokio::task::spawn_blocking(move || -> Result<PathBuf, IpcError> {
        fs::create_dir_all(&dest_dir).map_err(|e| IpcError::io(&e.to_string()))?;
        fs::canonicalize(&dest_dir).map_err(|e| IpcError::io(&e.to_string()))
    })
    .await
    .map_err(|e| IpcError::io(&e.to_string()))??;

    let inputs = config.inputs.clone();
    let walk_dest = dest_canon.clone();
    let walk_cancel = cancel.clone();
    let discovered =
        tokio::task::spawn_blocking(move || discover_inputs(&inputs, &walk_dest, &walk_cancel))
            .await
            .map_err(|e| IpcError::io(&e.to_string()))??;

    let total = discovered.loose.len() + discovered.zip_media;
    if total > MAX_FILES_PER_JOB {
        return Err(IpcError::validation("Exceeded MAX_FILES_PER_JOB limit"));
    }
    let protected = ProtectedTargets::new();
    let mut outcome = OrganizerOutcome {
        errors: discovered.scan_errors,
        ..Default::default()
    };
    let mut done = 0usize;
    for path in &discovered.loose {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        done += 1;
        report_progress(&handle, done, total, path);
        let file_result =
            process_single_file(path, &config, &dest_canon, &protected, &cancel).await;
        match file_result {
            Ok(FileOutcome::Processed) => outcome.processed += 1,
            Ok(FileOutcome::Skipped) => outcome.skipped += 1,
            Ok(FileOutcome::SidecarError) => {
                outcome.processed += 1;
                outcome.errors += 1;
            }
            Err(e) => {
                outcome.errors += 1;
                tracing::warn!(file = %path.display(), error = %e, "organizer file failed");
            }
        }
    }
    for zip_path in &discovered.zips {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        let job = ZipJob {
            zip_path: zip_path.clone(),
            config: config.clone(),
            dest_canon: dest_canon.clone(),
            protected: protected.clone(),
            cancel: cancel.clone(),
            handle: handle.clone(),
            done,
            total,
        };
        let zip_result = process_zip(job).await?;
        outcome.processed += zip_result.outcome.processed;
        outcome.skipped += zip_result.outcome.skipped;
        outcome.errors += zip_result.outcome.errors;
        done = zip_result.done;
    }
    Ok(outcome)
}

fn report_progress(handle: &Option<JobHandle>, current: usize, total: usize, path: &Path) {
    if let Some(h) = handle {
        let name = path.file_name().and_then(|s| s.to_str());
        let name = name.unwrap_or("file").to_string();
        h.update_progress(JobProgress {
            current: current as u64,
            total: Some(total as u64),
            message: Some(name),
        });
    }
}

fn is_zip(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str());
    match ext {
        Some(e) => e.eq_ignore_ascii_case("zip"),
        None => false,
    }
}

fn is_regular(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(m) => m.is_file(),
        Err(_) => false,
    }
}

fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

fn discover_inputs(
    inputs: &[PathBuf],
    dest_canon: &Path,
    cancel: &CancellationToken,
) -> Result<Discovered, IpcError> {
    let mut loose: Vec<PathBuf> = Vec::new();
    let mut zips: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut scan_errors = 0usize;
    for input in inputs {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        let meta = fs::symlink_metadata(input)
            .map_err(|e| IpcError::validation(&format!("Input path is not accessible: {e}")))?;
        if meta.file_type().is_symlink() {
            tracing::warn!(input = %input.display(), "skipping symlink input");
            continue;
        }
        if meta.is_dir() {
            let mut stack = vec![input.clone()];
            while let Some(dir) = stack.pop() {
                if cancel.is_cancelled() {
                    return Err(IpcError::cancelled("Organizer cancelled"));
                }
                let canon = match fs::canonicalize(&dir) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(dir = %dir.display(), error = %e, "cannot resolve folder");
                        scan_errors += 1;
                        continue;
                    }
                };
                if canon == *dest_canon {
                    continue;
                }
                let entries = match fs::read_dir(&dir) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(dir = %dir.display(), error = %e, "cannot read folder");
                        scan_errors += 1;
                        continue;
                    }
                };
                for entry in entries {
                    let entry = match entry {
                        Ok(e) => e,
                        Err(_) => {
                            scan_errors += 1;
                            continue;
                        }
                    };
                    let path = entry.path();
                    let emeta = match fs::symlink_metadata(&path) {
                        Ok(m) => m,
                        Err(_) => {
                            scan_errors += 1;
                            continue;
                        }
                    };
                    if emeta.file_type().is_symlink() {
                        continue;
                    }
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if emeta.is_dir() {
                        if !name_str.starts_with('.') {
                            stack.push(path);
                        }
                        continue;
                    }
                    if !emeta.is_file() || name_str.starts_with('.') {
                        continue;
                    }
                    let canon_file = match fs::canonicalize(&path) {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(file = %path.display(), error = %e, "cannot resolve file");
                            scan_errors += 1;
                            continue;
                        }
                    };
                    if !seen.insert(canon_file) {
                        continue;
                    }
                    if is_zip(&path) {
                        zips.push(path);
                    } else if is_media(&path) {
                        loose.push(path);
                    }
                }
            }
        } else if meta.is_file() {
            let canon =
                fs::canonicalize(input).map_err(|e| IpcError::validation(&e.to_string()))?;
            if !seen.insert(canon) {
                continue;
            }
            if is_zip(input) {
                zips.push(input.clone());
            } else if is_media(input) {
                loose.push(input.clone());
            } else {
                return Err(IpcError::validation(
                    "Input file is not a ZIP or supported media file",
                ));
            }
        }
    }
    let mut zip_media = 0usize;
    for zip_path in &zips {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        let file = File::open(zip_path).map_err(|e| IpcError::io(&e.to_string()))?;
        let archive = match ZipArchive::new(BufReader::new(file)) {
            Ok(a) => a,
            Err(e) => return Err(IpcError::validation(&format!("Corrupt ZIP: {e}"))),
        };
        for i in 0..archive.len() {
            if let Some(name) = archive.name_for_index(i) {
                let normalized = name.replace('\\', "/");
                if !normalized.ends_with('/') && is_media(Path::new(&normalized)) {
                    zip_media += 1;
                }
            }
        }
    }
    Ok(Discovered {
        loose,
        zips,
        zip_media,
        scan_errors,
    })
}

fn json_date(value: &serde_json::Value) -> Option<NaiveDateTime> {
    for key in ["photoTakenTime", "creationTime"] {
        let Some(entry) = value.get(key) else {
            continue;
        };
        let Some(ts) = entry.get("timestamp") else {
            continue;
        };
        let secs: Option<i64> = if let Some(s) = ts.as_str() {
            s.parse().ok()
        } else {
            ts.as_i64()
        };
        if let Some(secs) = secs {
            if let Some(dt) = naive_from_unix_timestamp(secs) {
                return Some(dt);
            }
        }
    }
    None
}

fn find_loose_sidecar(path: &Path) -> Option<PathBuf> {
    let a = append_suffix(path, ".json");
    if is_regular(&a) {
        return Some(a);
    }
    let b = append_suffix(path, ".supplemental-metadata.json");
    if is_regular(&b) {
        return Some(b);
    }
    None
}

fn read_loose_sidecar_date(path: &Path) -> Option<NaiveDateTime> {
    let buf = read_head(path, SIDECAR_MAX_READ).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&buf).ok()?;
    json_date(&value)
}

fn resolve_target(
    target: &Path,
    expected_size: u64,
    expected_mtime: Option<SystemTime>,
    source_hash: impl FnOnce() -> Result<[u8; 64], IpcError>,
    mode: ConflictMode,
    protected: &ProtectedTargets,
) -> Result<TargetAction, IpcError> {
    if protected.contains(target) {
        return Ok(TargetAction::Copy(ConflictMode::KeepBoth));
    }
    let meta = match fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TargetAction::Copy(mode));
        }
        Err(e) => {
            return Err(IpcError::io(&format!(
                "Cannot inspect destination metadata: {e}"
            )));
        }
    };
    if !meta.is_file() {
        return Ok(TargetAction::Copy(ConflictMode::KeepBoth));
    }
    if meta.len() == expected_size {
        let dest_mtime = meta.modified().ok();
        if let (Some(dm), Some(em)) = (dest_mtime, expected_mtime) {
            if dm == em {
                return Ok(TargetAction::Skip);
            }
        }
        let dest_hash = compute_file_hash(target)?;
        let src_hash = source_hash()?;
        if dest_hash == src_hash {
            return Ok(TargetAction::RepairMtime);
        }
    }
    match mode {
        ConflictMode::Skip => Ok(TargetAction::Skip),
        ConflictMode::Overwrite => Ok(TargetAction::Copy(ConflictMode::Overwrite)),
        ConflictMode::KeepBoth => Ok(TargetAction::Copy(ConflictMode::KeepBoth)),
    }
}

fn set_file_mtime(path: &Path, modified: SystemTime, accessed: SystemTime) -> Result<(), IpcError> {
    #[cfg(test)]
    if SIMULATE_MTIME_FAIL_ONCE.swap(false, std::sync::atomic::Ordering::SeqCst) {
        return Err(IpcError::io("Simulated mtime preservation failure"));
    }
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|e| IpcError::io(&format!("Cannot open file to preserve mtime: {e}")))?;
    let times = FileTimes::new()
        .set_modified(modified)
        .set_accessed(accessed);
    file.set_times(times)
        .map_err(|e| IpcError::io(&format!("Failed to preserve modification time: {e}")))
}

fn plan_loose_file(
    source: &Path,
    config: &OrganizerConfig,
    dest_root: &Path,
    protected: &ProtectedTargets,
) -> Result<LoosePlan, IpcError> {
    let meta = fs::symlink_metadata(source).map_err(|e| IpcError::io(&e.to_string()))?;
    if !meta.is_file() {
        return Err(IpcError::validation(
            "Organizer source is not a regular file",
        ));
    }
    let size = meta.len();
    let mtime = meta.modified().ok();
    let atime = meta.accessed().ok();
    let mut sidecar = None;
    let mut date = None;
    if config.source_type == SourceType::Takeout {
        sidecar = find_loose_sidecar(source);
        if let Some(sc) = &sidecar {
            date = read_loose_sidecar_date(sc);
        }
    }
    let date = match date {
        Some(d) => d,
        None => date_with_source(source, mtime).0,
    };
    let file_name = source.file_name().and_then(|s| s.to_str());
    let file_name = file_name.unwrap_or("file");
    let target = build_target_path(file_name, date, dest_root, config.sort_mode);
    let action = resolve_target(
        &target,
        size,
        mtime,
        || compute_file_hash(source).map_err(IpcError::from),
        config.conflict_mode,
        protected,
    )?;
    Ok(LoosePlan {
        action,
        target,
        size,
        mtime,
        atime,
        sidecar,
    })
}

async fn process_single_file(
    path: &Path,
    config: &OrganizerConfig,
    dest_canon: &Path,
    protected: &ProtectedTargets,
    cancel: &CancellationToken,
) -> Result<FileOutcome, IpcError> {
    let source = path.to_path_buf();
    let dest_root = dest_canon.to_path_buf();
    let cfg = config.clone();
    let protected_clone = protected.clone();
    let plan = tokio::task::spawn_blocking(move || {
        plan_loose_file(&source, &cfg, &dest_root, &protected_clone)
    })
    .await
    .map_err(|e| IpcError::io(&e.to_string()))??;
    match plan.action {
        TargetAction::Skip => {
            protected.insert(plan.target.clone());
            Ok(FileOutcome::Skipped)
        }
        TargetAction::RepairMtime => {
            let target = plan.target.clone();
            let m = plan.mtime;
            let a = plan.atime;
            tokio::task::spawn_blocking(move || -> Result<(), IpcError> {
                if let (Some(m), Some(a)) = (m, a) {
                    set_file_mtime(&target, m, a)?;
                }
                Ok(())
            })
            .await
            .map_err(|e| IpcError::io(&e.to_string()))??;
            protected.insert(plan.target.clone());
            Ok(FileOutcome::Skipped)
        }
        TargetAction::Copy(mode) => {
            let dest = Destination {
                final_path: plan.target.clone(),
                conflict_mode: mode,
                expected_size: Some(plan.size),
                verify: VerifyMode::SizeAndHash,
            };
            let src = Source::File(path.to_path_buf());
            let published = copy_verified(src, dest, cancel.clone()).await?;
            let target = published.final_path.clone();
            let m = plan.mtime;
            let a = plan.atime;
            tokio::task::spawn_blocking(move || -> Result<(), IpcError> {
                if let (Some(m), Some(a)) = (m, a) {
                    set_file_mtime(&target, m, a)?;
                }
                Ok(())
            })
            .await
            .map_err(|e| IpcError::io(&e.to_string()))??;
            protected.insert(published.final_path.clone());
            if let Some(sc) = plan.sidecar {
                let sidecar_result = copy_loose_sidecar(&sc, &published.final_path, cancel).await;
                if let Err(e) = sidecar_result {
                    tracing::warn!(error = %e, "sidecar copy failed");
                    return Ok(FileOutcome::SidecarError);
                }
            }
            Ok(FileOutcome::Processed)
        }
    }
}

async fn copy_loose_sidecar(
    sidecar: &Path,
    media_target: &Path,
    cancel: &CancellationToken,
) -> Result<(), IpcError> {
    let target = append_suffix(media_target, ".json");
    let sc = sidecar.to_path_buf();
    let target_check = target.clone();
    let check = tokio::task::spawn_blocking(move || -> Result<(u64, bool), IpcError> {
        let meta = fs::symlink_metadata(&sc).map_err(|e| IpcError::io(&e.to_string()))?;
        let size = meta.len();
        if !is_regular(&target_check) {
            return Ok((size, false));
        }
        let tmeta =
            fs::symlink_metadata(&target_check).map_err(|e| IpcError::io(&e.to_string()))?;
        if tmeta.len() != size {
            return Ok((size, false));
        }
        let dh = compute_file_hash(&target_check)?;
        let sh = compute_file_hash(&sc)?;
        Ok((size, dh == sh))
    })
    .await
    .map_err(|e| IpcError::io(&e.to_string()))??;
    let (size, same) = check;
    if same {
        return Ok(());
    }
    let dest = Destination {
        final_path: target,
        conflict_mode: ConflictMode::KeepBoth,
        expected_size: Some(size),
        verify: VerifyMode::SizeAndHash,
    };
    let src = Source::File(sidecar.to_path_buf());
    copy_verified(src, dest, cancel.clone()).await?;
    Ok(())
}

async fn process_zip(job: ZipJob) -> Result<ZipResult, IpcError> {
    let rt = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || process_zip_blocking(job, &rt))
        .await
        .map_err(|e| IpcError::io(&e.to_string()))?
}

fn find_zip_sidecar(member_name: &str, names: &HashMap<String, usize>) -> Option<usize> {
    let a = format!("{member_name}.json");
    if let Some(i) = names.get(&a) {
        return Some(*i);
    }
    let b = format!("{member_name}.supplemental-metadata.json");
    names.get(&b).copied()
}

fn read_zip_sidecar_date(
    archive: &mut ZipArchive<BufReader<File>>,
    index: usize,
) -> Option<NaiveDateTime> {
    let mut entry = archive.by_index(index).ok()?;
    // Do not trust dates from sidecars that exceed the supported size limit.
    if entry.size() > SIDECAR_MAX_READ as u64 {
        return None;
    }
    let mut buf = Vec::new();
    let mut limited = entry.by_ref().take(SIDECAR_MAX_READ as u64);
    let read_result = limited.read_to_end(&mut buf);
    read_result.ok()?;
    let value: serde_json::Value = serde_json::from_slice(&buf).ok()?;
    json_date(&value)
}

fn read_zip_head(
    archive: &mut ZipArchive<BufReader<File>>,
    index: usize,
    limit: usize,
) -> Result<Vec<u8>, IpcError> {
    let mut entry = match archive.by_index(index) {
        Ok(e) => e,
        Err(e) => return Err(IpcError::validation(&e.to_string())),
    };
    let mut buf = Vec::new();
    let mut limited = entry.by_ref().take(limit as u64);
    let read_result = limited.read_to_end(&mut buf);
    read_result.map_err(|e| IpcError::io(&e.to_string()))?;
    Ok(buf)
}

fn hash_zip_member(
    archive: &mut ZipArchive<BufReader<File>>,
    index: usize,
) -> Result<[u8; 64], IpcError> {
    let mut entry = match archive.by_index(index) {
        Ok(e) => e,
        Err(e) => return Err(IpcError::validation(&e.to_string())),
    };
    let mut hasher = Blake2b512::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let read_result = entry.read(&mut buf);
        let n = read_result.map_err(|e| IpcError::io(&e.to_string()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// Extract a ZIP member to an owned temporary file so it can be passed to
/// `Source::File`, whose reader must not borrow the ZipArchive. The member is
/// copied in bounded chunks, with cancellation checks and a size-integrity
/// check. The returned NamedTempFile owns cleanup on every exit path.
fn extract_zip_member_to_temp(
    archive: &mut ZipArchive<BufReader<File>>,
    index: usize,
    expected_size: u64,
    cancel: &CancellationToken,
) -> Result<tempfile::NamedTempFile, IpcError> {
    let mut entry = match archive.by_index(index) {
        Ok(e) => e,
        Err(e) => return Err(IpcError::validation(&e.to_string())),
    };
    if entry.size() != expected_size {
        return Err(IpcError::validation(
            "ZIP member size changed between planning and extraction",
        ));
    }

    let mut temp = tempfile::NamedTempFile::new()
        .map_err(|e| IpcError::io(&format!("Cannot create ZIP staging file: {e}")))?;
    let mut copied = 0u64;
    let mut buf = [0u8; 64 * 1024];

    loop {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        let n = entry
            .read(&mut buf)
            .map_err(|e| IpcError::io(&format!("Cannot read ZIP member: {e}")))?;
        if n == 0 {
            break;
        }
        copied = copied
            .checked_add(n as u64)
            .ok_or_else(|| IpcError::validation("ZIP member size overflow"))?;
        if copied > expected_size {
            return Err(IpcError::validation(
                "ZIP member exceeded its declared uncompressed size",
            ));
        }
        temp.as_file_mut()
            .write_all(&buf[..n])
            .map_err(|e| IpcError::io(&format!("Cannot write ZIP staging file: {e}")))?;
    }

    if copied != expected_size {
        return Err(IpcError::validation(
            "ZIP member size did not match its declared uncompressed size",
        ));
    }
    temp.as_file_mut()
        .flush()
        .map_err(|e| IpcError::io(&format!("Cannot flush ZIP staging file: {e}")))?;
    Ok(temp)
}

fn zip_datetime_to_system_time(dt: Option<zip::DateTime>) -> Option<SystemTime> {
    let dt = dt?;
    let date = NaiveDate::from_ymd_opt(
        i32::from(dt.year()),
        u32::from(dt.month()),
        u32::from(dt.day()),
    )?;
    let time = NaiveTime::from_hms_opt(
        u32::from(dt.hour()),
        u32::from(dt.minute()),
        u32::from(dt.second()),
    )?;
    let naive = NaiveDateTime::new(date, time);
    let local = Local.from_local_datetime(&naive).single()?;
    Some(SystemTime::from(local))
}

fn copy_zip_sidecar(
    archive: &mut ZipArchive<BufReader<File>>,
    sidecar_index: usize,
    media_target: &Path,
    rt: &tokio::runtime::Handle,
    cancel: &CancellationToken,
) -> Result<(), IpcError> {
    let target = append_suffix(media_target, ".json");
    let mut entry = match archive.by_index(sidecar_index) {
        Ok(e) => e,
        Err(e) => return Err(IpcError::validation(&e.to_string())),
    };
    // Refuse oversized sidecars rather than silently publishing truncated JSON.
    let declared_size = entry.size();
    if declared_size > SIDECAR_MAX_READ as u64 {
        return Err(IpcError::validation(&format!(
            "ZIP sidecar exceeds the {} byte limit; refusing to copy truncated metadata",
            SIDECAR_MAX_READ
        )));
    }

    let mut buf = Vec::new();
    let read_result = entry.read_to_end(&mut buf);
    read_result.map_err(|e| IpcError::io(&e.to_string()))?;
    let size = buf.len() as u64;

    // Protect against an unexpected mismatch between the ZIP size and bytes read.
    if size != declared_size {
        return Err(IpcError::io(
            "ZIP sidecar size did not match its declared uncompressed size",
        ));
    }
    if is_regular(&target) {
        let tmeta = fs::symlink_metadata(&target).map_err(|e| IpcError::io(&e.to_string()))?;
        if tmeta.len() == size {
            let dest_hash = compute_file_hash(&target)?;
            let mut hasher = Blake2b512::new();
            hasher.update(&buf);
            let src_hash: [u8; 64] = hasher.finalize().into();
            if dest_hash == src_hash {
                return Ok(());
            }
        }
    }
    let source = Source::Stream {
        reader: Box::new(std::io::Cursor::new(buf)),
        size_hint: Some(size),
    };
    let dest = Destination {
        final_path: target,
        conflict_mode: ConflictMode::KeepBoth,
        expected_size: Some(size),
        verify: VerifyMode::SizeAndHash,
    };
    rt.block_on(copy_verified(source, dest, cancel.clone()))?;
    Ok(())
}

fn process_zip_blocking(job: ZipJob, rt: &tokio::runtime::Handle) -> Result<ZipResult, IpcError> {
    let file = File::open(&job.zip_path).map_err(|e| IpcError::io(&e.to_string()))?;
    let reader = BufReader::new(file);
    let mut archive = match ZipArchive::new(reader) {
        Ok(a) => a,
        Err(e) => return Err(IpcError::validation(&format!("Corrupt ZIP: {e}"))),
    };
    let mut name_index: HashMap<String, usize> = HashMap::new();
    for i in 0..archive.len() {
        if let Some(name) = archive.name_for_index(i) {
            name_index.insert(name.to_string(), i);
        }
    }
    let mut plans: Vec<ZipMemberPlan> = Vec::new();
    let mut total_uncompressed = 0u64;
    for i in 0..archive.len() {
        if job.cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        let (name, size, is_dir, modified) = {
            let entry = match archive.by_index(i) {
                Ok(e) => e,
                Err(e) => return Err(IpcError::validation(&e.to_string())),
            };
            (
                entry.name().to_string(),
                entry.size(),
                entry.is_dir(),
                entry.last_modified(),
            )
        };
        if is_dir {
            continue;
        }
        let normalized = name.replace('\\', "/");
        let member_path = Path::new(&normalized);
        if !is_media(member_path) {
            continue;
        }
        total_uncompressed += size;
        if total_uncompressed > MAX_ARCHIVE_UNCOMPRESSED {
            return Err(IpcError::validation(
                "Archive exceeds MAX_ARCHIVE_UNCOMPRESSED limit",
            ));
        }
        if plans.len() >= MAX_FILES_PER_JOB {
            return Err(IpcError::validation(
                "Archive exceeds MAX_FILES_PER_JOB limit",
            ));
        }
        let mtime = zip_datetime_to_system_time(modified);
        let sidecar_index = find_zip_sidecar(&name, &name_index);
        let mut date = None;
        if job.config.source_type == SourceType::Takeout {
            if let Some(si) = sidecar_index {
                date = read_zip_sidecar_date(&mut archive, si);
            }
        }
        if date.is_none() {
            let stem = member_path.file_stem().and_then(|s| s.to_str());
            let stem = stem.unwrap_or("");
            date = parse_datetime_from_name(stem);
        }
        if date.is_none() && exif_supported(member_path) {
            let head = read_zip_head(&mut archive, i, EXIF_MAX_READ)?;
            date = exif_datetime_from_bytes(&head);
        }
        let date = date.unwrap_or_else(|| system_time_to_naive(mtime));
        let file_name = member_path.file_name().and_then(|s| s.to_str());
        let file_name = file_name.unwrap_or("file").to_string();
        plans.push(ZipMemberPlan {
            index: i,
            name: name.clone(),
            file_name,
            size,
            mtime,
            date,
            sidecar_index,
        });
    }
    let mut outcome = OrganizerOutcome::default();
    let mut done = job.done;
    for plan in &plans {
        if job.cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        done += 1;
        report_progress(&job.handle, done, job.total, Path::new(&plan.name));
        let target = build_target_path(
            &plan.file_name,
            plan.date,
            &job.dest_canon,
            job.config.sort_mode,
        );
        let action = resolve_target(
            &target,
            plan.size,
            plan.mtime,
            || hash_zip_member(&mut archive, plan.index),
            job.config.conflict_mode,
            &job.protected,
        )?;
        match action {
            TargetAction::Skip => {
                outcome.skipped += 1;
                job.protected.insert(target.clone());
            }
            TargetAction::RepairMtime => {
                if let Some(m) = plan.mtime {
                    set_file_mtime(&target, m, m)?;
                }
                outcome.skipped += 1;
                job.protected.insert(target.clone());
            }
            TargetAction::Copy(mode) => {
                // ZipFile borrows the archive and cannot outlive this scope.
                // Stage it to an owned temp file, then let the verified-copy
                // engine open that file independently.
                let temp =
                    extract_zip_member_to_temp(&mut archive, plan.index, plan.size, &job.cancel)?;
                let source = Source::File(temp.path().to_path_buf());
                let dest = Destination {
                    final_path: target.clone(),
                    conflict_mode: mode,
                    expected_size: Some(plan.size),
                    verify: VerifyMode::SizeAndHash,
                };
                let published = rt.block_on(copy_verified(source, dest, job.cancel.clone()))?;
                drop(temp);
                if let Some(m) = plan.mtime {
                    set_file_mtime(&published.final_path, m, m)?;
                }
                job.protected.insert(published.final_path.clone());
                outcome.processed += 1;
                if job.config.source_type == SourceType::Takeout {
                    if let Some(si) = plan.sidecar_index {
                        let sidecar_result = copy_zip_sidecar(
                            &mut archive,
                            si,
                            &published.final_path,
                            rt,
                            &job.cancel,
                        );
                        if let Err(e) = sidecar_result {
                            tracing::warn!(error = %e, "zip sidecar copy failed");
                            outcome.errors += 1;
                        }
                    }
                }
            }
        }
    }
    Ok(ZipResult { outcome, done })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::fileops::reset_collision_tracker;
    use std::io::Write;

    static ORGANIZER_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

    fn test_config(
        inputs: Vec<PathBuf>,
        dest: &Path,
        mode: ConflictMode,
        source_type: SourceType,
    ) -> OrganizerConfig {
        OrganizerConfig {
            inputs,
            dest_dir: dest.to_path_buf(),
            conflict_mode: mode,
            source_type,
            sort_mode: SortMode::Flat,
        }
    }

    fn set_mtime(path: &Path, time: SystemTime) -> Result<(), Box<dyn std::error::Error>> {
        let file = OpenOptions::new().write(true).open(path)?;
        file.set_times(FileTimes::new().set_modified(time).set_accessed(time))?;
        Ok(())
    }

    fn count_files(root: &Path) -> Result<usize, Box<dyn std::error::Error>> {
        let mut count = 0;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    fn exif_jpeg(datetime: &str) -> Vec<u8> {
        let text_len = (datetime.len() + 1) as u32;
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x8769u16.to_le_bytes());
        tiff.extend_from_slice(&4u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&26u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x9003u16.to_le_bytes());
        tiff.extend_from_slice(&2u16.to_le_bytes());
        tiff.extend_from_slice(&text_len.to_le_bytes());
        tiff.extend_from_slice(&44u32.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        tiff.extend_from_slice(datetime.as_bytes());
        tiff.push(0);
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg.extend_from_slice(&[0xFF, 0xE1]);
        let seg_len = (2 + 6 + tiff.len()) as u16;
        jpeg.extend_from_slice(&seg_len.to_be_bytes());
        jpeg.extend_from_slice(b"Exif\x00\x00");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        jpeg
    }

    fn stored_zip_options() -> zip::write::FileOptions<'static, ()> {
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn takeout_json_precedence() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let media = src.path().join("IMG_20200101_000000.jpg");
        fs::write(&media, b"jpeg-bytes-json")?;
        let sidecar = src.path().join("IMG_20200101_000000.jpg.json");
        fs::write(
            &sidecar,
            br#"{"photoTakenTime":{"timestamp":"1551500706"}}"#,
        )?;
        let config = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        let expected_date = naive_from_unix_timestamp(1551500706).ok_or("timestamp")?;
        let expected = build_target_path(
            "IMG_20200101_000000.jpg",
            expected_date,
            dest.path(),
            SortMode::Flat,
        );
        assert!(expected.is_file());
        assert_eq!(fs::read(&expected)?, b"jpeg-bytes-json");
        let filename_date = parse_datetime_from_name("IMG_20200101_000000").ok_or("name")?;
        let wrong = build_target_path(
            "IMG_20200101_000000.jpg",
            filename_date,
            dest.path(),
            SortMode::Flat,
        );
        assert!(!wrong.exists());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn exif_precedence() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let media = src.path().join("photo.jpg");
        fs::write(&media, exif_jpeg("2021:05:04 03:02:01"))?;
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(946_684_800);
        set_mtime(&media, old)?;
        let config = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::PhoneFolder,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        let expected = dest.path().join("2021-05-04_03-02-01_photo.jpg");
        assert!(expected.is_file());
        Ok(())
    }

    #[test]
    fn filename_fallback() -> Result<(), Box<dyn std::error::Error>> {
        let d = parse_datetime_from_name("IMG_20230101_120000").ok_or("date")?;
        let expected = NaiveDate::from_ymd_opt(2023, 1, 1)
            .ok_or("d")?
            .and_hms_opt(12, 0, 0)
            .ok_or("t")?;
        assert_eq!(d, expected);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mtime_fallback() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let media = src.path().join("plainphoto.jpg");
        fs::write(&media, b"not-exif")?;
        let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(946_684_800);
        set_mtime(&media, old)?;
        let config = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::PhoneFolder,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        let expected_date = system_time_to_naive(Some(old));
        let expected =
            build_target_path("plainphoto.jpg", expected_date, dest.path(), SortMode::Flat);
        assert!(expected.is_file());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zip_nested_folders() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let zip_path = src.path().join("takeout.zip");
        let file = File::create(&zip_path)?;
        let mut writer = zip::ZipWriter::new(file);
        writer.start_file(
            "Takeout/Photos/2018/IMG_20180102_030405.jpg",
            stored_zip_options(),
        )?;
        writer.write_all(b"nested-photo")?;
        writer.finish()?;
        let config = test_config(
            vec![zip_path],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        let expected = dest
            .path()
            .join("2018-01-02_03-04-05_IMG_20180102_030405.jpg");
        assert_eq!(fs::read(&expected)?, b"nested-photo");
        assert_eq!(count_files(dest.path())?, 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zip_sidecar_parsed() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let zip_path = src.path().join("sidecar.zip");
        let file = File::create(&zip_path)?;
        let mut writer = zip::ZipWriter::new(file);
        writer.start_file("IMG_0001.jpg", stored_zip_options())?;
        writer.write_all(b"sidecar-photo")?;
        writer.start_file("IMG_0001.jpg.json", stored_zip_options())?;
        writer.write_all(br#"{"photoTakenTime":{"timestamp":"1551500706"}}"#)?;
        writer.finish()?;
        let config = test_config(
            vec![zip_path],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        let expected_date = naive_from_unix_timestamp(1551500706).ok_or("timestamp")?;
        let expected =
            build_target_path("IMG_0001.jpg", expected_date, dest.path(), SortMode::Flat);
        assert!(expected.is_file());
        let copied_sidecar = append_suffix(&expected, ".json");
        assert!(copied_sidecar.is_file());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn corrupt_zip_handled() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let bad = src.path().join("bad.zip");
        fs::write(&bad, b"this is not a zip archive")?;
        let config = test_config(
            vec![bad],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );
        let result = run_organizer(config, CancellationToken::new(), None).await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.code, crate::ipc::error::VALIDATION_ERROR);
        }
        assert_eq!(count_files(dest.path())?, 0);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn idempotent_rerun_no_new_files() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        fs::write(src.path().join("IMG_20200202_020202.jpg"), b"loose-photo")?;
        let zip_path = src.path().join("bundle.zip");
        let file = File::create(&zip_path)?;
        let mut writer = zip::ZipWriter::new(file);
        writer.start_file("IMG_20210303_030303.jpg", stored_zip_options())?;
        writer.write_all(b"zipped-photo")?;
        writer.finish()?;
        let inputs = vec![src.path().to_path_buf()];
        let config1 = test_config(
            inputs.clone(),
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );
        let first = run_organizer(config1, CancellationToken::new(), None).await?;
        assert_eq!(first.processed, 2);
        let count1 = count_files(dest.path())?;
        reset_collision_tracker();
        let config2 = test_config(
            inputs,
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );
        let second = run_organizer(config2, CancellationToken::new(), None).await?;
        assert_eq!(second.processed, 0);
        assert_eq!(second.skipped, 2);
        assert_eq!(count_files(dest.path())?, count1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn collision_kept_both() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let media = src.path().join("IMG_20230101_120000.jpg");
        fs::write(&media, b"new-content-different")?;
        let target = dest
            .path()
            .join("2023-01-01_12-00-00_IMG_20230101_120000.jpg");
        fs::write(&target, b"old")?;
        let config = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::PhoneFolder,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        assert_eq!(fs::read(&target)?, b"old");
        let suffixed = dest
            .path()
            .join("2023-01-01_12-00-00_IMG_20230101_120000 (1).jpg");
        assert_eq!(fs::read(&suffixed)?, b"new-content-different");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn collision_overwrite() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let media = src.path().join("IMG_20230101_120000.jpg");
        fs::write(&media, b"new-content-different")?;
        let target = dest
            .path()
            .join("2023-01-01_12-00-00_IMG_20230101_120000.jpg");
        fs::write(&target, b"old")?;
        let config = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::Overwrite,
            SourceType::PhoneFolder,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        assert_eq!(fs::read(&target)?, b"new-content-different");
        let suffixed = dest
            .path()
            .join("2023-01-01_12-00-00_IMG_20230101_120000 (1).jpg");
        assert!(!suffixed.exists());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn collision_skip() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let media = src.path().join("IMG_20230101_120000.jpg");
        fs::write(&media, b"new-content-different")?;
        let target = dest
            .path()
            .join("2023-01-01_12-00-00_IMG_20230101_120000.jpg");
        fs::write(&target, b"old")?;
        let config = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::Skip,
            SourceType::PhoneFolder,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 0);
        assert_eq!(outcome.skipped, 1);
        assert_eq!(fs::read(&target)?, b"old");
        assert_eq!(count_files(dest.path())?, 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn mtime_failure_no_duplicate_on_rerun() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        SIMULATE_MTIME_FAIL_ONCE.store(false, std::sync::atomic::Ordering::SeqCst);
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let media = src.path().join("IMG_20220404_040404.jpg");
        fs::write(&media, b"precious-photo")?;
        let config1 = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::PhoneFolder,
        );
        arm_mtime_failure_once();
        let first = run_organizer(config1, CancellationToken::new(), None).await?;
        assert_eq!(first.processed, 0);
        assert_eq!(first.errors, 1);
        let target = dest
            .path()
            .join("2022-04-04_04-04-04_IMG_20220404_040404.jpg");
        assert!(target.is_file());
        assert_eq!(fs::read(&target)?, b"precious-photo");
        assert!(!SIMULATE_MTIME_FAIL_ONCE.load(std::sync::atomic::Ordering::SeqCst));
        reset_collision_tracker();
        let config2 = test_config(
            vec![src.path().to_path_buf()],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::PhoneFolder,
        );
        let second = run_organizer(config2, CancellationToken::new(), None).await?;
        assert_eq!(second.processed, 0);
        assert_eq!(second.skipped, 1);
        assert_eq!(second.errors, 0);
        assert_eq!(count_files(dest.path())?, 1);
        assert_eq!(fs::read(&target)?, b"precious-photo");
        let suffixed = dest
            .path()
            .join("2022-04-04_04-04-04_IMG_20220404_040404 (1).jpg");
        assert!(!suffixed.exists());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn oversized_zip_sidecar_is_rejected_without_truncation(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();

        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let zip_path = src.path().join("oversized-sidecar.zip");

        let file = File::create(&zip_path)?;
        let mut writer = zip::ZipWriter::new(file);

        writer.start_file("IMG_20200101_000000.jpg", stored_zip_options())?;
        writer.write_all(b"oversized-sidecar-photo")?;

        writer.start_file("IMG_20200101_000000.jpg.json", stored_zip_options())?;

        // Keep valid JSON at the beginning, then exceed the copy limit.
        let mut oversized_sidecar = br#"{"photoTakenTime":{"timestamp":"1551500706"}}"#.to_vec();
        oversized_sidecar.resize(SIDECAR_MAX_READ + 1, b' ');
        writer.write_all(&oversized_sidecar)?;

        writer.finish()?;

        let config = test_config(
            vec![zip_path],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );

        let outcome = run_organizer(config, CancellationToken::new(), None).await?;

        // The media file is processed, but its oversized sidecar is rejected.
        assert_eq!(outcome.processed, 1);
        assert_eq!(outcome.errors, 1);

        // The oversized sidecar is not trusted for its date; use the filename date.
        let expected_date =
            parse_datetime_from_name("IMG_20200101_000000").ok_or("filename date")?;
        let expected = build_target_path(
            "IMG_20200101_000000.jpg",
            expected_date,
            dest.path(),
            SortMode::Flat,
        );

        assert!(expected.is_file());
        assert_eq!(fs::read(&expected)?, b"oversized-sidecar-photo");

        let copied_sidecar = append_suffix(&expected, ".json");
        assert!(
            !copied_sidecar.exists(),
            "An oversized sidecar must not be published in truncated form"
        );

        assert_eq!(count_files(dest.path())?, 1);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zip_member_bounded_read() -> Result<(), Box<dyn std::error::Error>> {
        let _guard = ORGANIZER_TEST_LOCK.lock().await;
        reset_collision_tracker();
        let src = tempfile::tempdir()?;
        let dest = tempfile::tempdir()?;
        let mut payload = exif_jpeg("2019:06:07 08:09:10");
        payload.resize(payload.len() + 3 * 1024 * 1024, 0u8);
        let zip_path = src.path().join("big.zip");
        let file = File::create(&zip_path)?;
        let mut writer = zip::ZipWriter::new(file);
        writer.start_file("photo.jpg", stored_zip_options())?;
        writer.write_all(&payload)?;
        writer.finish()?;
        let config = test_config(
            vec![zip_path],
            dest.path(),
            ConflictMode::KeepBoth,
            SourceType::Takeout,
        );
        let outcome = run_organizer(config, CancellationToken::new(), None).await?;
        assert_eq!(outcome.processed, 1);
        let target = dest.path().join("2019-06-07_08-09-10_photo.jpg");
        let copied = fs::read(&target)?;
        assert_eq!(copied.len(), payload.len());
        assert_eq!(copied, payload);
        Ok(())
    }
}
