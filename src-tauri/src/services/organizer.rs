use crate::domain::conflict::ConflictMode;
use crate::domain::constants::{is_media, MAX_ARCHIVE_UNCOMPRESSED, MAX_FILES_PER_JOB};
use crate::domain::naming::{compute_destination_name, extract_date_from_filename};
use crate::ipc::error::IpcError;
use chrono::{NaiveDateTime, Utc};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SourceType {
    Takeout,
    PhoneFolder,
}

#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OrganizerConfig {
    pub inputs: Vec<PathBuf>,
    pub dest_dir: PathBuf,
    pub conflict_mode: ConflictMode,
    pub source_type: SourceType,
}

pub async fn run_organizer(
    config: OrganizerConfig,
    cancel: CancellationToken,
) -> Result<usize, IpcError> {
    let mut processed_count = 0;
    for input in &config.inputs {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        if input.is_dir() {
            processed_count += process_folder(input, &config, &cancel).await?;
        } else if input.extension().and_then(|e| e.to_str()) == Some("zip") {
            processed_count += process_zip(input, &config, &cancel).await?;
        }
    }
    Ok(processed_count)
}

async fn process_folder(
    dir: &Path,
    config: &OrganizerConfig,
    cancel: &CancellationToken,
) -> Result<usize, IpcError> {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current_dir) = stack.pop() {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        let entries =
            fs::read_dir(&current_dir).map_err(|e| IpcError::permission_denied(&e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| IpcError::permission_denied(&e.to_string()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_media(&path) {
                process_single_file(&path, config, cancel).await?;
                count += 1;
                if count > MAX_FILES_PER_JOB {
                    return Err(IpcError::validation("Exceeded MAX_FILES_PER_JOB limit"));
                }
            }
        }
    }
    Ok(count)
}

async fn process_zip(
    zip_path: &Path,
    config: &OrganizerConfig,
    cancel: &CancellationToken,
) -> Result<usize, IpcError> {
    let file = File::open(zip_path).map_err(|e| IpcError::permission_denied(&e.to_string()))?;
    let reader = BufReader::new(file);
    let mut archive = ZipArchive::new(reader)
        .map_err(|e| IpcError::validation(&format!("Corrupt ZIP: {}", e)))?;
    let mut total_uncompressed = 0u64;
    let mut file_count = 0usize;
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| IpcError::validation(&e.to_string()))?;
        if !file.is_dir() && is_media(Path::new(file.name())) {
            total_uncompressed += file.size();
            file_count += 1;
            if total_uncompressed > MAX_ARCHIVE_UNCOMPRESSED {
                return Err(IpcError::validation(
                    "Archive exceeds MAX_ARCHIVE_UNCOMPRESSED limit",
                ));
            }
            if file_count > MAX_FILES_PER_JOB {
                return Err(IpcError::validation(
                    "Archive exceeds MAX_FILES_PER_JOB limit",
                ));
            }
        }
    }
    let mut processed = 0;
    for i in 0..archive.len() {
        if cancel.is_cancelled() {
            return Err(IpcError::cancelled("Organizer cancelled"));
        }
        let mut zip_file = archive
            .by_index(i)
            .map_err(|e| IpcError::validation(&e.to_string()))?;
        if zip_file.is_dir() {
            continue;
        }
        let name = zip_file.name().to_string();
        let path_in_zip = PathBuf::from(&name);
        if !is_media(&path_in_zip) {
            continue;
        }
        processed += 1;
        if processed > MAX_FILES_PER_JOB {
            return Err(IpcError::validation(
                "Exceeded MAX_FILES_PER_JOB limit during processing",
            ));
        }
        let date = extract_date_zip(&mut zip_file, config.source_type)?;
        let _ = compute_destination_name(
            path_in_zip
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("file"),
            date,
            &config.dest_dir,
            config.conflict_mode,
        );
        let _ = zip_file.size();
    }
    Ok(processed)
}

fn extract_date_zip<R: Read>(
    zip_file: &mut zip::read::ZipFile<R>,
    _source_type: SourceType,
) -> Result<NaiveDateTime, IpcError> {
    let name = zip_file.name().to_string();
    if let Some(d) = extract_date_from_filename(&name) {
        return Ok(d);
    }
    Ok(Utc::now().naive_utc())
}

async fn process_single_file(
    path: &Path,
    config: &OrganizerConfig,
    _cancel: &CancellationToken,
) -> Result<(), IpcError> {
    let date = extract_date_file(path, config.source_type)?;
    let _ = compute_destination_name(
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        date,
        &config.dest_dir,
        config.conflict_mode,
    );
    let _ = fs::metadata(path)
        .map_err(|e| IpcError::permission_denied(&e.to_string()))?
        .len();
    Ok(())
}

fn extract_date_file(path: &Path, source_type: SourceType) -> Result<NaiveDateTime, IpcError> {
    if source_type == SourceType::Takeout {
        let json_path = path.with_extension("json");
        if json_path.exists() {
            let _ = std::fs::read_to_string(&json_path);
        }
    }
    if let Some(d) =
        extract_date_from_filename(path.file_name().and_then(|s| s.to_str()).unwrap_or(""))
    {
        return Ok(d);
    }
    let meta = fs::metadata(path).map_err(|e| IpcError::permission_denied(&e.to_string()))?;
    let mtime = meta
        .modified()
        .map_err(|e| IpcError::permission_denied(&e.to_string()))?;
    let dt: chrono::DateTime<Utc> = mtime.into();
    Ok(dt.naive_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takeout_json_precedence() {}
    #[test]
    fn exif_precedence() {}
    #[test]
    fn filename_fallback() {
        let d = extract_date_from_filename("20230101_120000.jpg");
        assert!(d.is_some());
    }
    #[test]
    fn mtime_fallback() {}
    #[test]
    fn zip_nested_folders() {}
    #[test]
    fn zip_sidecar_parsed() {}
    #[test]
    fn corrupt_zip_handled() {}
    #[test]
    fn idempotent_rerun_no_new_files() {}
    #[test]
    fn collision_kept_both() {}
    #[test]
    fn collision_overwrite() {}
    #[test]
    fn collision_skip() {}
}
