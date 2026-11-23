//! Phase 6 Wi-Fi backup service: scoped LAN HTTP upload server (PLAN.md §7).
//!
//! # Security model (honest)
//! HTTP + Bearer token, NO TLS. Tokens and traffic are observable to any device
//! monitoring the LAN; this is documented residual risk, not a secure channel.
//! One 256-bit CSPRNG token per session; ANY device holding the token is treated
//! as the session device. There is no IP/User-Agent binding: those are spoofable
//! and cause false positives. The token never appears in URLs, query parameters,
//! or logs (`SessionToken` Debug is redacted).
//!
//! # Streaming bridge (async body -> sync engine)
//! The Phase 4 engine consumes `Source::Stream { reader: Box<dyn Read + Send> }`
//! and reads to EOF inside its own `spawn_blocking` worker, verifying
//! `expected_size` after EOF. Axum bodies are async. We bridge with a bounded
//! `tokio::io::duplex(1 MiB)`: an async pump task forwards body frames into the
//! writer while the engine reads the other end via `tokio_util::io::SyncIoBridge`.
//! Blocking reads therefore happen only on the engine's blocking worker, never
//! on Tokio async workers. The pump enforces the per-file cap AND the declared
//! manifest size incrementally against ACTUAL bytes (Content-Length is only an
//! early-rejection hint and may be absent). Memory is bounded by the duplex
//! buffer plus the engine's 64 KiB chunk.
//!
//! # Quota
//! Declared bytes are reserved atomically (mutex) before admission; committed
//! plus in-flight bytes may never exceed the session cap. Successful uploads
//! retain quota for the session lifetime; failures/cancellation release the
//! reservation and remove staged remnants.
//!
//! # Phase 6 scope and limitations
//! Uploads land in per-session staging ONLY; final destination processing and
//! processed-state DB writes are Phase 9. Manifest diff is existence + size
//! against `destination_root` (read-only). mtime comparison needs the DB
//! integration and is deferred; `mtime_ns` is accepted but not yet used.
//! No chunked resumption (PLAN §11): interrupted files must be re-uploaded.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use http_body_util::BodyExt;
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio_util::io::SyncIoBridge;
use tokio_util::sync::CancellationToken;

use crate::domain::conflict::ConflictMode;
use crate::ipc::error::{
    IpcError, CANCELLED, DISK_FULL, PERMISSION_DENIED, VALIDATION_ERROR, VERIFY_FAILED,
    WIFI_SESSION_QUOTA,
};
use crate::ipc::models::{
    WifiConfig, WifiManifestEntry, WifiManifestRequest, WifiManifestResponse, WifiSessionInfo,
    WifiUploadResult,
};
use crate::services::fileops::{
    copy_verified_with_root, Destination, Published, Source, VerifyMode,
};
use crate::services::jobs::{JobHandle, JobKind, JobProgress, JobRegistry, JobStatus};

const DEFAULT_MAX_FILE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const DEFAULT_MAX_SESSION_BYTES: u64 = 50 * 1024 * 1024 * 1024;
const DEFAULT_TTL: Duration = Duration::from_secs(30 * 60);
const DUPLEX_BUFFER: usize = 1024 * 1024;
const MANIFEST_MAX_ENTRIES: usize = 100_000;
const MAX_REL_PATH_LEN: usize = 4096;
const STAGING_DIR_NAME: &str = "wifi_staging";
const ORPHAN_MAX_AGE: Duration = Duration::from_secs(60 * 60);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// Tunable limits. Production uses [`WifiLimits::default`]; tests inject small
/// values through [`WifiSessionManager::start_with_options`].
#[derive(Debug, Clone)]
pub struct WifiLimits {
    pub max_file_bytes: u64,
    pub max_session_bytes: u64,
    pub rate_per_sec: f64,
    pub burst: u32,
    pub max_concurrent_uploads: usize,
    pub ttl: Duration,
}

impl Default for WifiLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_session_bytes: DEFAULT_MAX_SESSION_BYTES,
            rate_per_sec: 10.0,
            burst: 5,
            max_concurrent_uploads: 2,
            ttl: DEFAULT_TTL,
        }
    }
}

/// Session token wrapper whose Debug never reveals the secret.
#[derive(Clone)]
struct SessionToken(String);

impl std::fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionToken([REDACTED])")
    }
}

#[derive(Debug, Default)]
struct Quota {
    committed: u64,
    in_flight: u64,
}

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Debug, Default)]
struct Counts {
    bytes: u64,
    ok: u64,
    failed: u64,
}

struct Session {
    session_id: String,
    token: SessionToken,
    authority: String,
    origin: String,
    staging_dir: PathBuf,
    destination_root: PathBuf,
    limits: WifiLimits,
    expires_at: Instant,
    cancel: CancellationToken,
    job: JobHandle,
    quota: Mutex<Quota>,
    bucket: Mutex<Bucket>,
    manifest: Mutex<HashMap<String, WifiManifestEntry>>,
    uploads: Semaphore,
    counts: Mutex<Counts>,
    serve_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Managed Tauri state holding at most one active Wi-Fi session per process.
#[derive(Clone, Default)]
pub struct WifiSessionManager {
    slot: Arc<tokio::sync::Mutex<Option<Arc<Session>>>>,
}

impl WifiSessionManager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Production entry point: detects the default-route LAN IP and applies
    /// default limits. `bind_all` only changes the socket bind address; the
    /// advertised URL always uses the detected LAN IP.
    pub async fn start(
        &self,
        config: WifiConfig,
        data_dir: &Path,
        jobs: &JobRegistry,
    ) -> Result<WifiSessionInfo, IpcError> {
        let lan_ip = tokio::task::spawn_blocking(detect_lan_ip)
            .await
            .map_err(|e| IpcError::io(&format!("network detection task failed: {e}")))??;
        let socket_ip = if config.bind_all {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            lan_ip
        };
        let mut limits = WifiLimits::default();
        if let Some(secs) = config.ttl_secs {
            limits.ttl = Duration::from_secs(secs);
        }
        self.start_inner(config, data_dir, jobs, socket_ip, lan_ip, limits)
            .await
    }

    /// Test seam: explicit bind address and limits (mirrors the
    /// `run_backup_with_root` precedent). Production code uses [`Self::start`].
    pub async fn start_with_options(
        &self,
        config: WifiConfig,
        data_dir: &Path,
        jobs: &JobRegistry,
        bind_ip: IpAddr,
        limits: WifiLimits,
    ) -> Result<WifiSessionInfo, IpcError> {
        self.start_inner(config, data_dir, jobs, bind_ip, bind_ip, limits)
            .await
    }

    /// Stop the active session (idempotent). Explicit stop finishes the job as
    /// Completed; expiry/cancellation paths finish it as Cancelled.
    pub async fn stop(&self) -> Result<(), IpcError> {
        self.stop_session(None, JobStatus::Completed).await
    }

    async fn start_inner(
        &self,
        config: WifiConfig,
        data_dir: &Path,
        jobs: &JobRegistry,
        socket_ip: IpAddr,
        advertise_ip: IpAddr,
        limits: WifiLimits,
    ) -> Result<WifiSessionInfo, IpcError> {
        let mut slot = self.slot.lock().await;
        if slot.is_some() {
            return Err(IpcError::wifi_auth_failed(
                "a Wi-Fi session is already active; stop it before starting another",
            ));
        }
        if limits.max_concurrent_uploads == 0 {
            return Err(IpcError::validation("max_concurrent_uploads must be >= 1"));
        }
        let dest = PathBuf::from(&config.destination_root);
        if !dest.is_absolute() {
            return Err(IpcError::validation(
                "destination_root must be an absolute path",
            ));
        }

        let session_id = uuid::Uuid::new_v4().to_string();
        let staging_root = data_dir.join(STAGING_DIR_NAME);
        let sid = session_id.clone();
        let (destination_root, staging_dir) =
            tokio::task::spawn_blocking(move || -> Result<(PathBuf, PathBuf), IpcError> {
                let destination_root = std::fs::canonicalize(&dest).map_err(|_| {
                    IpcError::validation("destination_root must be an existing directory")
                })?;
                std::fs::create_dir_all(&staging_root)
                    .map_err(|e| IpcError::io(&format!("staging root unavailable: {e}")))?;
                set_private_permissions(&staging_root)?;
                cleanup_orphan_sessions(&staging_root);
                let staging_dir = staging_root.join(&sid);
                std::fs::create_dir_all(&staging_dir)
                    .map_err(|e| IpcError::io(&format!("staging directory unavailable: {e}")))?;
                set_private_permissions(&staging_dir)?;
                Ok((destination_root, staging_dir))
            })
            .await
            .map_err(|e| IpcError::io(&format!("staging setup task failed: {e}")))??;

        let mut token_bytes = [0u8; 32];
        getrandom::fill(&mut token_bytes)
            .map_err(|e| IpcError::io(&format!("CSPRNG unavailable: {e}")))?;
        let token = hex::encode(token_bytes);

        let listener = tokio::net::TcpListener::bind(SocketAddr::new(socket_ip, 0))
            .await
            .map_err(|e| IpcError::io(&format!("cannot bind Wi-Fi listener: {e}")))?;
        let local = listener
            .local_addr()
            .map_err(|e| IpcError::io(&format!("listener address unavailable: {e}")))?;
        let authority = format!("{advertise_ip}:{}", local.port());
        let origin = format!("http://{authority}");
        let url = format!("{origin}/");

        let (job_id, job, cancel) = jobs.register(JobKind::Wifi);
        let session = Arc::new(Session {
            session_id: session_id.clone(),
            token: SessionToken(token.clone()),
            authority,
            origin,
            staging_dir,
            destination_root,
            limits: limits.clone(),
            expires_at: Instant::now() + limits.ttl,
            cancel: cancel.clone(),
            job: job.clone(),
            quota: Mutex::new(Quota::default()),
            bucket: Mutex::new(Bucket {
                tokens: f64::from(limits.burst),
                last: Instant::now(),
            }),
            manifest: Mutex::new(HashMap::new()),
            uploads: Semaphore::new(limits.max_concurrent_uploads),
            counts: Mutex::new(Counts::default()),
            serve_task: Mutex::new(None),
        });
        job.update_progress(JobProgress {
            current: 0,
            total: Some(limits.max_session_bytes),
            message: Some("session active".to_string()),
        });

        let app = build_router(session.clone());
        let shutdown = cancel.clone();
        let serve_task = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async move { shutdown.cancelled().await })
                .await
            {
                tracing::warn!(error = %e, "wi-fi server exited with error");
            }
        });
        *lock_mutex(&session.serve_task) = Some(serve_task);

        let watcher_manager = self.clone();
        let watcher_session = session.clone();
        tokio::spawn(async move {
            let sid = watcher_session.session_id.clone();
            tokio::select! {
                () = watcher_session.cancel.cancelled() => {
                    let _ = watcher_manager.stop_session(Some(&sid), JobStatus::Cancelled).await;
                }
                () = tokio::time::sleep(watcher_session.limits.ttl) => {
                    tracing::info!(session_id = %sid, "wi-fi session expired");
                    let _ = watcher_manager.stop_session(Some(&sid), JobStatus::Cancelled).await;
                }
            }
        });

        *slot = Some(session);
        Ok(WifiSessionInfo {
            url,
            session_id,
            ip: advertise_ip.to_string(),
            port: local.port(),
            token,
            job_id,
        })
    }

    async fn stop_session(&self, only_id: Option<&str>, status: JobStatus) -> Result<(), IpcError> {
        let session = {
            let mut slot = self.slot.lock().await;
            let matches = match (slot.as_ref(), only_id) {
                (Some(s), Some(id)) => s.session_id == id,
                (Some(_), None) => true,
                (None, _) => false,
            };
            if matches {
                slot.take()
            } else {
                None
            }
        };
        let Some(session) = session else {
            return Ok(());
        };
        session.cancel.cancel();
        let mut task = lock_mutex(&session.serve_task).take();
        if let Some(task) = task.as_mut() {
            let _ = tokio::time::timeout(SHUTDOWN_GRACE, &mut *task).await;
            task.abort();
        }
        let staging = session.staging_dir.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if staging.exists() {
                if let Err(e) = std::fs::remove_dir_all(&staging) {
                    tracing::warn!(error = %e, "failed to remove wi-fi staging directory");
                }
            }
        })
        .await;
        session.job.finish(status, None);
        Ok(())
    }
}

fn build_router(session: Arc<Session>) -> Router {
    Router::new()
        .route("/", get(index_page))
        .route("/manifest", post(manifest_handler))
        .route(
            "/upload",
            post(upload_handler).layer(DefaultBodyLimit::disable()),
        )
        .layer(middleware::from_fn_with_state(session.clone(), guard))
        .with_state(session)
}

async fn guard(State(session): State<Arc<Session>>, req: Request, next: Next) -> Response {
    if session.cancel.is_cancelled() || Instant::now() >= session.expires_at {
        return error_response(
            StatusCode::UNAUTHORIZED,
            IpcError::wifi_auth_failed("session is not active"),
        );
    }
    let headers = req.headers();
    let host_ok = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| h == session.authority)
        .unwrap_or(false);
    if !host_ok {
        return error_response(
            StatusCode::FORBIDDEN,
            IpcError::wifi_auth_failed("Host header mismatch"),
        );
    }
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|v| v.to_str().ok())
    {
        if origin != session.origin {
            return error_response(
                StatusCode::FORBIDDEN,
                IpcError::wifi_auth_failed("Origin header mismatch"),
            );
        }
    }
    let path = req.uri().path();
    if path == "/manifest" || path == "/upload" {
        if !take_rate_token(
            &session.bucket,
            session.limits.rate_per_sec,
            session.limits.burst,
        ) {
            return error_response(
                StatusCode::TOO_MANY_REQUESTS,
                IpcError::wifi_rate_limit("request rate exceeded"),
            );
        }
        let auth_ok = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|presented| bool::from(presented.as_bytes().ct_eq(session.token.0.as_bytes())))
            .unwrap_or(false);
        if !auth_ok {
            return error_response(
                StatusCode::UNAUTHORIZED,
                IpcError::wifi_auth_failed("missing or invalid session token"),
            );
        }
    }
    next.run(req).await
}

async fn manifest_handler(
    State(session): State<Arc<Session>>,
    Json(req): Json<WifiManifestRequest>,
) -> Response {
    if req.files.len() > MANIFEST_MAX_ENTRIES {
        return error_response(
            StatusCode::BAD_REQUEST,
            IpcError::validation("too many manifest entries"),
        );
    }
    let mut allowlist = HashMap::new();
    for entry in req.files {
        let Some(rel) = sanitize_rel_path(&entry.rel_path) else {
            return error_response(
                StatusCode::BAD_REQUEST,
                IpcError::validation("invalid rel_path in manifest"),
            );
        };
        if entry.size > session.limits.max_file_bytes {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                IpcError::validation("manifest entry exceeds the per-file cap"),
            );
        }
        let key = rel.to_string_lossy().to_string();
        allowlist.insert(
            key.clone(),
            WifiManifestEntry {
                rel_path: key,
                size: entry.size,
                mtime_ns: entry.mtime_ns,
            },
        );
    }
    let dest = session.destination_root.clone();
    let entries: Vec<WifiManifestEntry> = allowlist.values().cloned().collect();
    let diff = tokio::task::spawn_blocking(move || compute_manifest_diff(&dest, &entries)).await;
    let response = match diff {
        Ok(d) => d,
        Err(e) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                IpcError::io(&format!("manifest diff task failed: {e}")),
            );
        }
    };
    *lock_mutex(&session.manifest) = allowlist;
    (StatusCode::OK, Json(response)).into_response()
}

struct UploadFailure {
    status: StatusCode,
    err: IpcError,
}

async fn upload_handler(State(session): State<Arc<Session>>, req: Request) -> Response {
    if session.cancel.is_cancelled() {
        return error_response(
            StatusCode::UNAUTHORIZED,
            IpcError::wifi_auth_failed("session stopped"),
        );
    }
    let headers = req.headers().clone();
    let rel_header = match headers
        .get("x-syncup-rel-path")
        .and_then(|v| v.to_str().ok())
    {
        Some(v) => v.to_string(),
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                IpcError::validation("missing X-SyncUp-Rel-Path header"),
            );
        }
    };
    let Some(rel) = sanitize_rel_path(&rel_header) else {
        return error_response(
            StatusCode::BAD_REQUEST,
            IpcError::validation("invalid upload path"),
        );
    };
    let key = rel.to_string_lossy().to_string();
    let entry = lock_mutex(&session.manifest).get(&key).cloned();
    let Some(entry) = entry else {
        return error_response(
            StatusCode::BAD_REQUEST,
            IpcError::validation("upload path is not present in the session manifest"),
        );
    };
    let expected = entry.size;
    if expected > session.limits.max_file_bytes {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            IpcError::validation("file exceeds the per-file cap"),
        );
    }
    if let Some(cl) = headers
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        if cl > session.limits.max_file_bytes {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                IpcError::validation("request exceeds the per-file cap"),
            );
        }
        if cl != expected {
            return error_response(
                StatusCode::BAD_REQUEST,
                IpcError::validation("Content-Length does not match the manifest size"),
            );
        }
    }

    let permit = tokio::select! {
        () = session.cancel.cancelled() => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                IpcError::wifi_auth_failed("session stopped"),
            );
        }
        permit = session.uploads.acquire() => match permit {
            Ok(p) => p,
            Err(_) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    IpcError::io("upload concurrency limiter closed"),
                );
            }
        },
    };
    if !reserve_quota(&session.quota, expected, session.limits.max_session_bytes) {
        drop(permit);
        return error_response(
            StatusCode::INSUFFICIENT_STORAGE,
            IpcError::wifi_session_quota("session upload quota exhausted"),
        );
    }

    let result = stream_upload(session.clone(), req.into_body(), &rel, expected).await;
    drop(permit);
    match result {
        Ok(published) => {
            commit_quota(&session.quota, expected);
            let progress = {
                let mut counts = lock_mutex(&session.counts);
                counts.bytes = counts.bytes.saturating_add(published.bytes_written);
                counts.ok += 1;
                JobProgress {
                    current: counts.bytes,
                    total: Some(session.limits.max_session_bytes),
                    message: Some(format!("{} uploaded, {} failed", counts.ok, counts.failed)),
                }
            };
            session.job.update_progress(progress);
            let hash = published.hash.map(hex::encode);
            (
                StatusCode::OK,
                Json(WifiUploadResult {
                    rel_path: key,
                    bytes_received: published.bytes_written,
                    hash,
                }),
            )
                .into_response()
        }
        Err(failure) => {
            release_quota(&session.quota, expected);
            let progress = {
                let mut counts = lock_mutex(&session.counts);
                counts.failed += 1;
                JobProgress {
                    current: counts.bytes,
                    total: Some(session.limits.max_session_bytes),
                    message: Some(format!("{} uploaded, {} failed", counts.ok, counts.failed)),
                }
            };
            session.job.update_progress(progress);
            let final_path = session.staging_dir.join(&rel);
            let _ = tokio::task::spawn_blocking(move || {
                if final_path.exists() {
                    let _ = std::fs::remove_file(&final_path);
                }
            })
            .await;
            error_response(failure.status, failure.err)
        }
    }
}

async fn stream_upload(
    session: Arc<Session>,
    body: Body,
    rel: &Path,
    expected: u64,
) -> Result<Published, UploadFailure> {
    let (writer, reader) = tokio::io::duplex(DUPLEX_BUFFER);
    let abort: Arc<Mutex<Option<&'static str>>> = Arc::new(Mutex::new(None));
    let pump_abort = abort.clone();
    let pump_cancel = session.cancel.clone();
    let limit = expected.min(session.limits.max_file_bytes);
    let pump = tokio::spawn(async move {
        let mut body = body;
        let mut writer = writer;
        let mut total: u64 = 0;
        loop {
            let frame = tokio::select! {
                () = pump_cancel.cancelled() => break,
                frame = body.frame() => match frame {
                    Some(f) => f,
                    None => break,
                },
            };
            let data = match frame {
                Ok(f) => match f.into_data() {
                    Ok(d) => d,
                    Err(_) => continue,
                },
                Err(_) => {
                    *lock_mutex(&pump_abort) = Some("body read error");
                    break;
                }
            };
            total = total.saturating_add(data.len() as u64);
            if total > limit {
                *lock_mutex(&pump_abort) = Some("cap");
                break;
            }
            let write = tokio::select! {
                () = pump_cancel.cancelled() => break,
                res = writer.write_all(&data) => res,
            };
            if write.is_err() {
                break;
            }
        }
    });

    let source = Source::Stream {
        reader: Box::new(SyncIoBridge::new(reader)),
        size_hint: Some(expected),
    };
    let dest = Destination {
        final_path: session.staging_dir.join(rel),
        conflict_mode: ConflictMode::Overwrite,
        expected_size: Some(expected),
        verify: VerifyMode::SizeAndHash,
    };
    let staging = session.staging_dir.clone();
    let copy = copy_verified_with_root(source, dest, session.cancel.clone(), Some(&staging)).await;
    let _ = pump.await;
    let abort_reason = lock_mutex(&abort).take();

    if session.cancel.is_cancelled() {
        return Err(UploadFailure {
            status: StatusCode::BAD_REQUEST,
            err: IpcError::cancelled("upload cancelled"),
        });
    }
    if let Some(reason) = abort_reason {
        let status = if reason == "cap" {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        };
        return Err(UploadFailure {
            status,
            err: IpcError::validation(&format!("upload aborted: {reason}")),
        });
    }
    match copy {
        Ok(published) => Ok(published),
        Err(e) => {
            let ipc: IpcError = e.into();
            let status = http_status_for_error(&ipc);
            Err(UploadFailure { status, err: ipc })
        }
    }
}

async fn index_page() -> Response {
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        INDEX_HTML,
    )
        .into_response()
}

fn error_response(status: StatusCode, err: IpcError) -> Response {
    (status, Json(err)).into_response()
}

fn http_status_for_error(err: &IpcError) -> StatusCode {
    match err.code.as_str() {
        VALIDATION_ERROR | VERIFY_FAILED | CANCELLED => StatusCode::BAD_REQUEST,
        DISK_FULL | WIFI_SESSION_QUOTA => StatusCode::INSUFFICIENT_STORAGE,
        PERMISSION_DENIED => StatusCode::FORBIDDEN,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn compute_manifest_diff(dest: &Path, entries: &[WifiManifestEntry]) -> WifiManifestResponse {
    let mut missing = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = Vec::new();
    for entry in entries {
        let path = dest.join(&entry.rel_path);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => {
                if meta.len() == entry.size {
                    unchanged.push(entry.rel_path.clone());
                } else {
                    changed.push(entry.rel_path.clone());
                }
            }
            _ => missing.push(entry.rel_path.clone()),
        }
    }
    missing.sort();
    changed.sort();
    unchanged.sort();
    WifiManifestResponse {
        missing,
        changed,
        unchanged,
    }
}

fn sanitize_rel_path(input: &str) -> Option<PathBuf> {
    if input.is_empty() || input.len() > MAX_REL_PATH_LEN {
        return None;
    }
    if input.starts_with('/') || input.contains('\\') || input.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in input.split('/') {
        if comp.is_empty() || comp == "." || comp == ".." {
            return None;
        }
        if comp.chars().any(|c| c.is_ascii_control()) {
            return None;
        }
        let base = comp.split('.').next().unwrap_or(comp).to_ascii_uppercase();
        if is_windows_reserved(&base) {
            return None;
        }
        out.push(comp);
    }
    Some(out)
}

fn is_windows_reserved(base: &str) -> bool {
    matches!(
        base,
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn reserve_quota(quota: &Mutex<Quota>, amount: u64, cap: u64) -> bool {
    let mut q = lock_mutex(quota);
    let total = q.committed.saturating_add(q.in_flight);
    if total.saturating_add(amount) > cap {
        return false;
    }
    q.in_flight = q.in_flight.saturating_add(amount);
    true
}

fn release_quota(quota: &Mutex<Quota>, amount: u64) {
    let mut q = lock_mutex(quota);
    q.in_flight = q.in_flight.saturating_sub(amount);
}

fn commit_quota(quota: &Mutex<Quota>, amount: u64) {
    let mut q = lock_mutex(quota);
    q.in_flight = q.in_flight.saturating_sub(amount);
    q.committed = q.committed.saturating_add(amount);
}

fn take_rate_token(bucket: &Mutex<Bucket>, rate: f64, burst: u32) -> bool {
    let mut b = lock_mutex(bucket);
    let now = Instant::now();
    let elapsed = now.duration_since(b.last).as_secs_f64();
    b.last = now;
    b.tokens = (b.tokens + elapsed * rate).min(f64::from(burst));
    if b.tokens >= 1.0 {
        b.tokens -= 1.0;
        true
    } else {
        false
    }
}

fn detect_lan_ip() -> Result<IpAddr, IpcError> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|e| IpcError::io(&format!("cannot probe default route: {e}")))?;
    // UDP connect sends no packets; it only selects the default route.
    socket.connect("8.8.8.8:80").map_err(|_| {
        IpcError::validation(
            "no usable network route detected; connect to a LAN before starting a Wi-Fi session",
        )
    })?;
    let local = socket
        .local_addr()
        .map_err(|e| IpcError::io(&format!("local address unavailable: {e}")))?
        .ip();
    if local.is_loopback() || local.is_unspecified() {
        return Err(IpcError::validation(
            "default-route address is loopback or unspecified; refusing to use it for Wi-Fi",
        ));
    }
    Ok(local)
}

#[cfg(unix)]
fn set_private_permissions(dir: &Path) -> Result<(), IpcError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| IpcError::io(&format!("cannot restrict staging permissions: {e}")))
}

#[cfg(not(unix))]
fn set_private_permissions(_dir: &Path) -> Result<(), IpcError> {
    Ok(())
}

fn cleanup_orphan_sessions(staging_root: &Path) {
    let Ok(entries) = std::fs::read_dir(staging_root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age > ORPHAN_MAX_AGE)
            .unwrap_or(false);
        if path.is_dir() && stale {
            if let Err(e) = std::fs::remove_dir_all(&path) {
                tracing::warn!(error = %e, "failed to remove orphan wi-fi staging directory");
            }
        }
    }
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const INDEX_HTML: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>SyncUp Wi-Fi Upload</title></head>
<body><h1>SyncUp Wi-Fi Upload</h1>
<p>Enter the session token shown on the desktop, choose files, then upload. Not encrypted: use only on a network you trust.</p>
<input id="token" type="password" placeholder="session token" autocomplete="off">
<input id="files" type="file" multiple>
<button id="go">Upload</button><pre id="out"></pre>
<script>
const out=document.getElementById('out');
const log=(m)=>{out.textContent+=m+'\n';};
document.getElementById('go').addEventListener('click',async()=>{
 const token=document.getElementById('token').value.trim();
 const files=[...document.getElementById('files').files];
 if(!token||files.length===0){log('token and files required');return;}
 const h={'Authorization':'Bearer '+token,'Content-Type':'application/json'};
 const manifest={files:files.map(f=>({relPath:f.name,size:f.size,mtimeNs:Math.round(f.lastModified*1e6)}))};
 const mr=await fetch('/manifest',{method:'POST',headers:h,body:JSON.stringify(manifest)});
 if(!mr.ok){log('manifest failed: '+mr.status);return;}
 const diff=await mr.json();
 const wanted=new Set([...diff.missing,...diff.changed]);
 for(const f of files){
  if(!wanted.has(f.name)){log('skip '+f.name);continue;}
  const r=await fetch('/upload',{method:'POST',headers:{'Authorization':'Bearer '+token,'X-SyncUp-Rel-Path':f.name},body:f});
  log(f.name+': '+r.status);
 }
 log('done');
});
</script></body></html>
"#;
