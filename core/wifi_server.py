"""FastAPI application and Uvicorn lifecycle for Wi-Fi Backup.

The phone opens the page served at ``GET /``, announces every candidate file at
``POST /manifest``, uploads only the files the PC asks for at ``POST /upload``,
then calls ``POST /complete``.  The PC runs the normal backup engine against the
staging directory and the phone polls ``GET /status`` for progress.

Threading contract
------------------
* Uvicorn runs in a daemon thread (see :class:`WiFiServer`).
* Every route is a **sync** function, so FastAPI executes it in its worker
  thread pool.  Route code may therefore block on SQLite or disk I/O.
* Telemetry is forwarded through the existing ``progress_queue``.
* UI work is scheduled exclusively through ``post_ui``.  No route ever touches
  a Flet control or calls ``page.update()``.
* ``StateStore`` is opened and closed per request.  SQLite connections must not
  be shared across threads and the thread pool may reuse any worker thread.
"""

from __future__ import annotations

import queue
import shutil
import socket
import threading
import time
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path, PurePosixPath

import uvicorn
from fastapi import FastAPI, File, Form, HTTPException, Request, UploadFile
from fastapi.responses import HTMLResponse, Response

from core.constants import IMAGE_EXT, MEDIA_EXT, VERSION, VIDEO_EXT
from core.engine import run_wifi_backup
from core.fileops import copy_stream_atomic, human_size, is_partial_name
from core.storage import StateStore
from core.telemetry import LEVEL_INFO, LEVEL_ORGANIZE, LEVEL_SUCCESS
from core.wifi_models import (
    ManifestEntry,
    ManifestRequest,
    ManifestResponse,
    MissingFileResponse,
    StatusResponse,
    UploadResponse,
)
from core.wifi_telemetry import WiFiTelemetry

DEFAULT_PORT = 8555
PORT_SCAN_ATTEMPTS = 20
MTIME_TOLERANCE_NS = 2_000_000_000  # 2 seconds, same as core.engine
MAX_REL_PATH_LENGTH = 4096


# ---------------------------------------------------------------------------
# Mobile page (vanilla HTML/CSS/JS, no external dependencies)
# ---------------------------------------------------------------------------

MOBILE_HTML = """<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
  <meta name="theme-color" content="#0b57d0">
  <title>SyncUp Wi-Fi Backup</title>
  <style>
    :root {
      color-scheme: light dark;
      --bg: #f6f7fb;
      --surface: #ffffff;
      --on-surface: #1b1c1e;
      --muted: #5f6368;
      --primary: #0b57d0;
      --primary-on: #ffffff;
      --success: #137333;
      --error: #b3261e;
      --border: #d9dce3;
      --track: #e3e6ee;
    }
    @media (prefers-color-scheme: dark) {
      :root {
        --bg: #101114;
        --surface: #1b1c1f;
        --on-surface: #e6e8ee;
        --muted: #9aa0a6;
        --primary: #8ab4f8;
        --primary-on: #062e6f;
        --success: #81c995;
        --error: #f2b8b5;
        --border: #33363d;
        --track: #2a2d33;
      }
    }
    * { box-sizing: border-box; -webkit-tap-highlight-color: transparent; }
    body {
      margin: 0;
      padding: max(16px, env(safe-area-inset-top)) 16px
               calc(24px + env(safe-area-inset-bottom));
      background: var(--bg);
      color: var(--on-surface);
      font-family: system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
      line-height: 1.45;
    }
    .wrap { max-width: 540px; margin: 0 auto; display: flex;
            flex-direction: column; gap: 14px; }
    h1 { font-size: 1.35rem; margin: 0 0 4px; }
    .sub { color: var(--muted); font-size: .95rem; margin: 0; }
    .card { background: var(--surface); border: 1px solid var(--border);
            border-radius: 14px; padding: 16px; }
    .btn {
      display: flex; align-items: center; justify-content: center;
      width: 100%; min-height: 52px; padding: 12px 16px; margin-top: 12px;
      border: 0; border-radius: 12px; font: inherit; font-size: 1.05rem;
      font-weight: 600; background: var(--primary); color: var(--primary-on);
      cursor: pointer;
    }
    .btn:disabled { opacity: .55; cursor: not-allowed; }
    .btn.secondary { background: transparent; color: var(--primary);
                     border: 1px solid var(--border); }
    .file-input { position: absolute; width: 1px; height: 1px;
                  opacity: 0; overflow: hidden; }
    .picker {
      display: flex; flex-direction: column; gap: 6px; align-items: center;
      justify-content: center; min-height: 108px; padding: 16px;
      border: 2px dashed var(--border); border-radius: 12px;
      text-align: center; cursor: pointer;
    }
    .picker.disabled { opacity: .55; pointer-events: none; }
    .picker-title { font-size: 1.05rem; font-weight: 600; color: var(--primary); }
    .muted { color: var(--muted); font-size: .86rem; }
    #count { margin: 10px 0 0; }
    #progressWrap { margin-top: 14px; }
    #bar { height: 10px; border-radius: 999px; background: var(--track);
           overflow: hidden; }
    #barFill { height: 100%; width: 0%; background: var(--primary);
               transition: width .2s ease; }
    .status { margin-top: 14px; padding: 12px; border-radius: 10px;
              background: var(--track); font-size: .95rem;
              overflow-wrap: anywhere; }
    .status.ok { color: var(--success); }
    .status.err { color: var(--error); }
    ul.failures { margin: 10px 0 0; padding-left: 18px; color: var(--error);
                  font-size: .85rem; overflow-wrap: anywhere; }
    .hidden { display: none !important; }
  </style>
</head>
<body>
  <div class="wrap">
    <header>
      <h1>SyncUp Wi-Fi Backup</h1>
      <p class="sub">Send photos and videos to your PC over your local network.</p>
    </header>

    <section class="card">
      <label class="picker" id="pickerLabel" for="picker">
        <span class="picker-title" id="pickerTitle">Choose a folder</span>
        <span class="muted" id="pickerHint">Pick DCIM or Pictures so folder names are preserved.</span>
      </label>
      <input class="file-input" id="picker" type="file"
             webkitdirectory directory multiple accept="image/*,video/*">

      <p class="muted" id="count">No files selected.</p>

      <button class="btn" id="startBtn" type="button" disabled>Scan &amp; Upload</button>

      <div id="progressWrap" class="hidden">
        <div id="bar" role="progressbar" aria-valuemin="0" aria-valuemax="100"
             aria-valuenow="0"><div id="barFill"></div></div>
      </div>

      <div class="status" id="status" role="status" aria-live="polite">
        Select a folder to begin.
      </div>

      <ul class="failures hidden" id="failures"></ul>

      <button class="btn secondary hidden" id="resetBtn" type="button">
        Start Over
      </button>
    </section>

    <p class="muted">
      Keep this page open until the PC finishes. Only files that are new or
      changed are uploaded, so running it again is safe.
    </p>
  </div>

  <script>
  (() => {
    "use strict";

    const $ = (id) => document.getElementById(id);
    const picker = $("picker");
    const startBtn = $("startBtn");
    const resetBtn = $("resetBtn");
    const statusEl = $("status");
    const countEl = $("count");
    const failuresEl = $("failures");
    const progressWrap = $("progressWrap");
    const bar = $("bar");
    const barFill = $("barFill");

    const state = {
      files: new Map(),
      missing: [],
      uploaded: 0,
      failed: [],
      busy: false,
      polling: false,
      pollErrors: 0,
      sentBytes: 0,
      totalBytesToSend: 0,
    };

    const supportsDirectory =
      "webkitdirectory" in document.createElement("input");

    if (!supportsDirectory) {
      picker.removeAttribute("webkitdirectory");
      picker.removeAttribute("directory");
      $("pickerTitle").textContent = "Choose photos & videos";
      $("pickerHint").textContent =
        "This browser cannot upload a whole folder. Select files instead.";
    }

    function fmtBytes(value) {
      const n = Number(value) || 0;
      if (n < 1024) return n + " B";
      if (n < 1048576) return (n / 1024).toFixed(1) + " KB";
      if (n < 1073741824) return (n / 1048576).toFixed(1) + " MB";
      return (n / 1073741824).toFixed(2) + " GB";
    }

    function baseName(path) {
      const text = String(path || "");
      const index = text.lastIndexOf("/");
      return index === -1 ? text : text.slice(index + 1);
    }

    function setStatus(text, kind) {
      statusEl.textContent = text;
      statusEl.className = "status" + (kind ? " " + kind : "");
    }

    function setProgress(fraction, label) {
      const pct = Math.max(0, Math.min(1, Number(fraction) || 0)) * 100;
      progressWrap.classList.remove("hidden");
      barFill.style.width = pct.toFixed(1) + "%";
      bar.setAttribute("aria-valuenow", String(Math.round(pct)));
      if (label) countEl.textContent = label;
    }

    function showFailures() {
      failuresEl.textContent = "";
      if (!state.failed.length) {
        failuresEl.classList.add("hidden");
        return;
      }
      failuresEl.classList.remove("hidden");
      for (const failure of state.failed) {
        const item = document.createElement("li");
        item.textContent =
          baseName(failure.rel_path) + ": " + failure.error;
        failuresEl.appendChild(item);
      }
    }

    function setBusy(busy) {
      state.busy = busy;
      picker.disabled = busy;
      startBtn.disabled = busy || state.files.size === 0;
      $("pickerLabel").classList.toggle("disabled", busy);
    }

    function sleep(ms) {
      return new Promise((resolve) => setTimeout(resolve, ms));
    }

    function parseServerError(response, data) {
      const detail = data && data.detail;
      if (Array.isArray(detail)) {
        return detail.map((entry) => entry.msg || String(entry)).join("; ");
      }
      if (typeof detail === "string" && detail) return detail;
      return "The PC returned HTTP " + response.status + ".";
    }

    async function requestJSON(url, method, body) {
      let response;
      try {
        response = await fetch(url, {
          method: method,
          headers: body ? {"Content-Type": "application/json"} : undefined,
          body: body ? JSON.stringify(body) : undefined,
          cache: "no-store",
        });
      } catch (error) {
        throw new Error(
          "Cannot reach the PC. Make sure both devices are on the same Wi-Fi."
        );
      }
      let data = null;
      try { data = await response.json(); } catch (error) { data = null; }
      if (!response.ok) {
        const failure = new Error(parseServerError(response, data));
        failure.status = response.status;
        throw failure;
      }
      return data || {};
    }

    picker.addEventListener("change", () => {
      const chosen = Array.from(picker.files || []);
      state.files.clear();
      let duplicates = 0;

      for (const file of chosen) {
        if (!file || !file.name || file.name.startsWith(".")) continue;
        const raw = (file.webkitRelativePath || file.name)
          .replace(/\\\\/g, "/")
          .replace(/^\\/+/, "");
        const parts = raw.split("/").filter((part) => part && part !== ".");
        if (!parts.length) continue;
        if (parts.some((part) => part === ".." || part.startsWith("."))) {
          continue;
        }
        const rel = parts.join("/");
        if (state.files.has(rel)) {
          duplicates += 1;
          continue;
        }
        state.files.set(rel, file);
      }

      state.missing = [];
      state.uploaded = 0;
      state.failed = [];
      state.sentBytes = 0;
      state.totalBytesToSend = 0;
      state.polling = false;
      state.pollErrors = 0;

      showFailures();
      resetBtn.classList.add("hidden");
      progressWrap.classList.add("hidden");
      barFill.style.width = "0%";

      if (!state.files.size) {
        countEl.textContent = "No usable files selected.";
        setStatus("Choose a folder that contains photos or videos.");
        setBusy(false);
        return;
      }

      let total = 0;
      for (const file of state.files.values()) total += file.size || 0;
      countEl.textContent =
        state.files.size + " file(s), " + fmtBytes(total) +
        (duplicates
          ? " — " + duplicates + " duplicate path(s) ignored"
          : "");
      setStatus("Ready to compare with the PC.");
      setBusy(false);
    });

    startBtn.addEventListener("click", async () => {
      if (state.busy || !state.files.size) return;

      state.failed = [];
      state.uploaded = 0;
      state.sentBytes = 0;
      state.pollErrors = 0;
      showFailures();
      resetBtn.classList.add("hidden");
      setBusy(true);

      try {
        const manifest = [];
        for (const [rel, file] of state.files) {
          manifest.push({
            rel_path: rel,
            size: file.size || 0,
            mtime_ns: Math.max(
              0, Math.round((file.lastModified || 0) * 1e6)
            ),
          });
        }

        setStatus("Comparing with the PC…");
        const diff = await requestJSON("/manifest", "POST", {
          device_id: "wifi-backup",
          files: manifest,
        });

        if (!diff || !Array.isArray(diff.missing)) {
          throw new Error("The PC returned an invalid manifest response.");
        }

        if (diff.missing.length === 0) {
          setProgress(1, state.files.size + " file(s) already backed up");
          setStatus(
            "Everything is already backed up. Nothing to upload.",
            "ok"
          );
          setBusy(false);
          resetBtn.classList.remove("hidden");
          return;
        }

        state.missing = diff.missing;
        state.totalBytesToSend =
          Number(diff.total_bytes) ||
          state.missing.reduce(
            (sum, entry) => sum + (Number(entry.size) || 0), 0
          );

        setProgress(0, "0 of " + state.missing.length + " uploaded");
        setStatus(
          "Uploading " + state.missing.length + " file(s) (" +
          fmtBytes(state.totalBytesToSend) + "). Keep this page open."
        );

        for (let index = 0; index < state.missing.length; index++) {
          const entry = state.missing[index];
          const file = state.files.get(entry.rel_path);

          if (!file) {
            state.failed.push({
              rel_path: entry.rel_path,
              error: "file is no longer available in this browser",
            });
            showFailures();
          } else {
            setStatus(
              "Uploading " + (index + 1) + " of " +
              state.missing.length + ": " + baseName(entry.rel_path)
            );
            try {
              await uploadWithRetry(entry, file, (loaded) => {
                const overall =
                  state.sentBytes +
                  Math.min(loaded, Number(entry.size) || loaded);
                setProgress(
                  state.totalBytesToSend
                    ? overall / state.totalBytesToSend
                    : index / state.missing.length,
                  (index + 1) + " of " + state.missing.length + " uploaded"
                );
              });
              state.uploaded += 1;
            } catch (error) {
              state.failed.push({
                rel_path: entry.rel_path,
                error: String((error && error.message) || error),
              });
              showFailures();
            }
            state.sentBytes += Number(entry.size) || file.size || 0;
          }

          setProgress(
            (index + 1) / state.missing.length,
            state.uploaded + " uploaded, " + state.failed.length + " failed"
          );
        }

        if (state.uploaded === 0) {
          throw new Error(
            "No files could be uploaded. Check the Wi-Fi connection."
          );
        }

        setStatus(
          state.failed.length
            ? state.uploaded + " uploaded, " + state.failed.length +
              " failed. Starting the PC backup…"
            : "Upload complete. Starting the PC backup…"
        );
        await completeWithRetry();

        state.polling = true;
        setStatus("The PC is copying your files…");
        pollStatus();
      } catch (error) {
        state.polling = false;
        setStatus(String((error && error.message) || error), "err");
        setBusy(false);
        resetBtn.classList.remove("hidden");
      }
    });

    function uploadOne(entry, file, onProgress) {
      return new Promise((resolve, reject) => {
        const xhr = new XMLHttpRequest();
        xhr.open("POST", "/upload", true);

        xhr.upload.onprogress = (event) => {
          if (event.lengthComputable && typeof onProgress === "function") {
            onProgress(event.loaded, event.total);
          }
        };

        xhr.onload = () => {
          let data = null;
          try {
            data = xhr.responseText ? JSON.parse(xhr.responseText) : null;
          } catch (error) {
            data = null;
          }
          if (xhr.status >= 200 && xhr.status < 300) {
            resolve(data || {});
            return;
          }
          let detail = data && data.detail;
          if (Array.isArray(detail)) {
            detail = detail.map((part) => part.msg || String(part)).join("; ");
          }
          const failure = new Error(
            detail || "Upload failed (HTTP " + xhr.status + ")."
          );
          failure.status = xhr.status;
          reject(failure);
        };

        xhr.onerror = () => reject(new Error("Network error during upload."));
        xhr.ontimeout = () => reject(new Error("Upload timed out."));
        xhr.onabort = () => reject(new Error("Upload cancelled."));

        const form = new FormData();
        form.append("file", file, file.name);
        form.append("rel_path", entry.rel_path);
        form.append("size", String(entry.size));
        form.append("mtime_ns", String(entry.mtime_ns));
        xhr.send(form);
      });
    }

    async function uploadWithRetry(entry, file, onProgress) {
      let lastError = null;
      for (let attempt = 0; attempt < 3; attempt++) {
        try {
          return await uploadOne(entry, file, onProgress);
        } catch (error) {
          lastError = error;
          const status = error && error.status;
          const retryable = !status || status === 429 || status >= 500;
          if (!retryable || attempt === 2) throw error;
          setStatus(
            "Connection problem. Retrying " + baseName(entry.rel_path) +
            " (" + (attempt + 1) + "/2)…"
          );
          await sleep(800 * (attempt + 1));
        }
      }
      throw lastError || new Error("Upload failed.");
    }

    async function completeWithRetry() {
      for (let attempt = 0; attempt < 20; attempt++) {
        try {
          return await requestJSON("/complete", "POST", {});
        } catch (error) {
          if (error && error.status === 409 && attempt < 19) {
            setStatus("The PC is busy with another job. Waiting to start…");
            await sleep(3000);
            continue;
          }
          throw error;
        }
      }
      throw new Error("The PC stayed busy for too long. Try again later.");
    }

    async function pollStatus() {
      if (!state.polling) return;
      try {
        const response = await fetch("/status", {cache: "no-store"});
        if (!response.ok) throw new Error("Status request failed.");
        const data = await response.json();
        state.pollErrors = 0;

        if (data.stage === "done") {
          state.polling = false;
          setProgress(1, (data.files_processed || 0) + " file(s) processed");
          setStatus("Backup complete. You can close this page.", "ok");
          setBusy(false);
          resetBtn.classList.remove("hidden");
          return;
        }

        if (data.stage === "error") {
          state.polling = false;
          setStatus(
            data.error || "The PC could not finish the backup.", "err"
          );
          setBusy(false);
          resetBtn.classList.remove("hidden");
          return;
        }

        const processed = Number(data.files_processed) || 0;
        const total = Number(data.total_files) || 0;
        if (data.current_file) {
          setStatus(
            "Backing up on the PC: " + baseName(data.current_file) +
            " (" + processed + "/" + total + ")"
          );
        } else {
          setStatus(
            "Backing up on the PC…" +
            (total ? " " + processed + "/" + total : "")
          );
        }
        if (total > 0) {
          setProgress(processed / total, processed + " of " + total + " processed");
        }
        setTimeout(pollStatus, 1500);
      } catch (error) {
        state.pollErrors += 1;
        if (state.pollErrors >= 40) {
          state.polling = false;
          setStatus("Lost contact with the PC. Check the Wi-Fi connection.", "err");
          setBusy(false);
          resetBtn.classList.remove("hidden");
          return;
        }
        setTimeout(pollStatus, 2500);
      }
    }

    resetBtn.addEventListener("click", () => window.location.reload());

    window.addEventListener("beforeunload", (event) => {
      if (!state.busy) return;
      event.preventDefault();
      event.returnValue = "";
    });
  })();
  </script>
</body>
</html>
"""


# ---------------------------------------------------------------------------
# Manifest diff helpers
# ---------------------------------------------------------------------------


def safe_relative_path(rel_path: str) -> PurePosixPath | None:
    """Normalise *rel_path* or return ``None`` when it is unsafe.

    Rejects absolute paths, ``..`` segments, hidden files, Windows drive
    letters, NUL bytes, and SyncUp's own partial-file names.
    """
    raw = str(rel_path or "").replace("\\", "/").strip()
    if not raw or len(raw) > MAX_REL_PATH_LENGTH or "\x00" in raw:
        return None
    if raw.startswith("/") or raw.startswith("~"):
        return None

    parts: list[str] = []
    for part in raw.split("/"):
        if part in ("", "."):
            continue
        if part == ".." or part.startswith(".") or ":" in part:
            return None
        if is_partial_name(part):
            return None
        parts.append(part)

    if not parts:
        return None
    return PurePosixPath(*parts)


def matches_media_filter(rel_path: str, media_filter: str) -> bool:
    """Return True when *rel_path* passes the user's media filter."""
    suffix = PurePosixPath(rel_path).suffix.lower()
    if media_filter == "photos":
        return suffix in IMAGE_EXT
    if media_filter == "videos":
        return suffix in VIDEO_EXT
    return suffix in MEDIA_EXT


def _read_state(device_id: str) -> dict[str, dict]:
    """Read the processed-file map in the calling thread.

    A fresh :class:`StateStore` (and therefore a fresh SQLite connection) is
    created per request because FastAPI's worker threads are not stable.
    """
    store = StateStore(device_id)
    try:
        return store.load()
    finally:
        try:
            store.close()
        except Exception:  # noqa: BLE001 - closing must not mask route errors
            pass


def compute_missing_files(
    manifest: Sequence[ManifestEntry],
    state: Mapping[str, dict],
    media_filter: str = "both",
) -> tuple[list[MissingFileResponse], int, int]:
    """Diff the phone manifest against the SQLite state.

    Returns ``(missing, total_bytes, ignored)`` where *ignored* counts entries
    that were unsafe, hidden, duplicated, or excluded by the media filter.

    Rows written by older SyncUp versions have ``size`` set to ``None``.  They
    are trusted as already backed up, matching ``core.engine._is_processed``.
    """
    missing: list[MissingFileResponse] = []
    total_bytes = 0
    ignored = 0
    seen: set[str] = set()

    for entry in manifest:
        safe = safe_relative_path(entry.rel_path)
        if safe is None:
            ignored += 1
            continue
        rel = safe.as_posix()
        if rel in seen:
            ignored += 1
            continue
        seen.add(rel)

        if not matches_media_filter(rel, media_filter):
            ignored += 1
            continue

        stored = state.get(rel)
        status: str | None = None
        if stored is None:
            status = "new"
        else:
            stored_size = stored.get("size")
            if stored_size is None:
                # Legacy path-only row: the engine trusts it too.
                continue
            if int(stored_size) != entry.size:
                status = "modified"
            else:
                stored_mtime = stored.get("mtime")
                if (
                    stored_mtime is not None
                    and abs(int(stored_mtime) - entry.mtime_ns) > MTIME_TOLERANCE_NS
                ):
                    status = "modified"

        if status is not None:
            missing.append(
                MissingFileResponse(
                    rel_path=rel,
                    status=status,
                    size=entry.size,
                    mtime_ns=entry.mtime_ns,
                )
            )
            total_bytes += entry.size

    return missing, total_bytes, ignored


# ---------------------------------------------------------------------------
# Per-session shared state (request threads ↔ UI thread)
# ---------------------------------------------------------------------------


class BackupSession:
    """Thread-safe snapshot of one server session.

    All mutations happen under ``self._lock``; ``snapshot()`` is the only shape
    the Flet view consumes.
    """

    def __init__(self) -> None:
        self._lock = threading.RLock()
        self.stage = "waiting"
        self.client_ip: str | None = None
        self.expected_files = 0
        self.expected_bytes = 0
        self.files_received = 0
        self.bytes_received = 0
        self.files_processed = 0
        self.total_files = 0
        self.current_file: str | None = None
        self.error: str | None = None
        self.result: dict | None = None
        self.processing = False
        self._pending: set[str] = set()
        self._expectations: dict[str, tuple[int, int]] = {}
        self._selected_folders: set[str] = set()

    # -- manifest / uploads ------------------------------------------------

    def begin_manifest(
        self,
        missing: Sequence[MissingFileResponse],
        client_ip: str | None,
        selected_folders: Sequence[str],
    ) -> None:
        with self._lock:
            self.stage = "uploading"
            self.client_ip = client_ip
            self.expected_files = len(missing)
            self.expected_bytes = sum(item.size for item in missing)
            self.files_received = 0
            self.bytes_received = 0
            self.files_processed = 0
            self.total_files = len(missing)
            self.current_file = None
            self.error = None
            self.result = None
            self.processing = False
            self._pending = {item.rel_path for item in missing}
            self._expectations = {item.rel_path: (item.size, item.mtime_ns) for item in missing}
            self._selected_folders = set(selected_folders)

    def expectation(self, rel_path: str) -> tuple[int, int] | None:
        with self._lock:
            return self._expectations.get(rel_path)

    def note_upload(self, rel_path: str, size: int) -> bool:
        """Record a successful upload.  Returns False for a duplicate retry."""
        with self._lock:
            if rel_path not in self._pending:
                return False
            self._pending.discard(rel_path)
            self.files_received += 1
            self.bytes_received += max(0, int(size))
            return True

    # -- processing ---------------------------------------------------------

    def begin_processing(self) -> str:
        """Return ``started``, ``already``, ``done``, or ``empty``."""
        with self._lock:
            if self.processing:
                return "already"
            if self.stage == "done":
                return "done"
            if self.files_received <= 0:
                return "empty"
            self.processing = True
            self.stage = "processing"
            self.total_files = self.files_received
            self.files_processed = 0
            self.current_file = None
            self.error = None
            return "started"

    def note_progress(
        self,
        current: int,
        total: int,
        message: str,
        event: dict | None,
    ) -> None:
        if not isinstance(event, dict):
            return
        if event.get("phase") != "transfer":
            return
        with self._lock:
            if not self.processing:
                return
            if event.get("kind") == "phase":
                self.current_file = None
                if total:
                    self.total_files = int(total)
                return
            if event.get("kind") != "progress":
                return
            self.files_processed = max(0, int(current or 0))
            if total:
                self.total_files = int(total)
            metrics = event.get("metrics") or {}
            filename = metrics.get("filename")
            if filename:
                self.current_file = str(filename)

    def finish(self, result: dict | None) -> None:
        result = dict(result or {})
        with self._lock:
            self.processing = False
            self.files_processed = int(result.get("processed") or 0)
            self.total_files = int(result.get("total") or self.files_received)
            self.current_file = None
            self.result = result
            # Reject any straggling uploads after the session is finished.
            self._pending.clear()
            self._expectations.clear()
            if result.get("cancelled"):
                self.stage = "error"
                self.error = "Backup was cancelled on the PC."
            elif result.get("error"):
                self.stage = "error"
                self.error = str(result["error"])
            else:
                self.stage = "done"
                self.error = None

    def fail(self, message: str) -> None:
        with self._lock:
            self.processing = False
            self.stage = "error"
            self.current_file = None
            self.error = str(message)
            # Reject any straggling uploads after the session failed.
            self._pending.clear()
            self._expectations.clear()

    # -- readers ------------------------------------------------------------

    def is_processing(self) -> bool:
        with self._lock:
            return self.processing

    def counters(self) -> tuple[int, int]:
        with self._lock:
            return self.files_received, self.expected_files

    def selected_folders(self) -> list[str]:
        with self._lock:
            return sorted(self._selected_folders)

    def status(self) -> StatusResponse:
        with self._lock:
            return StatusResponse(
                stage=self.stage,
                files_received=self.files_received,
                files_processed=self.files_processed,
                total_files=self.total_files,
                current_file=self.current_file,
                error=self.error,
            )

    def snapshot(self) -> dict:
        """Plain dict for the Flet UI (never mutated by the caller)."""
        with self._lock:
            return {
                "stage": self.stage,
                "client_ip": self.client_ip,
                "expected_files": self.expected_files,
                "expected_bytes": self.expected_bytes,
                "files_received": self.files_received,
                "bytes_received": self.bytes_received,
                "files_processed": self.files_processed,
                "total_files": self.total_files,
                "current_file": self.current_file,
                "error": self.error,
                "processing": self.processing,
                "result": dict(self.result) if self.result else None,
            }


# ---------------------------------------------------------------------------
# FastAPI application factory
# ---------------------------------------------------------------------------


def create_app(
    device_id: str,
    staging_root: Path,
    dest_root: Path,
    sort_mode: str,
    media_filter: str,
    conflict_mode: str,
    progress_queue: "queue.Queue[tuple[int, int, str, dict | None]]",
    cancel_event: threading.Event,
    post_ui: Callable[[Callable[[], None]], None],
    *,
    on_state_changed: Callable[[dict], None] | None = None,
    is_busy: Callable[[], bool] | None = None,
) -> FastAPI:
    """Create the Wi-Fi backup application.

    Args:
        device_id: Logical ID for the SQLite state store (``"wifi-backup"``).
        staging_root: Temporary directory the phone uploads into.
        dest_root: Final destination folder on the PC.
        sort_mode: ``"year" | "month" | "day" | "flat"``.
        media_filter: ``"both" | "photos" | "videos"``.
        conflict_mode: ``"keep" | "overwrite"``.
        progress_queue: Queue consumed by ``main.progress_consumer``.
        cancel_event: Event shared with the engine and the Cancel button.
        post_ui: Schedules a callable on the Flet UI thread.
        on_state_changed: Optional UI callback receiving :meth:`BackupSession.snapshot`.
        is_busy: Optional predicate telling the server that another job is
            running (``/complete`` then returns HTTP 409 so the phone retries).
    """
    staging_root = Path(staging_root)
    dest_root = Path(dest_root)
    staging_root.mkdir(parents=True, exist_ok=True)

    app = FastAPI(title="SyncUp Wi-Fi Backup", version=VERSION)
    telemetry = WiFiTelemetry(progress_queue, post_ui)
    session = BackupSession()

    def notify() -> None:
        if on_state_changed is None:
            return
        try:
            snapshot = session.snapshot()
            post_ui(lambda: on_state_changed(snapshot))
        except Exception:  # noqa: BLE001 - UI notification is best-effort
            pass

    def client_ip(request: Request) -> str | None:
        try:
            return request.client.host if request.client else None
        except Exception:  # noqa: BLE001 - Starlette always provides one
            return None

    def engine_progress(
        current: int,
        total: int,
        message: str,
        event: dict | None = None,
    ) -> None:
        session.note_progress(current, total, message, event)
        telemetry.send(current, total, message, event)

    def engine_error(name: str, message: str) -> None:
        telemetry.error(f"Failed: {name}: {message}")

    def run_backup_job(selected_folders: list[str]) -> None:
        """Run the engine against the staging directory, then clean up."""
        result: dict = {}
        try:
            result = run_wifi_backup(
                device_id=device_id,
                staging_root=str(staging_root),
                selected_folders=list(selected_folders),
                dest_path=str(dest_root),
                sort_mode=sort_mode,
                media_filter=media_filter,
                conflict_mode=conflict_mode,
                progress_callback=engine_progress,
                cancel_event=cancel_event,
                error_callback=engine_error,
            )
            if not isinstance(result, dict):
                result = {
                    "error": "The backup engine returned an unexpected result.",
                    "total": session.status().total_files,
                    "processed": 0,
                    "skipped": 0,
                    "errors": 1,
                    "cancelled": False,
                }
        except Exception as exc:  # noqa: BLE001 - report, never crash Uvicorn
            result = {
                "error": f"Backup engine crashed: {exc}",
                "total": session.status().total_files,
                "processed": 0,
                "skipped": 0,
                "errors": 1,
                "cancelled": False,
            }
            telemetry.error(str(result["error"]))
        finally:
            session.finish(result)
            notify()
            try:
                shutil.rmtree(staging_root, ignore_errors=True)
            except OSError:
                pass
            telemetry.log("Staging area cleaned up.", LEVEL_INFO)

    # ------------------------------------------------------------------
    # Routes
    # ------------------------------------------------------------------

    @app.get("/", response_class=HTMLResponse, include_in_schema=False)
    def index() -> HTMLResponse:
        return HTMLResponse(
            MOBILE_HTML,
            headers={"Cache-Control": "no-store"},
        )

    @app.get("/favicon.ico", include_in_schema=False)
    def favicon() -> Response:
        return Response(status_code=204)

    @app.post("/manifest", response_model=ManifestResponse)
    def manifest_endpoint(
        payload: ManifestRequest,
        request: Request,
    ) -> ManifestResponse:
        """Diff the phone's manifest against the PC's SQLite state."""
        if session.is_processing():
            raise HTTPException(
                status_code=409,
                detail="A backup is already being processed.",
            )

        ip = client_ip(request)
        state = _read_state(device_id)
        missing, total_bytes, ignored = compute_missing_files(
            payload.files,
            state,
            media_filter,
        )

        folders = sorted(
            {
                parts[0]
                for item in missing
                for parts in [PurePosixPath(item.rel_path).parts]
                if len(parts) > 1
            }
        )
        session.begin_manifest(missing, ip, folders)

        telemetry.log(
            f"Manifest from {ip or 'phone'}: {len(payload.files)} announced, "
            f"{len(missing)} need uploading, {ignored} ignored."
        )
        if missing:
            telemetry.log(
                f"{len(missing)} file(s) queued for upload ({human_size(total_bytes)}).",
                LEVEL_ORGANIZE,
            )
        else:
            telemetry.log("The phone is already fully backed up.", LEVEL_SUCCESS)

        notify()
        return ManifestResponse(missing=missing, total_bytes=total_bytes)

    @app.post("/upload", response_model=UploadResponse)
    def upload_endpoint(
        request: Request,
        file: UploadFile = File(...),
        rel_path: str = Form(...),
        size: int = Form(...),
        mtime_ns: int = Form(...),
    ) -> UploadResponse:
        """Stream one file into the staging directory with atomic writes."""
        if session.is_processing():
            raise HTTPException(
                status_code=409,
                detail="A backup is already being processed.",
            )

        safe = safe_relative_path(rel_path)
        if safe is None:
            raise HTTPException(status_code=400, detail="Invalid relative path.")
        rel = safe.as_posix()

        if not matches_media_filter(rel, media_filter):
            raise HTTPException(
                status_code=400,
                detail=f"{rel} is not included by the current media filter.",
            )

        expected = session.expectation(rel)
        if expected is None:
            raise HTTPException(
                status_code=409,
                detail="This file was not requested by the manifest.",
            )
        expected_size, _expected_mtime = expected
        if size < 0 or mtime_ns < 0:
            raise HTTPException(status_code=400, detail="Invalid size or mtime.")
        if size != expected_size:
            raise HTTPException(
                status_code=400,
                detail="Size does not match the manifest.",
            )

        actual_size = getattr(file, "size", None)
        if actual_size is not None and int(actual_size) != size:
            raise HTTPException(
                status_code=400,
                detail="Uploaded size does not match the manifest.",
            )

        root = staging_root.resolve()
        destination = (root / Path(*safe.parts)).resolve()
        if not destination.is_relative_to(root):
            raise HTTPException(status_code=400, detail="Invalid relative path.")

        try:
            free_bytes = shutil.disk_usage(root).free
        except OSError:
            free_bytes = None
        if free_bytes is not None and size > free_bytes:
            raise HTTPException(
                status_code=507,
                detail="Not enough free space in the staging area.",
            )

        try:
            file.file.seek(0)
            destination.parent.mkdir(parents=True, exist_ok=True)
            copy_stream_atomic(
                file.file,
                destination,
                expected_size=size,
                mtime_ns=mtime_ns,
            )
        except Exception as exc:  # noqa: BLE001 - report per-upload failures
            telemetry.error(f"Upload failed for {rel}: {exc}")
            raise HTTPException(
                status_code=500,
                detail=f"Could not store {rel}.",
            ) from exc
        finally:
            try:
                file.file.close()
            except Exception:  # noqa: BLE001 - FastAPI will close it anyway
                pass

        counted = session.note_upload(rel, size)
        received, expected_files = session.counters()
        ip = client_ip(request)
        if counted:
            telemetry.upload_progress(received, expected_files, rel, size)
            telemetry.log(
                f"Received {rel} ({human_size(size)}) from {ip or 'phone'}.",
                LEVEL_INFO,
            )
        else:
            telemetry.log(f"Replaced staged copy of {rel}.")

        notify()
        return UploadResponse(
            rel_path=rel,
            files_received=received,
            expected_files=expected_files,
        )

    @app.post("/complete", response_model=StatusResponse)
    def complete_endpoint() -> StatusResponse:
        """Start processing every staged file, then return the initial status."""
        if is_busy is not None:
            try:
                busy = bool(is_busy())
            except Exception:  # noqa: BLE001 - never block the phone on this
                busy = False
            if busy:
                raise HTTPException(
                    status_code=409,
                    detail=("SyncUp is busy with another backup. Please try again in a moment."),
                )

        outcome = session.begin_processing()
        if outcome == "already":
            return session.status()
        if outcome == "done":
            return session.status()
        if outcome == "empty":
            session.fail("No files were uploaded.")
            notify()
            raise HTTPException(
                status_code=400,
                detail="No files were uploaded.",
            )

        # A stale Cancel from an earlier job must not abort this one.
        cancel_event.clear()
        telemetry.log("Phone finished uploading; starting the backup job.")
        notify()

        worker = threading.Thread(
            target=run_backup_job,
            args=(session.selected_folders(),),
            name="syncup-wifi-backup",
            daemon=True,
        )
        worker.start()
        return session.status()

    @app.get("/status", response_model=StatusResponse)
    def status_endpoint() -> StatusResponse:
        """Phone polling endpoint."""
        return session.status()

    # Exposed for debugging and for the Flet view.
    app.state.session = session
    app.state.staging_root = staging_root
    app.state.dest_root = dest_root
    app.state.device_id = device_id
    app.state.run_backup_job = run_backup_job
    app.state.telemetry = telemetry

    return app


# ---------------------------------------------------------------------------
# Uvicorn lifecycle
# ---------------------------------------------------------------------------


def find_available_port(
    preferred: int = DEFAULT_PORT,
    attempts: int = PORT_SCAN_ATTEMPTS,
) -> int:
    """Return the first bindable TCP port at or after *preferred*."""
    for offset in range(max(1, attempts)):
        port = preferred + offset
        if port > 65535:
            break
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            try:
                probe.bind(("0.0.0.0", port))
            except OSError:
                continue
            return port
    return preferred


class WiFiServer:
    """Run Uvicorn in a daemon thread with a clean shutdown path."""

    def __init__(
        self,
        host: str = "0.0.0.0",
        port: int = DEFAULT_PORT,
        *,
        log_level: str = "warning",
    ) -> None:
        self.host = host
        self.port = port
        self.log_level = log_level
        self._server: uvicorn.Server | None = None
        self._thread: threading.Thread | None = None
        self._error: str | None = None

    # -- lifecycle ----------------------------------------------------------

    def start(self, app: FastAPI) -> None:
        """Start serving *app* in a daemon thread.  Returns immediately."""
        if self.is_running:
            return

        self._error = None
        config = uvicorn.Config(
            app=app,
            host=self.host,
            port=self.port,
            log_level=self.log_level,
            access_log=False,
            lifespan="on",
        )
        self._server = uvicorn.Server(config=config)
        self._thread = threading.Thread(
            target=self._serve,
            name="syncup-wifi-uvicorn",
            daemon=True,
        )
        self._thread.start()

    def _serve(self) -> None:
        server = self._server
        if server is None:
            return
        try:
            server.run()
        except SystemExit:
            self._error = (
                f"Uvicorn exited while starting on port {self.port}. "
                "The port may already be in use."
            )
        except Exception as exc:  # noqa: BLE001 - surface via last_error
            self._error = f"{exc.__class__.__name__}: {exc}"

    def wait_until_ready(self, timeout: float = 5.0) -> bool:
        """Block until Uvicorn reports that it is accepting connections."""
        deadline = time.monotonic() + max(0.0, timeout)
        while time.monotonic() < deadline:
            if self._error is not None:
                return False
            server = self._server
            if server is not None and getattr(server, "started", False):
                return True
            thread = self._thread
            if thread is not None and not thread.is_alive():
                return False
            time.sleep(0.05)
        return False

    def stop(self, timeout: float = 5.0) -> None:
        """Ask Uvicorn to exit, then join its thread."""
        server = self._server
        thread = self._thread
        self._server = None
        self._thread = None

        if server is not None:
            server.should_exit = True

        if thread is not None and thread.is_alive():
            thread.join(timeout)
            if thread.is_alive() and server is not None:
                server.force_exit = True
                thread.join(1.0)
                if thread.is_alive():
                    self._error = "The Wi-Fi server did not stop cleanly."

    # -- introspection ------------------------------------------------------

    @property
    def is_running(self) -> bool:
        return self._thread is not None and self._thread.is_alive()

    @property
    def last_error(self) -> str | None:
        return self._error

    @property
    def url(self) -> str:
        host = "127.0.0.1" if self.host in ("0.0.0.0", "::") else self.host
        return f"http://{host}:{self.port}"


__all__ = [
    "DEFAULT_PORT",
    "MOBILE_HTML",
    "BackupSession",
    "compute_missing_files",
    "create_app",
    "find_available_port",
    "matches_media_filter",
    "safe_relative_path",
    "WiFiServer",
]
