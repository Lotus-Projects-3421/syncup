//! Platform-specific implementations. Linux MTP lives here; Windows and
//! macOS providers arrive in later phases.

#[cfg(target_os = "linux")]
pub mod linux;
