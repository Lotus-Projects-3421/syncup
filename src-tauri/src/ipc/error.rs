use serde::Serialize;
use specta::Type;

pub const DB_ERROR: &str = "DB_ERROR";
pub const UNKNOWN: &str = "UNKNOWN";

/// Structured error returned across the Tauri IPC boundary.
///
/// Messages are deliberately generic: internal paths, SQL text, and stack traces
/// must never be exposed to the webview.
#[derive(Debug, Clone, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: String,
    pub message: String,
}

impl IpcError {
    #[must_use]
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
        }
    }

    #[must_use]
    pub fn db(message: &str) -> Self {
        Self::new(DB_ERROR, message)
    }

    #[must_use]
    pub fn unknown() -> Self {
        Self::new(UNKNOWN, "An unexpected error occurred.")
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for IpcError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_camel_case_code_and_message() {
        let error = IpcError::new(DB_ERROR, "The local database is unavailable.");
        let value = serde_json::to_value(&error);

        assert!(value.is_ok());
        if let Ok(value) = value {
            assert_eq!(
                value.get("code").and_then(serde_json::Value::as_str),
                Some(DB_ERROR)
            );
            assert_eq!(
                value.get("message").and_then(serde_json::Value::as_str),
                Some("The local database is unavailable.")
            );
        }
    }

    #[test]
    fn unknown_error_has_a_stable_code() {
        let error = IpcError::unknown();
        assert_eq!(error.code, UNKNOWN);
    }
}
