use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::storage::StorageError;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeviceProfile {
    pub id: String,
    pub destination: String,
    pub sorting: String,
    pub filter: String,
    pub conflict: String,
    pub folders: Vec<String>,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            destination: String::new(),
            sorting: "year".to_string(),
            filter: "both".to_string(),
            conflict: "keep".to_string(),
            folders: Vec::new(),
        }
    }
}

/// Sanitizes a device identifier into a filesystem-safe profile ID.
///
/// Matches legacy Python `_safe_id`: retains only ASCII alphanumerics, `-`, and `_`,
/// falling back to `"default"` if empty.
#[must_use]
pub fn sanitize_safe_id(device_id: &str) -> String {
    let filtered: String = device_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if filtered.is_empty() {
        "default".to_string()
    } else {
        filtered
    }
}

/// Loads a profile from `<data_dir>/profile_<safe_id>.json`.
///
/// Returns `Ok(None)` if the profile file does not exist or is invalid.
pub fn load_profile(
    data_dir: &Path,
    device_id: &str,
) -> Result<Option<DeviceProfile>, StorageError> {
    let safe_id = sanitize_safe_id(device_id);
    let path = data_dir.join(format!("profile_{safe_id}.json"));
    if !path.exists() {
        return Ok(None);
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(error = %err, path = ?path, "could not read profile file");
            return Ok(None);
        }
    };

    match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(val) => {
            let destination = val
                .get("destination")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let sorting = val
                .get("sorting")
                .and_then(|v| v.as_str())
                .unwrap_or("year")
                .to_string();
            let filter = val
                .get("filter")
                .and_then(|v| v.as_str())
                .unwrap_or("both")
                .to_string();
            let conflict = val
                .get("conflict")
                .and_then(|v| v.as_str())
                .unwrap_or("keep")
                .to_string();
            let folders = val
                .get("folders")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| item.as_str().map(|s| s.trim_matches('/').to_string()))
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            Ok(Some(DeviceProfile {
                id: safe_id,
                destination,
                sorting,
                filter,
                conflict,
                folders,
            }))
        }
        Err(err) => {
            tracing::warn!(error = %err, path = ?path, "could not parse profile file");
            Ok(None)
        }
    }
}

/// Atomically saves a profile to `<data_dir>/profile_<safe_id>.json`.
pub fn save_profile(data_dir: &Path, profile: &DeviceProfile) -> Result<(), StorageError> {
    std::fs::create_dir_all(data_dir).map_err(StorageError::CreateDirectory)?;

    let safe_id = sanitize_safe_id(&profile.id);
    let filename = format!("profile_{safe_id}.json");
    let target_path = data_dir.join(&filename);

    let random_suffix: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let temp_path = data_dir.join(format!(
        ".{filename}.part-{}-{:x}",
        std::process::id(),
        random_suffix
    ));

    let json_bytes = serde_json::to_vec_pretty(profile).map_err(StorageError::Json)?;

    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp_path).map_err(StorageError::Io)?;
        file.write_all(&json_bytes).map_err(StorageError::Io)?;
        file.sync_all().map_err(StorageError::Io)?;
    }

    std::fs::rename(&temp_path, &target_path).map_err(StorageError::Io)?;

    #[cfg(unix)]
    {
        if let Ok(dir_file) = std::fs::File::open(data_dir) {
            let _ = dir_file.sync_all();
        }
    }

    Ok(())
}

/// Lists all profiles found in `<data_dir>/profile_*.json`.
pub fn list_profiles(data_dir: &Path) -> Result<Vec<DeviceProfile>, StorageError> {
    if !data_dir.exists() {
        return Ok(Vec::new());
    }

    let mut profiles = Vec::new();
    let entries = std::fs::read_dir(data_dir).map_err(StorageError::Io)?;

    for entry in entries {
        let entry = entry.map_err(StorageError::Io)?;
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }

        if let Some(rest) = name.strip_prefix("profile_") {
            if let Some(safe_id) = rest.strip_suffix(".json") {
                if !safe_id.is_empty() {
                    if let Ok(Some(profile)) = load_profile(data_dir, safe_id) {
                        profiles.push(profile);
                    }
                }
            }
        }
    }

    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(profiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_safe_id() {
        assert_eq!(sanitize_safe_id("phone_123"), "phone_123");
        assert_eq!(sanitize_safe_id("phone/with/slashes"), "phonewithslashes");
        assert_eq!(sanitize_safe_id("wifi-backup"), "wifi-backup");
        assert_eq!(sanitize_safe_id("!@#$%^"), "default");
    }

    #[test]
    fn test_save_and_load_profile() {
        let temp_dir = tempfile::tempdir().unwrap();
        let profile = DeviceProfile {
            id: "my-phone".to_string(),
            destination: "/photos".to_string(),
            sorting: "month".to_string(),
            filter: "photos".to_string(),
            conflict: "keep".to_string(),
            folders: vec!["DCIM/Camera".to_string(), "Pictures".to_string()],
        };

        save_profile(temp_dir.path(), &profile).unwrap();
        let loaded = load_profile(temp_dir.path(), "my-phone").unwrap().unwrap();
        assert_eq!(loaded, profile);

        let list = list_profiles(temp_dir.path()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], profile);
    }
}
