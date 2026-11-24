#[test]
fn same_size_different_content_not_grouped() {}
#[test]
fn exact_dupes_grouped() {}
#[test]
fn quick_hash_collision_caught_by_full_hash() {}
#[test]
fn preview_no_side_effects() {}
#[test]
fn manifest_written_before_first_move() {}
#[test]
fn manifest_fsync_after_each_move() {}
#[test]
fn crash_between_move_and_manifest_reconciled() {}
#[test]
fn partial_move_failure_recoverable() {}
#[test]
fn undo_restores_all() {}
#[test]
fn undo_idempotent() {}
#[test]
fn undo_collision_aware_skips_blocked_op() {}
#[test]
fn cross_filesystem_move_uses_copy_verified_and_removes_source() {}
#[test]
fn destination_collision_without_overwrite_same_fs() {}
#[test]
fn destination_collision_without_overwrite_cross_fs() {}
#[test]
fn source_modified_after_scan_rejected() {}
#[test]
fn symlink_not_followed_during_scan() {}
#[test]
fn overlapping_scan_roots_deduplicated() {}
#[test]
fn undo_pending_reconciliation_resumes_safely() {}
#[test]
fn undo_ambiguous_reconciliation_surfaced() {}
#[test]
fn malformed_manifest_moved_to_corrupt_folder() {}
#[test]
fn readiness_gate_blocks_commands_until_reconciliation_completes() {}
