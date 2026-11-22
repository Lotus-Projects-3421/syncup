//! Safe, atomic, and verified file-operation engine.
//!
//! # Durability and Windows Semantics
//! Standard file renames (`std::fs::rename` or `tempfile::NamedTempFile::persist()`)
//! alone are insufficient for Windows durability. On Windows NTFS and FAT32 filesystems,
//! standard rename operations do not guarantee metadata or directory entry durability
//! upon power loss without explicit write-through flags or directory flushes.
//! Furthermore, Windows lacks a direct user-space directory fsync equivalent. To ensure
//! durability on Windows, SyncUp uses `ReplaceFileW` (when replacing an existing file)
//! and falls back to `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`
//! when the destination does not yet exist.
//!
//! # `tempfile::NamedTempFile::persist()` Guarantees and Limitations
//! According to the `tempfile` crate documentation (see
//! `https://docs.rs/tempfile/3.27.0/tempfile/struct.NamedTempFile.html#method.persist`):
//! "If a file exists at the target path, persist will atomically replace it. If this method
//! fails, it will return `self` in the resulting `PathPersistError`. Note: Temporary files
//! cannot be persisted across filesystems. Also neither the file contents nor the containing
//! directory are synchronized, so the update may not yet have reached the disk when `persist`
//! returns."
//! SyncUp explicitly addresses these limitations by creating the temporary file in the target's
//! parent directory (ensuring same-filesystem rename), calling `sync_all()` on the temp file
//! before persist, and issuing a directory fsync on POSIX immediately following persist.
//!
//! # Threading and Runtime Isolation
//! All blocking I/O — copy loop, fsync, BLAKE2b hashing, directory fsync, disk-space statvfs —
//! runs inside tokio::task::spawn_blocking. No blocking call executes on the async runtime thread.
//!
//! # Python Oracle Deviations
//! Deviation from Python oracle: core/fileops.py uses os.replace() + os.fsync on temp only
//! (no directory fsync, no Windows durability). v2 adds both per PLAYBOOK §6.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use blake2::{Blake2b512, Digest};
use tokio_util::sync::CancellationToken;

#[path = "../domain/conflict.rs"]
pub mod conflict;
pub use conflict::{format_collision_name, ConflictMode, RunCollisionTracker};

use crate::ipc::error::{
    IpcError, CANCELLED, DISK_FULL, IO_ERROR, PERMISSION_DENIED, PUBLISH_FAILED,
    PUBLISH_PENDING_DURABILITY, VALIDATION_ERROR, VERIFY_FAILED,
};

/// Test fault injection hooks.
pub static SIMULATED_FREE_BYTES: AtomicU64 = AtomicU64::new(u64::MAX);
pub static SIMULATE_DIR_FSYNC_FAIL: AtomicBool = AtomicBool::new(false);
pub static SIMULATE_HASH_MISMATCH: AtomicBool = AtomicBool::new(false);

static GLOBAL_PATH_LOCKS: std::sync::LazyLock<PathLockRegistry> =
    std::sync::LazyLock::new(PathLockRegistry::new);

static GLOBAL_COLLISION_TRACKER: std::sync::LazyLock<std::sync::Mutex<RunCollisionTracker>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(RunCollisionTracker::new()));

/// Resolve destination path according to ConflictMode and active run reservations.
pub fn resolve_destination_path(candidate: &Path, mode: ConflictMode) -> PathBuf {
    let mut tracker = GLOBAL_COLLISION_TRACKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tracker.resolve_path(candidate, mode)
}

/// Reset run collision tracker for deterministic test setups.
pub fn reset_collision_tracker() {
    let mut tracker = GLOBAL_COLLISION_TRACKER
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *tracker = RunCollisionTracker::new();
}

/// Registry of per-path async mutexes to serialize concurrent writers to the same final path.
#[derive(Clone, Debug, Default)]
pub struct PathLockRegistry {
    inner: Arc<std::sync::Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>>,
}

impl PathLockRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    pub async fn lock_for(&self, path: &Path) -> tokio::sync::OwnedMutexGuard<()> {
        let path_buf = path.to_path_buf();
        let mutex = {
            let mut map = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            map.entry(path_buf)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        mutex.lock_owned().await
    }
}

pub enum Source {
    File(PathBuf),
    Stream {
        reader: Box<dyn Read + Send>,
        size_hint: Option<u64>,
    },
}

impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(p) => f.debug_tuple("File").field(p).finish(),
            Self::Stream { size_hint, .. } => f
                .debug_struct("Stream")
                .field("size_hint", size_hint)
                .finish_non_exhaustive(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyMode {
    SizeOnly,
    SizeAndHash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub final_path: PathBuf,
    pub conflict_mode: ConflictMode,
    pub expected_size: Option<u64>,
    pub verify: VerifyMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub final_path: PathBuf,
    pub bytes_written: u64,
    pub hash: Option<[u8; 64]>,
}

#[derive(Debug, thiserror::Error)]
pub enum FileOpError {
    #[error("Validation failed: {reason}")]
    Validation { reason: String },
    #[error("Disk full: needed {needed} bytes, available {available} bytes")]
    DiskFull { needed: u64, available: u64 },
    #[error("Permission denied: {path:?}")]
    PermissionDenied { path: PathBuf },
    #[error("I/O error at {path:?}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Verification failed: expected {expected_size} bytes, got {actual_size} bytes")]
    VerifyFailed {
        expected_size: u64,
        actual_size: u64,
    },
    #[error("Hash mismatch: expected {expected:?}, got {actual:?}")]
    HashMismatch {
        expected: Box<[u8; 64]>,
        actual: Box<[u8; 64]>,
    },
    #[error("Publish failed at {path:?}: {source}")]
    PublishFailed {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Publish pending durability at {final_path:?}: {source}")]
    PublishPendingDurability {
        final_path: PathBuf,
        source: std::io::Error,
    },
    #[error("Operation was cancelled")]
    Cancelled,
}

impl From<FileOpError> for IpcError {
    fn from(err: FileOpError) -> Self {
        match err {
            FileOpError::Validation { reason } => IpcError::new(VALIDATION_ERROR, &reason),
            FileOpError::DiskFull { needed, available } => IpcError::new(
                DISK_FULL,
                &format!(
                    "Insufficient disk space: needed {needed} bytes, available {available} bytes"
                ),
            ),
            FileOpError::PermissionDenied { .. } => {
                IpcError::new(PERMISSION_DENIED, "Permission denied accessing file path")
            }
            FileOpError::Io { source, .. } => IpcError::new(IO_ERROR, &source.to_string()),
            FileOpError::VerifyFailed {
                expected_size,
                actual_size,
            } => IpcError::new(
                VERIFY_FAILED,
                &format!(
                    "File size verification failed: expected {expected_size}, got {actual_size}"
                ),
            ),
            FileOpError::HashMismatch { .. } => {
                IpcError::new(VERIFY_FAILED, "File content hash verification failed")
            }
            FileOpError::PublishFailed { source, .. } => {
                IpcError::new(PUBLISH_FAILED, &source.to_string())
            }
            FileOpError::PublishPendingDurability { source, .. } => {
                IpcError::new(PUBLISH_PENDING_DURABILITY, &source.to_string())
            }
            FileOpError::Cancelled => IpcError::new(CANCELLED, "File operation cancelled"),
        }
    }
}

fn is_windows_reserved(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    let base = upper.split('.').next().unwrap_or(&upper);
    matches!(
        base,
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

impl Destination {
    pub fn validate_with_root(&self, approved_root: Option<&Path>) -> Result<(), FileOpError> {
        // 1. No ".." components
        for comp in self.final_path.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(FileOpError::Validation {
                    reason: "Destination path contains '..' traversal component".to_string(),
                });
            }
        }

        // 2. No ASCII control characters
        let path_str = self.final_path.to_string_lossy();
        if path_str.chars().any(|c| c.is_ascii_control()) {
            return Err(FileOpError::Validation {
                reason: "Destination path contains ASCII control characters".to_string(),
            });
        }

        // 3. File name component check
        let file_name = match self.final_path.file_name() {
            Some(name) => name.to_string_lossy(),
            None => {
                return Err(FileOpError::Validation {
                    reason: "Destination path missing file name component".to_string(),
                });
            }
        };

        // No absolute components or separators in the file name portion
        if file_name.contains('/') || file_name.contains('\\') || file_name.contains(':') {
            return Err(FileOpError::Validation {
                reason: "Destination file name portion contains separator or absolute prefix"
                    .to_string(),
            });
        }

        // Check Windows reserved names in all components
        for comp in self.final_path.components() {
            let s = comp.as_os_str().to_string_lossy();
            if is_windows_reserved(&s) {
                return Err(FileOpError::Validation {
                    reason: format!("Destination path contains Windows reserved name: {s}"),
                });
            }
        }

        // 4. symlink_metadata on final_path's PARENT
        let parent = match self.final_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };

        if let Ok(meta) = std::fs::symlink_metadata(parent) {
            if meta.file_type().is_symlink() {
                match approved_root {
                    Some(root) => {
                        let canonical_parent =
                            std::fs::canonicalize(parent).map_err(|e| FileOpError::Io {
                                path: parent.to_path_buf(),
                                source: e,
                            })?;
                        let canonical_root =
                            std::fs::canonicalize(root).map_err(|e| FileOpError::Io {
                                path: root.to_path_buf(),
                                source: e,
                            })?;
                        if !canonical_parent.starts_with(&canonical_root) {
                            return Err(FileOpError::Validation {
                                reason:
                                    "Parent directory is a symlink pointing outside approved root"
                                        .to_string(),
                            });
                        }
                    }
                    None => {
                        return Err(FileOpError::Validation {
                            reason: "Parent directory is a symlink pointing outside approved root"
                                .to_string(),
                        });
                    }
                }
            }
        }

        Ok(())
    }

    pub fn validate(&self) -> Result<(), FileOpError> {
        self.validate_with_root(None)
    }
}

#[cfg(unix)]
fn available_space(path: &Path) -> std::io::Result<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let simulated = SIMULATED_FREE_BYTES.load(Ordering::SeqCst);
    if simulated != u64::MAX {
        return Ok(simulated);
    }

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if res != 0 {
        return Err(std::io::Error::last_os_error());
    }

    let available = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
    Ok(available)
}

#[cfg(windows)]
fn available_space(path: &Path) -> std::io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let simulated = SIMULATED_FREE_BYTES.load(Ordering::SeqCst);
    if simulated != u64::MAX {
        return Ok(simulated);
    }

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);

    let mut free_bytes_available: u64 = 0;
    let mut total_number_of_bytes: u64 = 0;
    let mut total_number_of_free_bytes: u64 = 0;

    let success = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes_available,
            &mut total_number_of_bytes,
            &mut total_number_of_free_bytes,
        )
    };

    if success == 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(free_bytes_available)
}

#[cfg(not(any(unix, windows)))]
fn available_space(_path: &Path) -> std::io::Result<u64> {
    let simulated = SIMULATED_FREE_BYTES.load(Ordering::SeqCst);
    if simulated != u64::MAX {
        return Ok(simulated);
    }
    Ok(u64::MAX)
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    if SIMULATE_DIR_FSYNC_FAIL.load(Ordering::SeqCst) {
        return Err(std::io::Error::other("Simulated directory fsync failure"));
    }

    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    let dir_file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(dir)?;
    dir_file.sync_all()
}

#[cfg(windows)]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    if SIMULATE_DIR_FSYNC_FAIL.load(Ordering::SeqCst) {
        return Err(std::io::Error::other("Simulated directory fsync failure"));
    }
    // Windows durability strategy: reliance on MOVEFILE_WRITE_THROUGH and ReplaceFileW semantics.
    // Windows lacks a direct user-space directory fsync equivalent.
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

fn compute_file_hash(path: &Path) -> Result<[u8; 64], FileOpError> {
    let mut file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
            path: path.to_path_buf(),
        },
        _ => FileOpError::Io {
            path: path.to_path_buf(),
            source: e,
        },
    })?;

    let mut hasher = Blake2b512::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| FileOpError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let res: [u8; 64] = hasher.finalize().into();
    Ok(res)
}

#[cfg(unix)]
fn publish_file(temp_file: tempfile::NamedTempFile, final_path: &Path) -> Result<(), FileOpError> {
    match temp_file.persist(final_path) {
        Ok(_) => Ok(()),
        Err(err) => Err(FileOpError::PublishFailed {
            path: final_path.to_path_buf(),
            source: err.error,
        }),
    }
}

#[cfg(windows)]
fn publish_file(temp_file: tempfile::NamedTempFile, final_path: &Path) -> Result<(), FileOpError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        REPLACEFILE_WRITE_THROUGH,
    };

    let temp_path = temp_file.path().to_path_buf();
    let mut temp_w: Vec<u16> = temp_path.as_os_str().encode_wide().collect();
    temp_w.push(0);
    let mut final_w: Vec<u16> = final_path.as_os_str().encode_wide().collect();
    final_w.push(0);

    let replaced = if final_path.exists() {
        let res = unsafe {
            ReplaceFileW(
                final_w.as_ptr(),
                temp_w.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        res != 0
    } else {
        false
    };

    if !replaced {
        let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
        let res = unsafe { MoveFileExW(temp_w.as_ptr(), final_w.as_ptr(), flags) };
        if res == 0 {
            return Err(FileOpError::PublishFailed {
                path: final_path.to_path_buf(),
                source: std::io::Error::last_os_error(),
            });
        }
    }

    let _ = temp_file.keep();
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn publish_file(temp_file: tempfile::NamedTempFile, final_path: &Path) -> Result<(), FileOpError> {
    match temp_file.persist(final_path) {
        Ok(_) => Ok(()),
        Err(err) => Err(FileOpError::PublishFailed {
            path: final_path.to_path_buf(),
            source: err.error,
        }),
    }
}

/// Atomically and safely copies/streams a source to a verified destination.
pub async fn copy_verified(
    src: Source,
    dest: Destination,
    cancel: CancellationToken,
) -> Result<Published, FileOpError> {
    copy_verified_with_root(src, dest, cancel, None).await
}

/// Atomically and safely copies/streams a source to a verified destination with approved root check.
pub async fn copy_verified_with_root(
    src: Source,
    dest: Destination,
    cancel: CancellationToken,
    approved_root: Option<&Path>,
) -> Result<Published, FileOpError> {
    // Step 1 validation
    dest.validate_with_root(approved_root)?;

    // Resolve destination path based on conflict mode and run collision tracker
    let resolved_path = resolve_destination_path(&dest.final_path, dest.conflict_mode);

    if dest.conflict_mode == ConflictMode::Skip && resolved_path.exists() {
        return Ok(Published {
            final_path: resolved_path,
            bytes_written: 0,
            hash: None,
        });
    }

    // Acquire path serialization lock before step 3
    let lock_guard = GLOBAL_PATH_LOCKS.lock_for(&resolved_path).await;

    // Run blocking file operations in spawn_blocking task
    let task_resolved_path = resolved_path.clone();
    let task = tokio::task::spawn_blocking(move || {
        let _guard = lock_guard;

        if cancel.is_cancelled() {
            return Err(FileOpError::Cancelled);
        }

        // 1. Validate source
        let (mut reader, source_size_hint): (Box<dyn Read + Send>, Option<u64>) = match src {
            Source::File(src_path) => {
                let meta = std::fs::metadata(&src_path).map_err(|e| match e.kind() {
                    std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
                        path: src_path.clone(),
                    },
                    _ => FileOpError::Io {
                        path: src_path.clone(),
                        source: e,
                    },
                })?;
                if !meta.is_file() {
                    return Err(FileOpError::Validation {
                        reason: format!("Source path is not a regular file: {src_path:?}"),
                    });
                }
                let file = std::fs::File::open(&src_path).map_err(|e| match e.kind() {
                    std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
                        path: src_path.clone(),
                    },
                    _ => FileOpError::Io {
                        path: src_path.clone(),
                        source: e,
                    },
                })?;
                (Box::new(file), Some(meta.len()))
            }
            Source::Stream { reader, size_hint } => (reader, size_hint),
        };

        let parent_dir = match task_resolved_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };

        if !parent_dir.exists() {
            std::fs::create_dir_all(parent_dir).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
                    path: parent_dir.to_path_buf(),
                },
                _ => FileOpError::Io {
                    path: parent_dir.to_path_buf(),
                    source: e,
                },
            })?;
        }

        // 2. Free-space preflight
        let needed_size = dest.expected_size.or(source_size_hint);
        if let Some(needed) = needed_size {
            let available = available_space(parent_dir).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
                    path: parent_dir.to_path_buf(),
                },
                _ => FileOpError::Io {
                    path: parent_dir.to_path_buf(),
                    source: e,
                },
            })?;
            if available < needed {
                return Err(FileOpError::DiskFull { needed, available });
            }
        }

        // 3. Create NamedTempFile in destination directory
        let mut temp_file = tempfile::Builder::new()
            .prefix(".syncup-part-")
            .tempfile_in(parent_dir)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
                    path: parent_dir.to_path_buf(),
                },
                _ => FileOpError::Io {
                    path: parent_dir.to_path_buf(),
                    source: e,
                },
            })?;

        // 4. Buffered stream copy in 64 KiB chunks
        let mut buf = [0u8; 64 * 1024];
        let mut bytes_written: u64 = 0;
        let mut stream_hasher = Blake2b512::new();

        loop {
            if cancel.is_cancelled() {
                drop(temp_file);
                return Err(FileOpError::Cancelled);
            }

            let n = reader.read(&mut buf).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
                    path: task_resolved_path.clone(),
                },
                _ => FileOpError::Io {
                    path: task_resolved_path.clone(),
                    source: e,
                },
            })?;

            if n == 0 {
                break;
            }

            temp_file.write_all(&buf[..n]).map_err(|e| match e.kind() {
                std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
                    path: temp_file.path().to_path_buf(),
                },
                _ => FileOpError::Io {
                    path: temp_file.path().to_path_buf(),
                    source: e,
                },
            })?;

            if matches!(dest.verify, VerifyMode::SizeAndHash) {
                stream_hasher.update(&buf[..n]);
            }

            bytes_written += n as u64;
        }

        // 5. flush and sync_all
        temp_file.flush().map_err(|e| FileOpError::Io {
            path: temp_file.path().to_path_buf(),
            source: e,
        })?;
        temp_file
            .as_file()
            .sync_all()
            .map_err(|e| FileOpError::Io {
                path: temp_file.path().to_path_buf(),
                source: e,
            })?;

        // 6. Verify
        if let Some(expected) = dest.expected_size {
            if bytes_written != expected {
                drop(temp_file);
                return Err(FileOpError::VerifyFailed {
                    expected_size: expected,
                    actual_size: bytes_written,
                });
            }
        }

        let mut computed_hash = None;
        if matches!(dest.verify, VerifyMode::SizeAndHash) {
            if SIMULATE_HASH_MISMATCH.load(Ordering::SeqCst) {
                drop(temp_file);
                return Err(FileOpError::HashMismatch {
                    expected: Box::new([0u8; 64]),
                    actual: Box::new([1u8; 64]),
                });
            }

            let temp_hash = compute_file_hash(temp_file.path())?;
            let expected_hash: [u8; 64] = stream_hasher.finalize().into();
            if expected_hash != temp_hash {
                drop(temp_file);
                return Err(FileOpError::HashMismatch {
                    expected: Box::new(expected_hash),
                    actual: Box::new(temp_hash),
                });
            }
            computed_hash = Some(temp_hash);
        }

        // 7. Publish
        publish_file(temp_file, &task_resolved_path)?;

        // 8. Directory fsync
        if let Err(e) = sync_dir(parent_dir) {
            // 9. If directory durability fails, report PublishPendingDurability
            return Err(FileOpError::PublishPendingDurability {
                final_path: task_resolved_path,
                source: e,
            });
        }

        // 10. Return Ok(Published)
        Ok(Published {
            final_path: task_resolved_path,
            bytes_written,
            hash: computed_hash,
        })
    });

    task.await.map_err(|e| FileOpError::Io {
        path: resolved_path,
        source: std::io::Error::other(e),
    })?
}

/// Scans destination root for orphan `.syncup-part-*` files, logs and removes them.
pub fn cleanup_orphan_temp_files(dest_root: &Path) -> Result<usize, FileOpError> {
    if !dest_root.exists() {
        return Ok(0);
    }
    let entries = std::fs::read_dir(dest_root).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => FileOpError::PermissionDenied {
            path: dest_root.to_path_buf(),
        },
        _ => FileOpError::Io {
            path: dest_root.to_path_buf(),
            source: e,
        },
    })?;

    let mut count = 0;
    for entry in entries {
        let entry = entry.map_err(|e| FileOpError::Io {
            path: dest_root.to_path_buf(),
            source: e,
        })?;
        let file_name = entry.file_name();
        let name_str = file_name.to_string_lossy();
        if name_str.starts_with(".syncup-part-") {
            let path = entry.path();
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() || meta.file_type().is_symlink() {
                    tracing::debug!(path = ?path, "cleaning orphan temporary file");
                    if let Err(e) = std::fs::remove_file(&path) {
                        tracing::warn!(path = ?path, error = %e, "failed to remove orphan temp file");
                    } else {
                        count += 1;
                    }
                }
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_destination_rejects_parent_traversal() {
        let dest = Destination {
            final_path: PathBuf::from("photos/../secret.txt"),
            conflict_mode: ConflictMode::Overwrite,
            expected_size: None,
            verify: VerifyMode::SizeOnly,
        };
        let res = dest.validate();
        assert!(matches!(res, Err(FileOpError::Validation { .. })));
    }

    #[test]
    fn validate_destination_rejects_windows_reserved() {
        for name in &[
            "con", "PRN.txt", "aux", "nul.jpg", "COM1", "com9.bin", "Lpt3",
        ] {
            let dest = Destination {
                final_path: PathBuf::from(format!("photos/{name}")),
                conflict_mode: ConflictMode::Overwrite,
                expected_size: None,
                verify: VerifyMode::SizeOnly,
            };
            let res = dest.validate();
            assert!(
                matches!(res, Err(FileOpError::Validation { .. })),
                "Expected {name} to be rejected"
            );
        }
    }

    #[test]
    fn validate_destination_rejects_control_chars() {
        let dest = Destination {
            final_path: PathBuf::from("photos/bad\x00file.jpg"),
            conflict_mode: ConflictMode::Overwrite,
            expected_size: None,
            verify: VerifyMode::SizeOnly,
        };
        let res = dest.validate();
        assert!(matches!(res, Err(FileOpError::Validation { .. })));

        let dest2 = Destination {
            final_path: PathBuf::from("photos/bad\nfile.jpg"),
            conflict_mode: ConflictMode::Overwrite,
            expected_size: None,
            verify: VerifyMode::SizeOnly,
        };
        let res2 = dest2.validate();
        assert!(matches!(res2, Err(FileOpError::Validation { .. })));
    }

    #[test]
    fn keep_both_counter_is_deterministic() {
        let mut tracker = RunCollisionTracker::new();
        let target = PathBuf::from("/photos/image.jpg");

        let p1 = tracker.resolve_path(&target, ConflictMode::KeepBoth);
        let p2 = tracker.resolve_path(&target, ConflictMode::KeepBoth);
        let p3 = tracker.resolve_path(&target, ConflictMode::KeepBoth);

        assert_eq!(p1, PathBuf::from("/photos/image.jpg"));
        assert_eq!(p2, PathBuf::from("/photos/image (1).jpg"));
        assert_eq!(p3, PathBuf::from("/photos/image (2).jpg"));
    }

    #[test]
    fn error_maps_to_ipc_codes() {
        use crate::ipc::error::*;

        let errs: [(FileOpError, &str); 9] = [
            (
                FileOpError::Validation {
                    reason: "invalid".to_string(),
                },
                VALIDATION_ERROR,
            ),
            (
                FileOpError::DiskFull {
                    needed: 100,
                    available: 50,
                },
                DISK_FULL,
            ),
            (
                FileOpError::PermissionDenied {
                    path: PathBuf::from("/restricted"),
                },
                PERMISSION_DENIED,
            ),
            (
                FileOpError::Io {
                    path: PathBuf::from("/path"),
                    source: std::io::Error::other("io error"),
                },
                IO_ERROR,
            ),
            (
                FileOpError::VerifyFailed {
                    expected_size: 100,
                    actual_size: 90,
                },
                VERIFY_FAILED,
            ),
            (
                FileOpError::HashMismatch {
                    expected: Box::new([0u8; 64]),
                    actual: Box::new([1u8; 64]),
                },
                VERIFY_FAILED,
            ),
            (
                FileOpError::PublishFailed {
                    path: PathBuf::from("/dest"),
                    source: std::io::Error::other("publish error"),
                },
                PUBLISH_FAILED,
            ),
            (
                FileOpError::PublishPendingDurability {
                    final_path: PathBuf::from("/final"),
                    source: std::io::Error::other("fsync error"),
                },
                PUBLISH_PENDING_DURABILITY,
            ),
            (FileOpError::Cancelled, CANCELLED),
        ];

        for (err, expected_code) in errs {
            let ipc: IpcError = err.into();
            assert_eq!(ipc.code, expected_code);
        }
    }
}
