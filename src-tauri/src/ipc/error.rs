use serde::Serialize;
use specta::Type;

pub const DB_ERROR: &str = "DB_ERROR";
pub const MIGRATION_FAILED: &str = "MIGRATION_FAILED";
pub const UNKNOWN: &str = "UNKNOWN";

pub const DISK_FULL: &str = "DISK_FULL";
pub const PERMISSION_DENIED: &str = "PERMISSION_DENIED";
pub const IO_ERROR: &str = "IO_ERROR";
pub const PUBLISH_PENDING_DURABILITY: &str = "PUBLISH_PENDING_DURABILITY";
pub const CANCELLED: &str = "CANCELLED";
pub const VALIDATION_ERROR: &str = "VALIDATION_ERROR";
pub const VERIFY_FAILED: &str = "VERIFY_FAILED";
pub const PUBLISH_FAILED: &str = "PUBLISH_FAILED";
pub const DEVICE_DISCONNECTED: &str = "DEVICE_DISCONNECTED";

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
    pub fn migration(message: &str) -> Self {
        Self::new(MIGRATION_FAILED, message)
    }

    #[must_use]
    pub fn disk_full(message: &str) -> Self {
        Self::new(DISK_FULL, message)
    }

    #[must_use]
    pub fn permission_denied(message: &str) -> Self {
        Self::new(PERMISSION_DENIED, message)
    }

    #[must_use]
    pub fn io(message: &str) -> Self {
        Self::new(IO_ERROR, message)
    }

    #[must_use]
    pub fn publish_pending_durability(message: &str) -> Self {
        Self::new(PUBLISH_PENDING_DURABILITY, message)
    }

    #[must_use]
    pub fn cancelled(message: &str) -> Self {
        Self::new(CANCELLED, message)
    }

    #[must_use]
    pub fn validation(message: &str) -> Self {
        Self::new(VALIDATION_ERROR, message)
    }

    #[must_use]
    pub fn verify_failed(message: &str) -> Self {
        Self::new(VERIFY_FAILED, message)
    }

    #[must_use]
    pub fn publish_failed(message: &str) -> Self {
        Self::new(PUBLISH_FAILED, message)
    }

    #[must_use]
    pub fn device_disconnected(message: &str) -> Self {
        Self::new(DEVICE_DISCONNECTED, message)
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

    #[test]
    fn constructors_map_correct_error_codes() {
        assert_eq!(IpcError::disk_full("Disk is full").code, DISK_FULL);
        assert_eq!(
            IpcError::permission_denied("Permission denied").code,
            PERMISSION_DENIED
        );
        assert_eq!(IpcError::io("I/O error").code, IO_ERROR);
        assert_eq!(
            IpcError::publish_pending_durability("Publish pending durability").code,
            PUBLISH_PENDING_DURABILITY
        );
        assert_eq!(IpcError::cancelled("Cancelled").code, CANCELLED);
        assert_eq!(
            IpcError::validation("Validation error").code,
            VALIDATION_ERROR
        );
        assert_eq!(IpcError::verify_failed("Verify failed").code, VERIFY_FAILED);
        assert_eq!(
            IpcError::publish_failed("Publish failed").code,
            PUBLISH_FAILED
        );
        assert_eq!(
            IpcError::device_disconnected("Device disconnected").code,
            DEVICE_DISCONNECTED
        );
    }
}
