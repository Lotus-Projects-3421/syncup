//! Phase 6 Wi-Fi security test matrix (PLAN.md §7 + approved Q3 conditions).
//!
//! Synthetic fixtures only: every session uses tempfile::tempdir() for both the
//! app data dir and the destination root. No real user data, no real device
//! (except the single #[ignore] LAN hardware test). Requests are raw HTTP/1.1
//! over tokio TcpStream so tests can control Host/Origin/Content-Length/chunked
//! framing exactly. Limits are injected small via start_with_options.
//!
//! Synchronisation rule: server-side abort/expiry/cancel cleanup is asynchronous
//! and involves fsync. Tests that assert post-cleanup state poll for the real
//! condition with a bounded budget instead of sleeping a fixed interval, which
//! is unreliable when the suite runs in parallel on slow hardware.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use syncup_lib::ipc::models::WifiConfig;
use syncup_lib::services::jobs::{JobRegistry, JobStatus};
use syncup_lib::services::wifi::{WifiLimits, WifiSessionManager};

struct Harness {
    _data: tempfile::TempDir,
    _dest: tempfile::TempDir,
    manager: WifiSessionManager,
    registry: JobRegistry,
    info: syncup_lib::ipc::models::WifiSessionInfo,
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

async fn start_session(limits: WifiLimits, dest_files: &[(&str, &[u8])]) -> Harness {
    let data = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    for (name, content) in dest_files {
        let path = dest.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }
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
        _dest: dest,
        manager,
        registry,
        info,
        addr,
        staging,
    }
}

fn parse_response(raw: &[u8]) -> (u16, Vec<u8>) {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("HTTP response header terminator");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, raw[split + 4..].to_vec())
}

async fn raw_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: &[u8],
) -> (u16, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
    if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
        req.push_str(&format!("Host: {addr}\r\n"));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if !body.is_empty()
        && !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
    {
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    let _ = stream.write_all(req.as_bytes()).await;
    if !body.is_empty() {
        let _ = stream.write_all(body).await;
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    parse_response(&response)
}

fn auth_headers(token: &str) -> Vec<(&'static str, String)> {
    vec![("Authorization", format!("Bearer {token}"))]
}

async fn post_manifest(h: &Harness, files: serde_json::Value) -> (u16, Vec<u8>) {
    let mut headers = auth_headers(&h.info.token);
    headers.push(("Content-Type", "application/json".to_string()));
    let body = serde_json::to_vec(&json!({ "files": files })).unwrap();
    raw_request(h.addr, "POST", "/manifest", &headers, &body).await
}

/// POST /manifest that tolerates an already-closed listener. Returns `None`
/// when the session's server has shut down (connection refused or empty reply),
/// which is the expected outcome once a session expires or is stopped.
async fn try_post_manifest(h: &Harness, files: serde_json::Value) -> Option<(u16, Vec<u8>)> {
    let mut stream = match tokio::net::TcpStream::connect(h.addr).await {
        Ok(stream) => stream,
        Err(err) if err.kind() == std::io::ErrorKind::ConnectionRefused => return None,
        Err(err) => panic!("unexpected connect error: {err}"),
    };
    let body = serde_json::to_vec(&json!({ "files": files })).unwrap();
    let head = format!(
        "POST /manifest HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        h.addr, h.info.token, body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let mut response = Vec::new();
    let read = stream.read_to_end(&mut response).await.unwrap_or(0);
    if read == 0 {
        return None;
    }
    Some(parse_response(&response))
}

async fn post_upload(h: &Harness, rel: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let mut headers = auth_headers(&h.info.token);
    headers.push(("X-SyncUp-Rel-Path", rel.to_string()));
    raw_request(h.addr, "POST", "/upload", &headers, body).await
}

async fn chunked_upload(
    h: &Harness,
    rel: &str,
    chunks: &[Vec<u8>],
    delay: Duration,
    finish: bool,
) -> Option<(u16, Vec<u8>)> {
    let mut stream = tokio::net::TcpStream::connect(h.addr).await.unwrap();
    let head = format!(
        "POST /upload HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAuthorization: Bearer {}\r\nX-SyncUp-Rel-Path: {}\r\nTransfer-Encoding: chunked\r\n\r\n",
        h.addr, h.info.token, rel
    );
    let _ = stream.write_all(head.as_bytes()).await;
    for chunk in chunks {
        let _ = stream
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .await;
        let _ = stream.write_all(chunk).await;
        let _ = stream.write_all(b"\r\n").await;
        tokio::time::sleep(delay).await;
    }
    if !finish {
        drop(stream);
        return None;
    }
    let _ = stream.write_all(b"0\r\n\r\n").await;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    Some(parse_response(&response))
}

fn manifest_entry(rel: &str, size: u64) -> serde_json::Value {
    json!({ "relPath": rel, "size": size, "mtimeNs": 1_700_000_000_000_000_000_i64 })
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

#[tokio::test]
async fn expired_session_rejected() {
    let mut limits = small_limits();
    limits.ttl = Duration::from_millis(50);
    let h = start_session(limits, &[]).await;

    // Expiry performs a FULL stop: the watcher cancels uploads, shuts the
    // server down, wipes staging and finishes the job. Wait for that real
    // condition instead of racing a fixed sleep.
    let mut stopped = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let job = h.registry.get(&h.info.job_id).unwrap();
        if job.status == JobStatus::Cancelled && !h.staging.exists() {
            stopped = true;
            break;
        }
    }
    assert!(stopped, "expired session must stop and wipe staging");

    // The listener is normally closed by now, so the connection is refused.
    // If a socket is still draining, the guard must answer 401. Either outcome
    // proves the expired session cannot be used.
    if let Some((status, _)) = try_post_manifest(&h, json!([])).await {
        assert_eq!(status, 401, "expired session must reject requests");
    }
}

#[tokio::test]
async fn wrong_token_rejected() {
    let h = start_session(small_limits(), &[]).await;
    let headers = vec![("Authorization", "Bearer not-the-token".to_string())];
    let (status, body) =
        raw_request(h.addr, "POST", "/manifest", &headers, b"{\"files\":[]}").await;
    assert_eq!(status, 401);
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("WIFI_AUTH_FAILED"), "body: {text}");
}

#[tokio::test]
async fn missing_token_rejected() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = raw_request(h.addr, "POST", "/manifest", &[], b"{\"files\":[]}").await;
    assert_eq!(status, 401);
}

static LOG_CAPTURE: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();

struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
    type Writer = CaptureWriter;
    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter(self.0.clone())
    }
}

fn init_log_capture() -> Arc<Mutex<Vec<u8>>> {
    LOG_CAPTURE
        .get_or_init(|| {
            let buf = Arc::new(Mutex::new(Vec::new()));
            let subscriber = tracing_subscriber::fmt()
                .with_writer(CaptureWriter(buf.clone()))
                .with_ansi(false)
                .finish();
            let _ = tracing::subscriber::set_global_default(subscriber);
            buf
        })
        .clone()
}

#[tokio::test]
async fn token_never_logged() {
    let buf = init_log_capture();
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("a.jpg", 3)])).await;
    assert_eq!(status, 200);
    let (status, _) = post_upload(&h, "a.jpg", b"abc").await;
    assert_eq!(status, 200);
    let wrong = vec![("Authorization", "Bearer wrong".to_string())];
    let _ = raw_request(h.addr, "POST", "/manifest", &wrong, b"{\"files\":[]}").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let captured = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
    assert!(
        !captured.contains(&h.info.token),
        "session token must never appear in logs"
    );
}

#[tokio::test]
async fn token_never_in_url() {
    let h = start_session(small_limits(), &[]).await;
    assert!(!h.info.url.contains(&h.info.token));
    let path = format!("/manifest?token={}", h.info.token);
    let (status, _) = raw_request(h.addr, "POST", &path, &[], b"{\"files\":[]}").await;
    assert_eq!(status, 401, "query token must not authenticate");
}

#[tokio::test]
async fn path_traversal_rejected() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("../evil.jpg", 3)])).await;
    assert_eq!(status, 400);
    let (status, _) = post_manifest(&h, json!([manifest_entry("a/../../evil.jpg", 3)])).await;
    assert_eq!(status, 400);
    let (status, _) = post_manifest(&h, json!([manifest_entry("/abs.jpg", 3)])).await;
    assert_eq!(status, 400);
    let (status, _) = post_upload(&h, "../evil.jpg", b"abc").await;
    assert_eq!(status, 400);
    assert_eq!(count_files(&h.staging), 0);
}

#[tokio::test]
async fn filename_reserved_rejected() {
    let h = start_session(small_limits(), &[]).await;
    for name in ["CON", "NUL.jpg", "com1.png", "LPT9", "aux"] {
        let (status, _) = post_manifest(&h, json!([manifest_entry(name, 3)])).await;
        assert_eq!(status, 400, "reserved name {name} must be rejected");
    }
}

#[tokio::test]
async fn oversized_request_rejected() {
    let mut limits = small_limits();
    limits.max_file_bytes = 1024;
    let h = start_session(limits, &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("big.bin", 2048)])).await;
    assert_eq!(status, 413, "manifest entry above cap rejected early");
    let (status, _) = post_manifest(&h, json!([manifest_entry("ok.bin", 1024)])).await;
    assert_eq!(status, 200);
    let chunks = [vec![7u8; 600], vec![7u8; 600]];
    let (status, _) = chunked_upload(&h, "ok.bin", &chunks, Duration::ZERO, true)
        .await
        .unwrap();
    assert_eq!(status, 413, "streamed bytes above declared size aborted");
    assert_eq!(
        count_files(&h.staging),
        0,
        "no temp or partial file remains"
    );
    let (status, _) = post_upload(&h, "ok.bin", &[9u8; 1024]).await;
    assert_eq!(status, 200, "quota released after aborted upload");
}

#[tokio::test]
async fn origin_mismatch_rejected() {
    let h = start_session(small_limits(), &[]).await;
    let mut headers = auth_headers(&h.info.token);
    headers.push(("Origin", "http://evil.example".to_string()));
    let (status, _) = raw_request(h.addr, "POST", "/manifest", &headers, b"{\"files\":[]}").await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn host_mismatch_rejected() {
    let h = start_session(small_limits(), &[]).await;
    let mut headers = auth_headers(&h.info.token);
    headers.push(("Host", "evil.example".to_string()));
    let (status, _) = raw_request(h.addr, "POST", "/manifest", &headers, b"{\"files\":[]}").await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn rate_limit_enforced() {
    let mut limits = small_limits();
    limits.rate_per_sec = 0.0;
    limits.burst = 3;
    let h = start_session(limits, &[]).await;
    for _ in 0..3 {
        let (status, _) = post_manifest(&h, json!([])).await;
        assert_eq!(status, 200);
    }
    let (status, body) = post_manifest(&h, json!([])).await;
    assert_eq!(status, 429);
    assert!(String::from_utf8_lossy(&body).contains("WIFI_RATE_LIMIT"));
}

#[tokio::test]
async fn session_isolation_staging() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("a.jpg", 3)])).await;
    assert_eq!(status, 200);
    let (status, _) = post_upload(&h, "a.jpg", b"abc").await;
    assert_eq!(status, 200);
    let first_staging = h.staging.clone();
    let old_token = h.info.token.clone();
    let old_addr = h.addr;
    h.manager.stop().await.unwrap();
    assert!(!first_staging.exists());

    let config = WifiConfig {
        destination_root: h._dest.path().to_string_lossy().to_string(),
        bind_all: false,
        ttl_secs: None,
    };
    let info2 = h
        .manager
        .start_with_options(
            config,
            h._data.path(),
            &h.registry,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            small_limits(),
        )
        .await
        .unwrap();
    assert_ne!(info2.session_id, h.info.session_id);
    assert_ne!(info2.token, old_token);
    let staging2 = h._data.path().join("wifi_staging").join(&info2.session_id);
    assert!(staging2.exists());
    assert_eq!(count_files(&staging2), 0, "sessions must not share staging");
    let addr2: SocketAddr = format!("{}:{}", info2.ip, info2.port).parse().unwrap();
    let headers = vec![("Authorization", format!("Bearer {old_token}"))];
    let (status, _) = raw_request(addr2, "POST", "/manifest", &headers, b"{\"files\":[]}").await;
    assert_eq!(status, 401, "old token rejected by new session");
    let _ = old_addr;
    h.manager.stop().await.unwrap();
}

#[tokio::test]
async fn cleanup_on_shutdown() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("a.jpg", 3)])).await;
    assert_eq!(status, 200);
    let (status, _) = post_upload(&h, "a.jpg", b"abc").await;
    assert_eq!(status, 200);
    assert!(h.staging.exists());
    h.manager.stop().await.unwrap();
    assert!(!h.staging.exists(), "staging wiped on stop");
    let job = h.registry.get(&h.info.job_id).unwrap();
    assert_eq!(job.status, JobStatus::Completed);
    h.manager.stop().await.unwrap();
}

#[tokio::test]
async fn manifest_returns_correct_diff() {
    let h = start_session(
        small_limits(),
        &[("same.jpg", b"abc"), ("different.jpg", b"longer-content")],
    )
    .await;
    let (status, body) = post_manifest(
        &h,
        json!([
            manifest_entry("same.jpg", 3),
            manifest_entry("different.jpg", 3),
            manifest_entry("new.jpg", 5)
        ]),
    )
    .await;
    assert_eq!(status, 200);
    let diff: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(diff["unchanged"], json!(["same.jpg"]));
    assert_eq!(diff["changed"], json!(["different.jpg"]));
    assert_eq!(diff["missing"], json!(["new.jpg"]));
}

#[tokio::test]
async fn upload_publishes_via_engine() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("DCIM/photo.jpg", 5)])).await;
    assert_eq!(status, 200);
    let (status, body) = post_upload(&h, "DCIM/photo.jpg", b"hello").await;
    assert_eq!(status, 200);
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["bytesReceived"], json!(5));
    assert!(
        result["hash"].as_str().unwrap().len() == 128,
        "blake2b-512 hex"
    );
    let staged = h.staging.join("DCIM/photo.jpg");
    assert_eq!(std::fs::read(&staged).unwrap(), b"hello");
    assert_eq!(count_files(&h.staging), 1, "no leftover temp files");
}

#[tokio::test]
async fn retry_resume_after_disconnect() {
    // PLAN §11: no chunked resumption. This tests clean abort + full re-upload.
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("clip.mp4", 4096)])).await;
    assert_eq!(status, 200);
    let partial = [vec![1u8; 1024]];
    let result = chunked_upload(&h, "clip.mp4", &partial, Duration::ZERO, false).await;
    assert!(result.is_none());

    // Abort cleanup is asynchronous: the pump reports a body read error, the
    // engine sees EOF, fails size verification, drops its temp file, and the
    // handler then releases quota and records the failure. Poll for the real
    // condition. Requiring the recorded failure first also guarantees the
    // aborted handler has finished before the re-upload writes the same path.
    let mut aborted = false;
    for _ in 0..150 {
        let job = h.registry.get(&h.info.job_id).unwrap();
        let message = job.progress.message.as_deref().unwrap_or("");
        if message.contains("1 failed") && count_files(&h.staging) == 0 {
            aborted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(aborted, "interrupted upload must abort and leave nothing");

    let (status, _) = post_upload(&h, "clip.mp4", &[2u8; 4096]).await;
    assert_eq!(
        status, 200,
        "full re-upload succeeds and quota was released"
    );
    assert_eq!(
        std::fs::read(h.staging.join("clip.mp4")).unwrap().len(),
        4096
    );
}

#[tokio::test]
async fn second_device_rejected_single_session() {
    let h = start_session(small_limits(), &[]).await;
    let config = WifiConfig {
        destination_root: h._dest.path().to_string_lossy().to_string(),
        bind_all: false,
        ttl_secs: None,
    };
    let err = h
        .manager
        .start_with_options(
            config,
            h._data.path(),
            &h.registry,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            small_limits(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, "WIFI_AUTH_FAILED");
}

#[tokio::test]
async fn session_quota_exhausted() {
    let mut limits = small_limits();
    limits.max_file_bytes = 1024;
    limits.max_session_bytes = 2048;
    let h = start_session(limits, &[]).await;
    let (status, _) = post_manifest(
        &h,
        json!([
            manifest_entry("a.bin", 1024),
            manifest_entry("b.bin", 1024),
            manifest_entry("c.bin", 1024)
        ]),
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = post_upload(&h, "a.bin", &[1u8; 1024]).await;
    assert_eq!(status, 200);
    let (status, _) = post_upload(&h, "b.bin", &[2u8; 1024]).await;
    assert_eq!(status, 200);
    let (status, body) = post_upload(&h, "c.bin", &[3u8; 1024]).await;
    assert_eq!(status, 507);
    assert!(String::from_utf8_lossy(&body).contains("WIFI_SESSION_QUOTA"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_quota_reservations() {
    let mut limits = small_limits();
    limits.max_file_bytes = 2048;
    limits.max_session_bytes = 2048;
    let h = start_session(limits, &[]).await;
    let (status, _) = post_manifest(
        &h,
        json!([manifest_entry("x.bin", 1536), manifest_entry("y.bin", 1536)]),
    )
    .await;
    assert_eq!(status, 200);
    let addr = h.addr;
    let token = h.info.token.clone();
    let slow = tokio::spawn(async move {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let head = format!(
            "POST /upload HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nX-SyncUp-Rel-Path: x.bin\r\nTransfer-Encoding: chunked\r\n\r\n"
        );
        let _ = stream.write_all(head.as_bytes()).await;
        let chunk = [1u8; 1536];
        let _ = stream
            .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
            .await;
        let _ = stream.write_all(&chunk).await;
        let _ = stream.write_all(b"\r\n").await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = stream.write_all(b"0\r\n\r\n").await;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        parse_response(&response).0
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let (status, _) = post_upload(&h, "y.bin", &[2u8; 1536]).await;
    let first = slow.await.unwrap();
    let mut statuses = [first, status];
    statuses.sort_unstable();
    assert_eq!(
        statuses,
        [200, 507],
        "exactly one concurrent upload may admit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_releases_quota_and_cleans() {
    let mut limits = small_limits();
    limits.max_file_bytes = 4096;
    limits.max_session_bytes = 4096;
    let h = start_session(limits, &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("slow.bin", 4096)])).await;
    assert_eq!(status, 200);
    let addr = h.addr;
    let token = h.info.token.clone();
    let uploader = tokio::spawn(async move {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let head = format!(
            "POST /upload HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nAuthorization: Bearer {token}\r\nX-SyncUp-Rel-Path: slow.bin\r\nTransfer-Encoding: chunked\r\n\r\n"
        );
        let _ = stream.write_all(head.as_bytes()).await;
        for _ in 0..4 {
            let chunk = [5u8; 1024];
            let _ = stream
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await;
            let _ = stream.write_all(&chunk).await;
            let _ = stream.write_all(b"\r\n").await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let _ = stream.write_all(b"0\r\n\r\n").await;
        let mut response = Vec::new();
        let _ = stream.read_to_end(&mut response).await;
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    h.registry.cancel(&h.info.job_id).unwrap();
    let _ = uploader.await;

    // Cancellation drives the same full-stop watcher as expiry. Poll for the
    // real terminal condition rather than assuming a fixed sleep is enough.
    let mut stopped = false;
    for _ in 0..150 {
        let job = h.registry.get(&h.info.job_id).unwrap();
        if job.status == JobStatus::Cancelled && !h.staging.exists() {
            stopped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(stopped, "cancelled session must stop and wipe staging");

    let config = WifiConfig {
        destination_root: h._dest.path().to_string_lossy().to_string(),
        bind_all: false,
        ttl_secs: None,
    };
    let info2 = h
        .manager
        .start_with_options(
            config,
            h._data.path(),
            &h.registry,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            small_limits(),
        )
        .await
        .unwrap();
    let addr2: SocketAddr = format!("{}:{}", info2.ip, info2.port).parse().unwrap();
    let staging2 = h._data.path().join("wifi_staging").join(&info2.session_id);
    let mut headers = vec![("Authorization", format!("Bearer {}", info2.token))];
    headers.push(("Content-Type", "application/json".to_string()));
    let body = serde_json::to_vec(&json!({"files":[manifest_entry("fresh.bin", 4096)]})).unwrap();
    let (status, _) = raw_request(addr2, "POST", "/manifest", &headers, &body).await;
    assert_eq!(status, 200);
    let mut headers = vec![("Authorization", format!("Bearer {}", info2.token))];
    headers.push(("X-SyncUp-Rel-Path", "fresh.bin".to_string()));
    let (status, _) = raw_request(addr2, "POST", "/upload", &headers, &[6u8; 4096]).await;
    assert_eq!(status, 200, "new session starts with fresh quota");
    assert!(staging2.join("fresh.bin").exists());
    h.manager.stop().await.unwrap();
}

#[tokio::test]
async fn missing_content_length_handled() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("chunked.bin", 2048)])).await;
    assert_eq!(status, 200);
    let chunks = [vec![3u8; 1024], vec![4u8; 1024]];
    let (status, _) = chunked_upload(&h, "chunked.bin", &chunks, Duration::ZERO, true)
        .await
        .unwrap();
    assert_eq!(
        status, 200,
        "chunked upload without Content-Length accepted"
    );
    assert_eq!(
        std::fs::read(h.staging.join("chunked.bin")).unwrap().len(),
        2048
    );
}

#[tokio::test]
async fn incorrect_content_length_rejected() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("liar.bin", 100)])).await;
    assert_eq!(status, 200);
    let mut headers = auth_headers(&h.info.token);
    headers.push(("X-SyncUp-Rel-Path", "liar.bin".to_string()));
    headers.push(("Content-Length", "99".to_string()));
    let (status, _) = raw_request(h.addr, "POST", "/upload", &headers, &[1u8; 100]).await;
    assert_eq!(status, 400, "Content-Length mismatch rejected early");
    assert_eq!(count_files(&h.staging), 0);
}

#[tokio::test]
async fn short_body_verify_failed() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_manifest(&h, json!([manifest_entry("short.bin", 2048)])).await;
    assert_eq!(status, 200);
    let chunks = [vec![1u8; 512]];
    let (status, body) = chunked_upload(&h, "short.bin", &chunks, Duration::ZERO, true)
        .await
        .unwrap();
    assert_eq!(status, 400);
    assert!(String::from_utf8_lossy(&body).contains("VERIFY_FAILED"));
    assert_eq!(
        count_files(&h.staging),
        0,
        "temp cleaned after verify failure"
    );
}

#[tokio::test]
async fn upload_not_in_manifest_rejected() {
    let h = start_session(small_limits(), &[]).await;
    let (status, _) = post_upload(&h, "unlisted.jpg", b"abc").await;
    assert_eq!(status, 400);
    assert_eq!(count_files(&h.staging), 0);
}

#[tokio::test]
#[ignore = "requires a real Android phone on the same LAN; run with --ignored --nocapture"]
async fn lan_upload_hardware() {
    let data = tempfile::tempdir().unwrap();
    let dest = tempfile::tempdir().unwrap();
    let manager = WifiSessionManager::new();
    let registry = JobRegistry::new(None);
    let config = WifiConfig {
        destination_root: dest.path().to_string_lossy().to_string(),
        bind_all: false,
        ttl_secs: Some(300),
    };
    let info = manager.start(config, data.path(), &registry).await.unwrap();
    println!("OPEN THIS URL ON THE PHONE: {}", info.url);
    println!("SESSION TOKEN (type into the page): {}", info.token);
    println!("Waiting 120s for phone uploads...");
    tokio::time::sleep(Duration::from_secs(120)).await;
    let staging = data.path().join("wifi_staging").join(&info.session_id);
    let count = count_files(&staging);
    println!("STAGED FILE COUNT: {count}");
    assert!(count >= 1, "expected at least one uploaded file");
    manager.stop().await.unwrap();
    assert!(!staging.exists(), "staging wiped after stop");
}
