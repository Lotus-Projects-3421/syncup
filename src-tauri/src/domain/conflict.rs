//! Destination conflict resolution and same-run collision handling.
//!
//! Deviation from Python oracle (core/fileops.py:40 uses `name__N.ext`); v2 uses
//! `name (N).ext` per Playbook §4.3 PROMPT C4.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Strategy to apply when a target destination already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ConflictMode {
    /// Keep existing file and write the new file with a deterministic `name (N).ext` suffix.
    KeepBoth,
    /// Overwrite the existing file after verifying the new copy.
    Overwrite,
    /// Skip the operation if the target file already exists.
    Skip,
}

/// Formats a collision-free filename given stem, 1-based counter, and optional extension.
#[must_use]
pub fn format_collision_name(stem: &str, counter: usize, extension: Option<&str>) -> String {
    match extension {
        Some(ext) if !ext.is_empty() => format!("{stem} ({counter}).{ext}"),
        _ => format!("{stem} ({counter})"),
    }
}

/// Tracks assigned paths across a job/run so KeepBoth collisions resolve
/// deterministically without depending on intermediate filesystem state.
#[derive(Debug, Default)]
pub struct RunCollisionTracker {
    reserved_paths: HashSet<PathBuf>,
}

impl RunCollisionTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            reserved_paths: HashSet::new(),
        }
    }

    /// Resolves the destination path for `candidate` under `mode`.
    ///
    /// - If `mode` is `Overwrite` or `Skip`: returns candidate as-is.
    /// - If `mode` is `KeepBoth`:
    ///   If candidate is neither present on disk nor already reserved in this run,
    ///   reserves and returns `candidate`.
    ///   Otherwise, increments a deterministic counter (1, 2, ...) producing `name (N).ext`
    ///   until an unreserved, non-existing path is found, marks it reserved, and returns it.
    pub fn resolve_path(&mut self, candidate: &Path, mode: ConflictMode) -> PathBuf {
        match mode {
            ConflictMode::Overwrite | ConflictMode::Skip => candidate.to_path_buf(),
            ConflictMode::KeepBoth => {
                if !candidate.exists() && !self.reserved_paths.contains(candidate) {
                    self.reserved_paths.insert(candidate.to_path_buf());
                    return candidate.to_path_buf();
                }

                let parent = candidate.parent().unwrap_or_else(|| Path::new(""));
                let stem = candidate.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let ext = candidate.extension().and_then(|e| e.to_str());

                let mut counter = 1;
                loop {
                    let file_name = format_collision_name(stem, counter, ext);
                    let next_candidate = parent.join(&file_name);
                    if !next_candidate.exists() && !self.reserved_paths.contains(&next_candidate) {
                        self.reserved_paths.insert(next_candidate.clone());
                        return next_candidate;
                    }
                    counter += 1;
                }
            }
        }
    }

    #[must_use]
    pub fn is_reserved(&self, path: &Path) -> bool {
        self.reserved_paths.contains(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_collision_name_with_and_without_extension() {
        assert_eq!(
            format_collision_name("photo", 1, Some("jpg")),
            "photo (1).jpg"
        );
        assert_eq!(
            format_collision_name("archive", 2, Some("tar.gz")),
            "archive (2).tar.gz"
        );
        assert_eq!(format_collision_name("README", 3, None), "README (3)");
        assert_eq!(format_collision_name("data", 4, Some("")), "data (4)");
    }

    #[test]
    fn deterministic_same_run_collision_without_filesystem_state() {
        let mut tracker = RunCollisionTracker::new();
        let target = PathBuf::from("/nonexistent/target/path/IMG_0001.jpg");

        let first = tracker.resolve_path(&target, ConflictMode::KeepBoth);
        assert_eq!(first, target);

        let second = tracker.resolve_path(&target, ConflictMode::KeepBoth);
        assert_eq!(
            second,
            PathBuf::from("/nonexistent/target/path/IMG_0001 (1).jpg")
        );

        let third = tracker.resolve_path(&target, ConflictMode::KeepBoth);
        assert_eq!(
            third,
            PathBuf::from("/nonexistent/target/path/IMG_0001 (2).jpg")
        );
    }

    #[test]
    fn overwrite_and_skip_return_candidate_directly() {
        let mut tracker = RunCollisionTracker::new();
        let target = PathBuf::from("/some/path/file.txt");

        let resolved_overwrite = tracker.resolve_path(&target, ConflictMode::Overwrite);
        assert_eq!(resolved_overwrite, target);

        let resolved_skip = tracker.resolve_path(&target, ConflictMode::Skip);
        assert_eq!(resolved_skip, target);
    }
}
