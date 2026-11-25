//! Phase 9 Slice E backend correction: end-to-end Wi-Fi finalization coverage.
//!
//! Every test here drives `WifiSessionManager::stop()`, the real IPC entry point.
//! The unit tests inside `services/wifi.rs` call `finalize_blocking` directly with
//! a freshly registered cancellation token, so they cannot detect a defect in the
//! `stop_session` wiring that decides whether finalization runs at all.
//!
//! Synthetic fixtures only: tempfile data dir and destination root, loopback
//! listener, small injected limits. No real user data and no real device.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use syncup_lib::ipc::models::{WifiConfig, WifiSessionInfo};
use syncup_lib::services::jobs::{JobRegistry, JobStatus};
use syncup_lib::services::wifi::{WifiLimits, WifiSessionManager};

struct Harness {
    _data: tempfile::TempDir,
    dest: tempfile::TempDir,
    manager: WifiSessionManager,
    registry: JobRegistry,
    info: WifiSessionInfo,
    addr: SocketAddr,
    staging: PathBuf,
}

fn small_limits() -> WifiLimits {
    WifiLimits {
        max_file_bytes: 4096,
        max_session_bytes: 8192,
        rate_per_sec: 0.0,
        burst: 100,
        max_concurrent_uploads: 2,
        ttl: Duration::from_secs(60),
    }
}

async fn start_session(limits: WifiLimits) -> Harness {
    let data = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let manager = WifiSessionManager::new();
    let registry = JobRegistry::new(None);
    let config = WifiConfig {
        destination_root: dest.path().to_string_lossy().to_string(),
        bind_all: false,
        ttl_secs: None,
    };
    let info = manager
        .start_with_options(
            config,
            data.path(),
            &registry,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            limits,
        )
        .await
        .unwrap();
    let addr: SocketAddr = format!("{}:{}", info.ip, info.port).parse().unwrap();
    let staging = data.path().join("wifi_staging").join(&info.session_id);
    Harness {
        _data: data,
        dest,
        manager,
        registry,
        info,
        addr,
        staging,
    }
}

fn parse_status(raw: &[u8]) -> u16 {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP response header terminator");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    head.lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}

async fn post(h: &Harness, path: &str, extra: &[(&str, String)], body: &[u8]) -> u16 {
    let mut stream = tokio::net::TcpStream::connect(h.addr).await.unwrap();
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAuthorization: Bearer {}\r\nContent-Length: {}\r\n",
        h.addr,
        h.info.token,
        body.len()
    );
    for (key, value) in extra {
        head.push_str(&format!("{key}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    parse_status(&response)
}

/// Declares one file and uploads it. The manifest allowlist is replaced on each
/// POST, so files are declared and uploaded one at a time.
async fn upload(h: &Harness, rel: &str, body: &[u8]) {
    let manifest = serde_json::to_vec(&json!({
        "files": [{ "relPath": rel, "size": body.len(), "mtimeNs": 1_700_000_000_000_000_000_i64 }]
    }))
    .unwrap();
    let headers = [("Content-Type", "application/json".to_string())];
    assert_eq!(post(h, "/manifest", &headers, &manifest).await, 200);
    let upload_headers = [("X-SyncUp-Rel-Path", rel.to_string())];
    assert_eq!(post(h, "/upload", &upload_headers, body).await, 200);
}

fn count_files(dir: &Path) -> usize {
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                count += 1;
            }
        }
    }
    count
}

/// Polls for the asynchronous watcher-driven terminal state used by the
/// cancellation and expiry paths.
async fn wait_for_cancelled(h: &Harness) -> bool {
    for _ in 0..250 {
        let job = h.registry.get(&h.info.job_id).unwrap();
        if job.status == JobStatus::Cancelled && !h.staging.exists() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn stop_publishes_staged_files_to_destination() {
    let h = start_session(small_limits()).await;
    upload(&h, "DCIM/Camera/IMG_0001.jpg", b"first-photo").await;
    upload(&h, "notes.txt", b"second-file").await;
    assert_eq!(count_files(&h.staging), 2, "both uploads staged");
    assert_eq!(
        count_files(h.dest.path()),
        0,
        "nothing published before stop"
    );

    h.manager.stop().await.unwrap();

    assert_eq!(
        std::fs::read(h.dest.path().join("DCIM/Camera/IMG_0001.jpg")).unwrap(),
        b"first-photo"
    );
    assert_eq!(
        std::fs::read(h.dest.path().join("notes.txt")).unwrap(),
        b"second-file"
    );
    assert!(
        !h.staging.exists(),
        "staging removed only after every file published"
    );
    let job = h.registry.get(&h.info.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Completed);
    assert!(job.last_error.is_none());
    assert!(
        job.finished_at.is_some(),
        "stop() returns after the job is terminal"
    );
}

#[tokio::test]
async fn stop_with_empty_staging_completes() {
    let h = start_session(small_limits()).await;
    assert_eq!(count_files(&h.staging), 0);

    h.manager.stop().await.unwrap();

    assert!(!h.staging.exists());
    assert_eq!(count_files(h.dest.path()), 0);
    let job = h.registry.get(&h.info.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Completed);
}

#[tokio::test]
async fn stop_is_idempotent_after_success() {
    let h = start_session(small_limits()).await;
    upload(&h, "a.jpg", b"abc").await;

    h.manager.stop().await.unwrap();
    h.manager.stop().await.unwrap();

    assert_eq!(std::fs::read(h.dest.path().join("a.jpg")).unwrap(), b"abc");
    assert_eq!(
        count_files(h.dest.path()),
        1,
        "second stop republishes nothing"
    );
    let job = h.registry.get(&h.info.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Completed);
}

#[tokio::test]
async fn cancelled_session_does_not_publish() {
    let h = start_session(small_limits()).await;
    upload(&h, "discard.jpg", b"discard-me").await;

    h.registry.cancel(&h.info.job_id).unwrap();

    assert!(
        wait_for_cancelled(&h).await,
        "cancelled session must stop and wipe staging"
    );
    assert_eq!(
        count_files(h.dest.path()),
        0,
        "cancellation discards staging without publishing"
    );
}

#[tokio::test]
async fn expiry_does_not_publish() {
    let mut limits = small_limits();
    limits.ttl = Duration::from_millis(800);
    let h = start_session(limits).await;
    upload(&h, "expire.jpg", b"expire-me").await;

    assert!(
        wait_for_cancelled(&h).await,
        "expired session must stop and wipe staging"
    );
    assert_eq!(
        count_files(h.dest.path()),
        0,
        "expiry discards staging without publishing"
    );
}

#[tokio::test]
async fn finalization_failure_preserves_staging() {
    let h = start_session(small_limits()).await;
    // A regular file occupying the destination parent makes the publish fail
    // with ENOTDIR. Deterministic for any uid, unlike a chmod-based fixture.
    std::fs::write(h.dest.path().join("blocked"), b"i am a file").unwrap();
    upload(&h, "blocked/child.jpg", b"cannot-land").await;

    let err = h.manager.stop().await.unwrap_err();

    assert!(
        err.message.contains("errors"),
        "unexpected error message: {}",
        err.message
    );
    assert!(
        h.staging.exists(),
        "staging preserved on finalization failure"
    );
    assert_eq!(count_files(&h.staging), 1);
    assert!(!h.dest.path().join("blocked/child.jpg").exists());
    let job = h.registry.get(&h.info.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Failed);
    assert!(job.last_error.is_some());
}

#[tokio::test]
async fn partial_publication_preserves_staging_on_error() {
    let h = start_session(small_limits()).await;
    std::fs::write(h.dest.path().join("blocked"), b"i am a file").unwrap();
    upload(&h, "good.jpg", b"published-ok").await;
    upload(&h, "blocked/child.jpg", b"cannot-land").await;

    let err = h.manager.stop().await.unwrap_err();
    assert!(err.message.contains("errors"));

    // Iteration order is filesystem-dependent; one file always publishes and
    // one always fails, so both assertions hold either way.
    assert_eq!(
        std::fs::read(h.dest.path().join("good.jpg")).unwrap(),
        b"published-ok",
        "a failed run can still leave published files behind"
    );
    assert!(
        h.staging.exists(),
        "staging preserved so the unpublished remainder survives"
    );
    assert_eq!(count_files(&h.staging), 2);
    let job = h.registry.get(&h.info.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Failed);
}
