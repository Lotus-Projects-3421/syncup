use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::storage::StorageError;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: String,
    pub window_width: u32,
    pub window_height: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            window_width: 1200,
            window_height: 950,
        }
    }
}

/// Loads application settings from `<data_dir>/settings.json`.
///
/// Falls back to defaults if the file does not exist or cannot be parsed.
pub fn load_settings(data_dir: &Path) -> Result<AppSettings, StorageError> {
    let path = data_dir.join("settings.json");
    if !path.exists() {
        return Ok(AppSettings::default());
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(error = %err, "could not read settings.json, using defaults");
            return Ok(AppSettings::default());
        }
    };

    match serde_json::from_str::<AppSettings>(&content) {
        Ok(settings) => Ok(settings),
        Err(err) => {
            tracing::warn!(error = %err, "could not parse settings.json, using defaults");
            Ok(AppSettings::default())
        }
    }
}

/// Atomically saves application settings to `<data_dir>/settings.json`.
pub fn save_settings(data_dir: &Path, settings: &AppSettings) -> Result<(), StorageError> {
    std::fs::create_dir_all(data_dir).map_err(StorageError::CreateDirectory)?;

    let target_path = data_dir.join("settings.json");
    let random_suffix: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let temp_path = data_dir.join(format!(
        ".settings.json.part-{}-{:x}",
        std::process::id(),
        random_suffix
    ));

    let json_bytes = serde_json::to_vec_pretty(settings).map_err(StorageError::Json)?;

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let defaults = AppSettings::default();
        assert_eq!(defaults.theme, "dark");
        assert_eq!(defaults.window_width, 1200);
        assert_eq!(defaults.window_height, 950);
    }

    #[test]
    fn test_save_and_load_settings() {
        let temp_dir = tempfile::tempdir().unwrap();
        let custom = AppSettings {
            theme: "light".to_string(),
            window_width: 1920,
            window_height: 1080,
        };

        save_settings(temp_dir.path(), &custom).unwrap();
        let loaded = load_settings(temp_dir.path()).unwrap();
        assert_eq!(loaded, custom);
    }

    #[test]
    fn test_load_nonexistent_returns_defaults() {
        let temp_dir = tempfile::tempdir().unwrap();
        let loaded = load_settings(temp_dir.path()).unwrap();
        assert_eq!(loaded, AppSettings::default());
    }
}
