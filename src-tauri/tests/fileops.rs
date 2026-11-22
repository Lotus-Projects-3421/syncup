use std::io::Read;
use std::sync::atomic::Ordering;

use syncup_lib::ipc::error::IpcError;
use syncup_lib::services::fileops::{
    cleanup_orphan_temp_files, copy_verified, copy_verified_with_root, reset_collision_tracker,
    ConflictMode, Destination, FileOpError, Source, VerifyMode, SIMULATED_FREE_BYTES,
    SIMULATE_DIR_FSYNC_FAIL, SIMULATE_HASH_MISMATCH,
};
use tokio_util::sync::CancellationToken;

/// Serializes integration tests that use the process-global fault-injection hooks
/// (`SIMULATE_DIR_FSYNC_FAIL`, `SIMULATED_FREE_BYTES`, `SIMULATE_HASH_MISMATCH`) and the
/// global run collision tracker. Without this, a test that enables a hook can leak it into
/// a concurrently running test and cause spurious failures. This only orders test execution;
/// the production durability/fsync behavior is unchanged.
static FILEOPS_TEST_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

#[tokio::test]
async fn copy_success_file_source() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let src_dir = tempfile::tempdir().expect("source tempdir");
    let dest_dir = tempfile::tempdir().expect("dest tempdir");

    let src_file = src_dir.path().join("source.dat");
    let payload = vec![0xAB; 1024 * 100]; // 100 KiB
    std::fs::write(&src_file, &payload).expect("write source file");

    let final_path = dest_dir.path().join("target.dat");
    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(1024 * 100),
        verify: VerifyMode::SizeAndHash,
    };

    let published = copy_verified(Source::File(src_file), dest, CancellationToken::new())
        .await
        .expect("copy_verified file source");

    assert_eq!(published.final_path, final_path);
    assert_eq!(published.bytes_written, 1024 * 100);
    assert!(published.hash.is_some());

    let copied = std::fs::read(&final_path).expect("read final path");
    assert_eq!(copied, payload);
}

#[tokio::test]
async fn copy_success_stream_source() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let payload = b"Hello, stream source world!";

    let final_path = dest_dir.path().join("stream.txt");
    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(payload.len() as u64),
        verify: VerifyMode::SizeAndHash,
    };

    let stream = Box::new(std::io::Cursor::new(payload.to_vec()));
    let published = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(payload.len() as u64),
        },
        dest,
        CancellationToken::new(),
    )
    .await
    .expect("copy_verified stream source");

    assert_eq!(published.final_path, final_path);
    assert_eq!(published.bytes_written, payload.len() as u64);
    assert!(published.hash.is_some());

    let copied = std::fs::read(&final_path).expect("read final path");
    assert_eq!(copied, payload);
}

#[tokio::test]
async fn copy_size_mismatch_rejected() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let payload = b"12345";

    let final_path = dest_dir.path().join("mismatch_size.txt");
    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(100), // Expecting 100 bytes, only 5 provided
        verify: VerifyMode::SizeOnly,
    };

    let stream = Box::new(std::io::Cursor::new(payload.to_vec()));
    let res = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(5),
        },
        dest,
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(
        res,
        Err(FileOpError::VerifyFailed {
            expected_size: 100,
            actual_size: 5
        })
    ));
    assert!(!final_path.exists());
}

#[tokio::test]
async fn copy_content_mismatch_rejected() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let payload = b"content to hash";

    let final_path = dest_dir.path().join("mismatch_hash.txt");
    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(payload.len() as u64),
        verify: VerifyMode::SizeAndHash,
    };

    SIMULATE_HASH_MISMATCH.store(true, Ordering::SeqCst);
    let stream = Box::new(std::io::Cursor::new(payload.to_vec()));
    let res = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(payload.len() as u64),
        },
        dest,
        CancellationToken::new(),
    )
    .await;
    SIMULATE_HASH_MISMATCH.store(false, Ordering::SeqCst);

    assert!(matches!(res, Err(FileOpError::HashMismatch { .. })));
    assert!(!final_path.exists());
}

#[tokio::test]
async fn overwrite_existing_replaces_verified() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("replaceable.txt");
    std::fs::write(&final_path, b"initial old content").expect("write initial file");

    let new_payload = b"new verified content";
    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(new_payload.len() as u64),
        verify: VerifyMode::SizeAndHash,
    };

    let stream = Box::new(std::io::Cursor::new(new_payload.to_vec()));
    let published = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(new_payload.len() as u64),
        },
        dest,
        CancellationToken::new(),
    )
    .await
    .expect("overwrite succeeded");

    assert_eq!(published.final_path, final_path);
    let copied = std::fs::read(&final_path).expect("read replaced file");
    assert_eq!(copied, new_payload);
}

#[tokio::test]
async fn overwrite_existing_preserved_on_publish_failure() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("cannot_replace_directory");
    std::fs::create_dir_all(&final_path).expect("create directory at final_path");
    let sentinel = final_path.join("sentinel.txt");
    std::fs::write(&sentinel, b"sentinel value").expect("write sentinel");

    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(4),
        verify: VerifyMode::SizeOnly,
    };

    let stream = Box::new(std::io::Cursor::new(b"test".to_vec()));
    let res = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(4),
        },
        dest,
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(res, Err(FileOpError::PublishFailed { .. })));
    assert!(final_path.is_dir());
    assert_eq!(
        std::fs::read(&sentinel).expect("read sentinel"),
        b"sentinel value"
    );
}

#[tokio::test]
async fn overwrite_existing_preserved_on_verify_failure() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("preserve_me.txt");
    std::fs::write(&final_path, b"preserved original").expect("write original");

    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(100), // Mismatch with 4 bytes
        verify: VerifyMode::SizeOnly,
    };

    let stream = Box::new(std::io::Cursor::new(b"test".to_vec()));
    let res = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(4),
        },
        dest,
        CancellationToken::new(),
    )
    .await;

    assert!(matches!(res, Err(FileOpError::VerifyFailed { .. })));
    assert_eq!(
        std::fs::read(&final_path).expect("read preserved"),
        b"preserved original"
    );
}

#[tokio::test]
async fn disk_full_error_mapped() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("disk_full.txt");

    SIMULATED_FREE_BYTES.store(50, Ordering::SeqCst);

    let dest = Destination {
        final_path,
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(500), // Needed 500, only 50 available
        verify: VerifyMode::SizeOnly,
    };

    let stream = Box::new(std::io::Cursor::new(vec![0u8; 500]));
    let res = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(500),
        },
        dest,
        CancellationToken::new(),
    )
    .await;

    SIMULATED_FREE_BYTES.store(u64::MAX, Ordering::SeqCst);

    assert!(matches!(
        res,
        Err(FileOpError::DiskFull {
            needed: 500,
            available: 50
        })
    ));

    if let Err(err) = res {
        let ipc: IpcError = err.into();
        assert_eq!(ipc.code, "DISK_FULL");
    }
}

#[tokio::test]
async fn permission_denied_mapped() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let dest_dir = tempfile::tempdir().expect("dest tempdir");
        let sub_dir = dest_dir.path().join("readonly_dir");
        std::fs::create_dir_all(&sub_dir).expect("create readonly_dir");
        std::fs::set_permissions(&sub_dir, std::fs::Permissions::from_mode(0o555))
            .expect("set 0o555 perms");

        let dest = Destination {
            final_path: sub_dir.join("test.txt"),
            conflict_mode: ConflictMode::Overwrite,
            expected_size: Some(4),
            verify: VerifyMode::SizeOnly,
        };

        let stream = Box::new(std::io::Cursor::new(b"test".to_vec()));
        let res = copy_verified(
            Source::Stream {
                reader: stream,
                size_hint: Some(4),
            },
            dest,
            CancellationToken::new(),
        )
        .await;

        // Restore permissions so tempdir drops cleanly
        let _ = std::fs::set_permissions(&sub_dir, std::fs::Permissions::from_mode(0o755));

        assert!(matches!(res, Err(FileOpError::PermissionDenied { .. })));
        if let Err(err) = res {
            let ipc: IpcError = err.into();
            assert_eq!(ipc.code, "PERMISSION_DENIED");
        }
    }
    #[cfg(not(unix))]
    {
        // Chmod 0o555 permission test is unix-only; skipped on non-unix platforms.
    }
}

struct CancellingReader {
    token: CancellationToken,
    chunks_read: usize,
    cancel_after: usize,
}

impl Read for CancellingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.chunks_read += 1;
        if self.chunks_read >= self.cancel_after {
            self.token.cancel();
        }
        let chunk_size = buf.len().min(64 * 1024);
        for byte in &mut buf[..chunk_size] {
            *byte = 0x55;
        }
        Ok(chunk_size)
    }
}

#[tokio::test]
async fn cancel_mid_copy_leaves_no_temp() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("cancelled.bin");
    let token = CancellationToken::new();

    let reader = CancellingReader {
        token: token.clone(),
        chunks_read: 0,
        cancel_after: 3,
    };

    let dest = Destination {
        final_path,
        conflict_mode: ConflictMode::Overwrite,
        expected_size: None,
        verify: VerifyMode::SizeOnly,
    };

    let res = copy_verified(
        Source::Stream {
            reader: Box::new(reader),
            size_hint: None,
        },
        dest,
        token,
    )
    .await;

    assert!(matches!(res, Err(FileOpError::Cancelled)));

    let entries = std::fs::read_dir(dest_dir.path()).expect("read dest_dir");
    for entry in entries {
        let entry = entry.expect("entry");
        let name = entry.file_name().to_string_lossy().to_string();
        assert!(
            !name.starts_with(".syncup-part-"),
            "Found leaked temporary file: {name}"
        );
    }
}

#[tokio::test]
async fn cancel_preserves_existing_dest() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("preserved_on_cancel.txt");
    std::fs::write(&final_path, b"do not overwrite me").expect("write existing file");

    let token = CancellationToken::new();
    token.cancel(); // Pre-cancelled

    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(4),
        verify: VerifyMode::SizeOnly,
    };

    let stream = Box::new(std::io::Cursor::new(b"test".to_vec()));
    let res = copy_verified(
        Source::Stream {
            reader: stream,
            size_hint: Some(4),
        },
        dest,
        token,
    )
    .await;

    assert!(matches!(res, Err(FileOpError::Cancelled)));
    assert_eq!(
        std::fs::read(&final_path).expect("read preserved"),
        b"do not overwrite me"
    );
}

#[tokio::test]
async fn symlink_outside_root_rejected() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let approved_root = temp.path().join("approved");
    let outside_dir = temp.path().join("outside");
    std::fs::create_dir_all(&approved_root).expect("create approved root");
    std::fs::create_dir_all(&outside_dir).expect("create outside dir");

    let sym_parent = approved_root.join("symlink_folder");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside_dir, &sym_parent).expect("create symlink");

    let dest = Destination {
        final_path: sym_parent.join("photo.jpg"),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(4),
        verify: VerifyMode::SizeOnly,
    };

    let stream = Box::new(std::io::Cursor::new(b"test".to_vec()));
    let res = copy_verified_with_root(
        Source::Stream {
            reader: stream,
            size_hint: Some(4),
        },
        dest,
        CancellationToken::new(),
        Some(&approved_root),
    )
    .await;

    #[cfg(unix)]
    assert!(matches!(res, Err(FileOpError::Validation { .. })));

    #[cfg(not(unix))]
    let _ = res;
}

#[tokio::test]
async fn concurrent_same_destination_serialized() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("concurrent_target.txt");

    let path_a = final_path.clone();
    let task_a = tokio::spawn(async move {
        let payload = vec![b'A'; 64 * 1024];
        let dest = Destination {
            final_path: path_a,
            conflict_mode: ConflictMode::Overwrite,
            expected_size: Some(payload.len() as u64),
            verify: VerifyMode::SizeAndHash,
        };
        copy_verified(
            Source::Stream {
                reader: Box::new(std::io::Cursor::new(payload)),
                size_hint: Some(64 * 1024),
            },
            dest,
            CancellationToken::new(),
        )
        .await
    });

    let path_b = final_path.clone();
    let task_b = tokio::spawn(async move {
        let payload = vec![b'B'; 64 * 1024];
        let dest = Destination {
            final_path: path_b,
            conflict_mode: ConflictMode::Overwrite,
            expected_size: Some(payload.len() as u64),
            verify: VerifyMode::SizeAndHash,
        };
        copy_verified(
            Source::Stream {
                reader: Box::new(std::io::Cursor::new(payload)),
                size_hint: Some(64 * 1024),
            },
            dest,
            CancellationToken::new(),
        )
        .await
    });

    let (res_a, res_b) = tokio::join!(task_a, task_b);
    let pub_a = res_a.expect("task_a panicked").expect("task_a ok");
    let pub_b = res_b.expect("task_b panicked").expect("task_b ok");

    assert_eq!(pub_a.final_path, final_path);
    assert_eq!(pub_b.final_path, final_path);

    let content = std::fs::read(&final_path).expect("read serialized output");
    assert!(
        content == vec![b'A'; 64 * 1024] || content == vec![b'B'; 64 * 1024],
        "Expected complete content from either task A or task B"
    );
}

#[tokio::test]
async fn same_run_collision_deterministic() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    reset_collision_tracker();

    let final_path = dest_dir.path().join("photo.jpg");

    let dest1 = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::KeepBoth,
        expected_size: Some(5),
        verify: VerifyMode::SizeAndHash,
    };
    let pub1 = copy_verified(
        Source::Stream {
            reader: Box::new(std::io::Cursor::new(b"first".to_vec())),
            size_hint: Some(5),
        },
        dest1,
        CancellationToken::new(),
    )
    .await
    .expect("first copy ok");

    let dest2 = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::KeepBoth,
        expected_size: Some(6),
        verify: VerifyMode::SizeAndHash,
    };
    let pub2 = copy_verified(
        Source::Stream {
            reader: Box::new(std::io::Cursor::new(b"second".to_vec())),
            size_hint: Some(6),
        },
        dest2,
        CancellationToken::new(),
    )
    .await
    .expect("second copy ok");

    assert_eq!(pub1.final_path, dest_dir.path().join("photo.jpg"));
    assert_eq!(pub2.final_path, dest_dir.path().join("photo (1).jpg"));

    assert_eq!(
        std::fs::read(&pub1.final_path).expect("read pub1"),
        b"first"
    );
    assert_eq!(
        std::fs::read(&pub2.final_path).expect("read pub2"),
        b"second"
    );
}

#[test]
fn orphan_temp_cleanup_on_startup() {
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let p1 = dest_dir.path().join(".syncup-part-orphan1");
    let p2 = dest_dir.path().join(".syncup-part-orphan2");
    let p3 = dest_dir.path().join(".syncup-part-orphan3");
    let regular = dest_dir.path().join("unrelated_photo.jpg");

    std::fs::write(&p1, b"orphan 1").expect("write p1");
    std::fs::write(&p2, b"orphan 2").expect("write p2");
    std::fs::write(&p3, b"orphan 3").expect("write p3");
    std::fs::write(&regular, b"keep this photo").expect("write regular");

    let cleaned = cleanup_orphan_temp_files(dest_dir.path()).expect("cleanup ok");
    assert_eq!(cleaned, 3);

    assert!(!p1.exists());
    assert!(!p2.exists());
    assert!(!p3.exists());
    assert!(regular.exists());
}

#[tokio::test]
async fn publish_pending_durability_state_reported() {
    let _test_guard = FILEOPS_TEST_LOCK.lock().await;
    let dest_dir = tempfile::tempdir().expect("dest tempdir");
    let final_path = dest_dir.path().join("pending_durability.txt");

    SIMULATE_DIR_FSYNC_FAIL.store(true, Ordering::SeqCst);

    let dest = Destination {
        final_path: final_path.clone(),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(4),
        verify: VerifyMode::SizeAndHash,
    };

    let res = copy_verified(
        Source::Stream {
            reader: Box::new(std::io::Cursor::new(b"data".to_vec())),
            size_hint: Some(4),
        },
        dest,
        CancellationToken::new(),
    )
    .await;

    SIMULATE_DIR_FSYNC_FAIL.store(false, Ordering::SeqCst);

    match res {
        Err(FileOpError::PublishPendingDurability {
            final_path: err_path,
            ..
        }) => {
            assert_eq!(err_path, final_path);
            let ipc: IpcError = FileOpError::PublishPendingDurability {
                final_path: err_path,
                source: std::io::Error::new(std::io::ErrorKind::Other, "simulated fail"),
            }
            .into();
            assert_eq!(ipc.code, "PUBLISH_PENDING_DURABILITY");
            assert!(
                final_path.exists(),
                "File must exist at final_path despite durability error"
            );
        }
        other => panic!("Expected PublishPendingDurability, got {other:?}"),
    }
}

#[test]
#[cfg(all(test, windows))]
fn windows_durability_compiles_only_on_windows() {
    // Windows durability implementation verification
}
