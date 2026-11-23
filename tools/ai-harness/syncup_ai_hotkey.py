#!/usr/bin/env python3
"""Clipboard-driven, two-stage local runner for the SyncUp AI harness.

Version: 1.2.0

Default hotkey behavior (v1.2.0):
- Opens a visible terminal so progress and command output are shown live.
- need_context: run read-only context commands and return the output to the clipboard.
- ready: dry-run, apply files, show a concise Git summary, then run verification commands
  automatically in the same shortcut press and return the output.
- preflight/blocked: do not execute anything.
- Optional --review-mode keeps the previous two-press review checkpoint.

The actual project path is fixed by default, but SYNCUP_PROJECT_ROOT can override
it for testing or a different checkout.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
import queue
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime
from pathlib import Path
from typing import Any

PROJECT = Path(os.environ.get("SYNCUP_PROJECT_ROOT", "/home/uday/Music/SyncUp")).expanduser().resolve()
HARNESS = PROJECT / "tools" / "ai-harness"
RUNNER = HARNESS / "ai_runner.py"
REPLY = PROJECT / "reply.json"
RUN_OUTPUT = PROJECT / "ai-run-output.txt"
REVIEW_OUTPUT = PROJECT / "ai-review-diff.txt"
STATE = PROJECT / ".ai-automation-pending.json"
LOG = PROJECT / "ai-hotkey.log"
SESSION_LOG = PROJECT / "ai-hotkey-session.log"

# Keep the read-only context phase deliberately narrower than ai_runner.py's
# general command allowlist. Unsupported commands fail closed and can be run
# manually after review.
READ_ONLY_EXECUTABLES = {
    "rg", "grep", "cat", "head", "tail", "ls", "find", "tree", "pwd",
    "stat", "file", "git",
}
READY_EXECUTABLES = {
    "cargo", "rustc", "npm", "pnpm", "yarn", "node", "make", "git",
    "rg", "grep", "cat", "head", "tail", "ls", "find", "tree", "pwd",
    "stat", "file", "diff", "patch", "gio", "echo",
}
SHELL_META = (";", "&", "|", "<", ">", "`", "$(")
SENSITIVE_MARKERS = (
    ".env", ".ssh", ".gnupg", ".config/syncup", "id_rsa", "id_ed25519",
    "credential", "password", "wallet", "secret", "access_token", "refresh_token",
)


class HarnessError(Exception):
    pass


def log(message: str) -> None:
    timestamp = datetime.now().astimezone().isoformat(timespec="seconds")
    try:
        with LOG.open("a", encoding="utf-8") as handle:
            handle.write(f"[{timestamp}] {message}\n")
    except OSError:
        pass


def progress(message: str) -> None:
    """Print and persist a timestamped progress line in the visible terminal."""
    timestamp = datetime.now().astimezone().strftime("%H:%M:%S")
    line = f"[{timestamp}] {message}"
    print(line, flush=True)
    try:
        with SESSION_LOG.open("a", encoding="utf-8") as handle:
            handle.write(line + "\n")
    except OSError:
        pass
    log(message)


def notify(title: str, message: str, urgency: str = "normal") -> None:
    log(f"NOTIFY {title}: {message}")
    binary = shutil.which("notify-send")
    if binary:
        try:
            subprocess.run(
                [binary, "-a", "SyncUp AI Harness", "-u", urgency, "-t", "9000", title, message],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                timeout=5,
            )
            return
        except (OSError, subprocess.TimeoutExpired):
            pass
    # Fallback: opening a small Mousepad note still makes the error visible.
    try:
        note = PROJECT / "ai-hotkey-notice.txt"
        note.write_text(f"{title}\n\n{message}\n", encoding="utf-8")
        open_mousepad(note)
    except OSError:
        print(f"{title}: {message}", file=sys.stderr)


def open_mousepad(path: Path) -> None:
    mousepad = shutil.which("mousepad")
    if not mousepad:
        raise HarnessError("Mousepad was not found. Install it with: sudo apt install mousepad")
    subprocess.Popen(
        [mousepad, str(path)],
        cwd=str(PROJECT),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        start_new_session=True,
    )


def read_clipboard() -> str:
    xclip = shutil.which("xclip")
    if not xclip:
        raise HarnessError("xclip was not found. Install it with: sudo apt install xclip")
    try:
        result = subprocess.run(
            [xclip, "-selection", "clipboard", "-o"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=8,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise HarnessError("Timed out reading the clipboard.") from exc
    if result.returncode != 0:
        raise HarnessError(f"Could not read clipboard: {result.stderr.strip() or result.returncode}")
    if not result.stdout.strip():
        raise HarnessError("Clipboard is empty. Copy Chat B's JSON response first.")
    return result.stdout


def copy_to_clipboard(content: str) -> None:
    xclip = shutil.which("xclip")
    if not xclip:
        raise HarnessError("xclip was not found; cannot copy the output automatically.")
    try:
        # xclip normally forks a background owner process after reading stdin.
        # Do not capture stderr: the forked owner can inherit that pipe, which
        # keeps communicate() waiting for EOF even though the clipboard is set.
        # Redirecting both output streams to DEVNULL lets the short-lived
        # launcher exit while the background xclip process owns the selection.
        result = subprocess.run(
            [xclip, "-selection", "clipboard", "-i"],
            input=content.encode("utf-8"),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=8,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise HarnessError(
            "Timed out while handing the output to xclip. The output remains saved in ai-run-output.txt."
        ) from exc
    if result.returncode != 0:
        raise HarnessError(
            f"xclip could not set the clipboard (exit {result.returncode}). "
            "The output remains saved in ai-run-output.txt."
        )


def extract_payload(raw: str) -> tuple[dict[str, Any], str]:
    # Prefer one fenced JSON block, as Chat B's output contract requests.
    fenced = re.findall(r"```\s*(?:json)?\s*([\[{].*?[\]}])\s*```", raw, flags=re.IGNORECASE | re.DOTALL)
    candidates = list(reversed(fenced))
    candidates.append(raw.strip())
    decoder = json.JSONDecoder()
    for candidate in candidates:
        start = candidate.find("{")
        if start < 0:
            continue
        try:
            value, _end = decoder.raw_decode(candidate[start:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            canonical = json.dumps(value, ensure_ascii=False, indent=2) + "\n"
            return value, canonical
    raise HarnessError("No valid JSON object found. Copy the complete JSON block, including its opening and closing braces.")


def normalize_commands(payload: dict[str, Any]) -> list[str]:
    commands = payload.get("commands", [])
    if isinstance(commands, str):
        commands = [commands]
    if not isinstance(commands, list) or any(not isinstance(item, str) or not item.strip() for item in commands):
        raise HarnessError("The JSON 'commands' field must be an array of non-empty strings.")
    return commands


def validate_command_list(commands: list[str], mode: str) -> None:
    allowed = READ_ONLY_EXECUTABLES if mode == "need_context" else READY_EXECUTABLES
    for command in commands:
        if any(token in command for token in SHELL_META):
            raise HarnessError(
                f"Blocked command because it contains a shell operator: {command}\n"
                "Ask Chat B to split it into separate commands without |, ;, &, <, >, backticks or $()."
            )
        try:
            argv = shlex.split(command)
        except ValueError as exc:
            raise HarnessError(f"Could not parse command: {command}: {exc}") from exc
        if not argv:
            raise HarnessError("An empty command was found in the payload.")
        lowered = command.lower().replace("\\", "/")
        for marker in SENSITIVE_MARKERS:
            if marker in lowered:
                raise HarnessError(f"Blocked command because it references a protected path or credential term ({marker}): {command}")
        for token in argv[1:]:
            if token.startswith(("/", "~/", "~\\")):
                raise HarnessError(f"Only project-relative paths are allowed in automated commands: {command}")
            if ".." in Path(token).parts:
                raise HarnessError(f"Parent-directory traversal is not allowed in automated commands: {command}")
        executable = Path(argv[0]).name
        if executable not in allowed:
            raise HarnessError(f"Command is not approved for the {mode} phase: {command}")
        if executable == "find" and any(flag in argv for flag in ("-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprintf", "-fls")):
            raise HarnessError(f"Destructive or write-capable find option blocked: {command}")
        if executable == "git" and any(arg.startswith("--output") for arg in argv[1:]):
            raise HarnessError(f"Git output-to-file option is not allowed: {command}")
        if executable == "git":
            subcommand = argv[1] if len(argv) > 1 else ""
            readonly_git = {"status", "diff", "log", "show", "branch", "rev-parse", "ls-files", "describe"}
            if subcommand not in readonly_git:
                raise HarnessError(f"Only read-only git commands are allowed in this workflow: {command}")


def atomic_write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", dir=path.parent, delete=False) as handle:
        temp_path = Path(handle.name)
        handle.write(content)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temp_path, path)


def save_payload(canonical: str) -> str:
    if REPLY.exists():
        backup_dir = PROJECT / ".ai-backups" / "clipboard-payloads"
        backup_dir.mkdir(parents=True, exist_ok=True)
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
        shutil.copy2(REPLY, backup_dir / f"reply-{stamp}.json")
    atomic_write(REPLY, canonical)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def run_process(
    argv: list[str],
    timeout: int | None = None,
    *,
    stream: bool = False,
) -> subprocess.CompletedProcess[str]:
    """Run a local command; optionally stream output and heartbeat to the terminal."""
    log(f"RUN {shlex.join(argv)}")
    if not stream:
        try:
            result = subprocess.run(
                argv,
                cwd=str(PROJECT),
                capture_output=True,
                text=True,
                encoding="utf-8",
                errors="replace",
                timeout=timeout,
                check=False,
            )
        except subprocess.TimeoutExpired as exc:
            raise HarnessError(f"Command timed out: {shlex.join(argv)}") from exc
        log(f"EXIT {result.returncode}: {shlex.join(argv)}")
        return result

    print("\n$ " + shlex.join(argv), flush=True)
    try:
        proc = subprocess.Popen(
            argv,
            cwd=str(PROJECT),
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )
    except OSError as exc:
        raise HarnessError(f"Could not start command {shlex.join(argv)}: {exc}") from exc

    output_queue: queue.Queue[str | None] = queue.Queue()

    def read_output() -> None:
        assert proc.stdout is not None
        try:
            for output_line in proc.stdout:
                output_queue.put(output_line)
        finally:
            output_queue.put(None)

    reader = threading.Thread(target=read_output, name="syncup-command-output", daemon=True)
    reader.start()
    collected: list[str] = []
    started = time.monotonic()
    last_heartbeat = started
    stream_finished = False

    while not stream_finished or proc.poll() is None:
        try:
            item = output_queue.get(timeout=3)
        except queue.Empty:
            now = time.monotonic()
            if timeout is not None and now - started > timeout and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait()
                raise HarnessError(f"Command timed out after {timeout}s: {shlex.join(argv)}")
            if proc.poll() is None and now - last_heartbeat >= 6:
                elapsed = int(now - started)
                heartbeat = f"[RUNNING] Still working ({elapsed}s elapsed): {shlex.join(argv)}"
                print(heartbeat, flush=True)
                try:
                    with SESSION_LOG.open("a", encoding="utf-8") as handle:
                        handle.write(heartbeat + "\n")
                except OSError:
                    pass
                last_heartbeat = now
            continue

        if item is None:
            stream_finished = True
            continue
        collected.append(item)
        print(item, end="", flush=True)
        try:
            with SESSION_LOG.open("a", encoding="utf-8") as handle:
                handle.write(item)
        except OSError:
            pass
        last_heartbeat = time.monotonic()

    return_code = proc.wait()
    stdout = "".join(collected)
    print(f"[EXIT] {return_code}: {shlex.join(argv)}", flush=True)
    log(f"EXIT {return_code}: {shlex.join(argv)}")
    return subprocess.CompletedProcess(argv, return_code, stdout, "")


def runner_command(*flags: str) -> list[str]:
    if not RUNNER.is_file():
        raise HarnessError(f"Runner not found: {RUNNER}")
    return [sys.executable, str(RUNNER), str(REPLY), *flags]


def git_output(*args: str) -> str:
    result = run_process(["git", *args], timeout=30)
    if result.returncode != 0:
        raise HarnessError(f"git {' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


def current_branch() -> str:
    return git_output("branch", "--show-current").strip()


def tree_snapshot() -> str:
    """Hashable view of tracked, staged and untracked working-tree changes."""
    chunks: list[bytes] = []
    for args in (
        ["status", "--porcelain", "-z"],
        ["diff", "--binary", "HEAD"],
        ["diff", "--cached", "--binary", "HEAD"],
        ["ls-files", "--others", "--exclude-standard", "-z"],
    ):
        result = subprocess.run(["git", *args], cwd=str(PROJECT), capture_output=True, check=False)
        if result.returncode == 0:
            chunks.append(result.stdout)
    untracked = subprocess.run(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"],
        cwd=str(PROJECT), capture_output=True, check=False,
    )
    if untracked.returncode == 0:
        for raw_path in untracked.stdout.split(b"\0"):
            if not raw_path:
                continue
            try:
                rel = raw_path.decode("utf-8")
                path = (PROJECT / rel).resolve()
                path.relative_to(PROJECT)
                if path.is_file() and path.stat().st_size <= 1024 * 1024:
                    data = path.read_bytes()
                    chunks.append(rel.encode("utf-8") + b"\0" + data)
            except (UnicodeDecodeError, OSError, ValueError):
                continue
    return hashlib.sha256(b"\0".join(chunks)).hexdigest()


def make_review_file(payload: dict[str, Any]) -> str:
    branch = current_branch()
    status = git_output("status", "--short")
    diffstat = git_output("diff", "--stat")
    staged_stat = git_output("diff", "--cached", "--stat")
    diff = git_output("diff", "--no-ext-diff", "--no-color")
    staged_diff = git_output("diff", "--cached", "--no-ext-diff", "--no-color")
    untracked_raw = git_output("ls-files", "--others", "--exclude-standard", "-z")
    untracked_paths = [item for item in untracked_raw.split("\0") if item]

    lines = [
        "SYNCUP AI CHANGE REVIEW",
        "=======================",
        f"Generated: {datetime.now().astimezone().isoformat(timespec='seconds')}",
        f"Project: {PROJECT}",
        f"Branch: {branch}",
        "",
        "PAYLOAD SUMMARY",
        "---------------",
        str(payload.get("summary", "(no summary)")),
        "",
        "GIT STATUS --SHORT",
        "------------------",
        status or "(clean)",
        "",
        "GIT DIFF --STAT",
        "----------------",
        diffstat or "(no tracked unstaged diff)",
        "",
        "GIT DIFF --CACHED --STAT",
        "------------------------",
        staged_stat or "(no staged diff)",
        "",
        "TRACKED UNSTAGED DIFF",
        "---------------------",
        diff or "(none)",
        "",
        "TRACKED STAGED DIFF",
        "-------------------",
        staged_diff or "(none)",
        "",
        "UNTRACKED FILES",
        "---------------",
        "\n".join(untracked_paths) if untracked_paths else "(none)",
        "",
        "NEW / UNTRACKED TEXT FILE CONTENTS",
        "-----------------------------------",
        "Note: ordinary git diff does not show untracked file contents. Inspect these too.",
    ]
    for rel in untracked_paths:
        path = (PROJECT / rel).resolve()
        try:
            path.relative_to(PROJECT)
            if not path.is_file():
                continue
            data = path.read_bytes()
            if len(data) > 512 * 1024 or b"\0" in data:
                lines.extend(["", f"### {rel}", "[Skipped: binary or larger than 512 KiB]"])
                continue
            content = data.decode("utf-8")
            lines.extend(["", f"### {rel}", content])
        except (OSError, UnicodeDecodeError, ValueError):
            lines.extend(["", f"### {rel}", "[Could not read this file as UTF-8]"])
    review = "\n".join(lines) + "\n"
    atomic_write(REVIEW_OUTPUT, review)
    return tree_snapshot()


def write_run_output(
    runner_result: subprocess.CompletedProcess[str],
    phase: str,
    extra_sections: str = "",
) -> str:
    pieces: list[str] = [
        "SYNCUP AI HARNESS OUTPUT",
        "========================",
        f"Phase: {phase}",
        f"Time: {datetime.now().astimezone().isoformat(timespec='seconds')}",
        f"Runner exit code: {runner_result.returncode}",
        "",
    ]
    if RUN_OUTPUT.exists():
        try:
            body = RUN_OUTPUT.read_text(encoding="utf-8", errors="replace")
        except OSError:
            body = ""
    else:
        body = ""
    if not body.strip():
        body = (runner_result.stdout or "")
    else:
        # ai_runner.py writes command output to RUN_OUTPUT and also streams the
        # same output to stdout. Keep the file log as the canonical command
        # transcript, and copy only the runner's short pre-command summary so
        # command output is not duplicated in the final Mousepad document.
        wrapper_lines: list[str] = []
        for line in (runner_result.stdout or "").splitlines():
            if line.startswith("$ "):
                break
            wrapper_lines.append(line)
        wrapper = "\n".join(wrapper_lines).strip()
        if wrapper and "AI payload" not in body:
            body = wrapper + "\n\n--- RUNNER COMMAND LOG ---\n" + body
    if runner_result.stderr and runner_result.stderr.strip():
        body += "\n\n--- RUNNER STDERR ---\n" + runner_result.stderr.strip()
    pieces.append(body.rstrip())
    if extra_sections.strip():
        pieces.extend(["", "=== AUTOMATION PRE-VERIFICATION STEPS ===", extra_sections.rstrip()])
    pieces.extend([
        "",
        "=== GIT STATUS --SHORT (after run) ===",
        safe_git_output("status", "--short"),
        "",
        "=== GIT DIFF --STAT (after run) ===",
        safe_git_output("diff", "--stat"),
        "",
        f"=== AUTOMATION RESULT: runner exit code {runner_result.returncode} ===",
    ])
    final_text = "\n".join(pieces) + "\n"
    atomic_write(RUN_OUTPUT, final_text)
    return final_text


def safe_git_output(*args: str) -> str:
    try:
        result = subprocess.run(["git", *args], cwd=str(PROJECT), capture_output=True, text=True, encoding="utf-8", errors="replace", check=False)
        if result.returncode == 0:
            return result.stdout.rstrip() or "(none)"
        return f"[git command failed: {result.stderr.strip()}]"
    except OSError as exc:
        return f"[git unavailable: {exc}]"


def return_output_to_user(text: str, phase: str, exit_code: int) -> None:
    copy_error = None
    try:
        copy_to_clipboard(text)
    except HarnessError as exc:
        copy_error = str(exc)
    try:
        open_mousepad(RUN_OUTPUT)
    except HarnessError as exc:
        log(str(exc))
    if copy_error:
        notify("SyncUp AI Harness", f"Run finished (exit {exit_code}), but clipboard copy failed: {copy_error}", "critical")
    elif exit_code == 0:
        notify("SyncUp AI Harness", f"{phase} finished. Output is open in Mousepad and copied to clipboard. Paste it into Chat B.")
    else:
        notify("SyncUp AI Harness", f"{phase} finished with errors (exit {exit_code}). Output is open and copied; paste it into Chat B.", "critical")


def handle_context(payload: dict[str, Any]) -> None:
    commands = normalize_commands(payload)
    if payload.get("files") or payload.get("edits") or str(payload.get("patch", "")).strip():
        raise HarnessError("A need_context payload must not contain file writes or patches. Nothing was executed.")
    validate_command_list(commands, "need_context")
    if not commands:
        notify("SyncUp AI Harness", "Chat B requested context but supplied no commands. Nothing was executed.")
        return
    progress("CONTEXT PHASE: validating read-only commands")
    RUN_OUTPUT.unlink(missing_ok=True)
    result = run_process(runner_command("--apply", "--run-only", "--yes"), stream=True)
    output = write_run_output(result, "need_context")
    return_output_to_user(output, "Context collection", result.returncode)


def show_git_change_summary() -> str:
    status = safe_git_output("status", "--short")
    diffstat = safe_git_output("diff", "--stat")
    cached_stat = safe_git_output("diff", "--cached", "--stat")
    summary = (
        "GIT STATUS --SHORT\n" + status + "\n\n"
        "GIT DIFF --STAT\n" + diffstat + "\n\n"
        "GIT DIFF --CACHED --STAT\n" + cached_stat
    )
    print("\n========== CHANGE SUMMARY (informational) ==========", flush=True)
    print(summary, flush=True)
    print("====================================================\n", flush=True)
    return summary


def prepare_specta_bindings(commands: list[str]) -> str:
    """Regenerate and stage only the generated IPC binding before specta:check.

    The project's specta:check script compares the working tree to the Git index.
    This narrowly stages only src/lib/ipc.ts after regeneration; it never stages
    application source files and never commits anything.
    """
    if "npm run specta:check" not in commands:
        return ""

    status_before = git_output("status", "--porcelain", "--", "src/lib/ipc.ts").strip()
    if status_before:
        raise HarnessError(
            "Cannot automatically prepare specta bindings because src/lib/ipc.ts already had Git changes "
            "before this automation run. Review that file and stage it manually if appropriate. "
            f"Existing status: {status_before}"
        )

    progress("SPECTA PREPARATION: regenerating src/lib/ipc.ts")
    generated = run_process(["npm", "run", "gen:bindings"], stream=True, timeout=600)
    if generated.returncode != 0:
        raise HarnessError("npm run gen:bindings failed. Verification stopped; see terminal output.")

    progress("SPECTA PREPARATION: staging only generated src/lib/ipc.ts (no commit)")
    staged = run_process(["git", "add", "--", "src/lib/ipc.ts"], stream=True, timeout=30)
    if staged.returncode != 0:
        raise HarnessError("Could not stage generated src/lib/ipc.ts. Verification stopped.")
    return (
        "$ npm run gen:bindings\n" + (generated.stdout or "") + f"EXIT={generated.returncode}\n\n"
        "$ git add -- src/lib/ipc.ts\n" + (staged.stdout or "") + f"EXIT={staged.returncode}"
    )


def handle_ready(payload: dict[str, Any], payload_hash: str, review_mode: bool = False) -> None:
    if payload.get("questions"):
        raise HarnessError("Chat B's ready payload still contains questions. Ask Chat B to resolve them before applying anything.")
    commands = normalize_commands(payload)
    validate_command_list(commands, "ready")

    progress("READY PAYLOAD ACCEPTED: changes and verification are authorized by this shortcut press")
    progress("STEP 1/4 — DRY RUN: checking every proposed file change")
    RUN_OUTPUT.unlink(missing_ok=True)
    dry_result = run_process(runner_command("--yes"), stream=True, timeout=300)
    dry_text = (dry_result.stdout or "") + "\n" + (dry_result.stderr or "")
    error_markers = ("ERROR:", "MISSING FILE", "found 0 occurrence(s)", "multiple occurrences found")
    if dry_result.returncode != 0 or any(marker.lower() in dry_text.lower() for marker in error_markers):
        failure = "DRY RUN FAILED. No file changes were applied.\n\n" + dry_text
        atomic_write(RUN_OUTPUT, failure)
        return_output_to_user(failure, "Dry run", dry_result.returncode or 1)
        return

    progress("STEP 2/4 — APPLY: writing files only; verification commands are not running yet")
    write_result = run_process(runner_command("--apply", "--write-only", "--yes"), stream=True, timeout=600)
    if write_result.returncode != 0:
        output = write_run_output(write_result, "write-only (failed or partially applied)", dry_result.stdout or "")
        return_output_to_user(output, "File application", write_result.returncode)
        return

    progress("STEP 3/4 — PREPARING VERIFICATION: checking generated bindings if specta:check is requested")
    try:
        specta_note = prepare_specta_bindings(commands)
    except HarnessError as exc:
        failure = "VERIFICATION PREPARATION STOPPED. File changes were applied, but verification did not run.\n\n" + str(exc)
        atomic_write(RUN_OUTPUT, failure)
        return_output_to_user(failure, "Verification preparation", 1)
        return

    change_summary = show_git_change_summary()
    if review_mode:
        snapshot_hash = make_review_file(payload)
        branch = current_branch()
        head = git_output("rev-parse", "HEAD").strip()
        state = {
            "phase": "awaiting_review",
            "created_at": datetime.now().astimezone().isoformat(timespec="seconds"),
            "payload_sha256": payload_hash,
            "tree_sha256": snapshot_hash,
            "branch": branch,
            "head": head,
        }
        atomic_write(STATE, json.dumps(state, indent=2) + "\n")
        open_mousepad(REVIEW_OUTPUT)
        notify(
            "SyncUp AI Harness",
            "File changes applied. Review ai-review-diff.txt. Press Ctrl+` again only after approving; this runs verification.",
        )
        return

    progress("STEP 4/4 — VERIFICATION: running Chat B's commands one by one")
    progress(f"Verification command count: {len(commands)}")
    result = run_process(runner_command("--apply", "--run-only", "--yes"), stream=True, timeout=None)
    extra = (
        "DRY RUN OUTPUT\n---------------\n" + (dry_result.stdout or "(no output)") + "\n\n"
        "WRITE-ONLY OUTPUT\n------------------\n" + (write_result.stdout or "(no output)") + "\n\n"
        "GIT CHANGE SUMMARY\n-------------------\n" + change_summary
    )
    if specta_note:
        extra += "\n\nSPECTA BINDING PREPARATION\n--------------------------\n" + specta_note
    output = write_run_output(result, "ready (automatic apply + verification)", extra)
    return_output_to_user(output, "Implementation and verification", result.returncode)


def resume_verification() -> None:
    try:
        state = json.loads(STATE.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise HarnessError(f"Pending review state is unreadable: {exc}") from exc
    if state.get("phase") != "awaiting_review":
        raise HarnessError("Pending state is invalid. Use --cancel-pending to clear it safely.")
    payload_text = REPLY.read_text(encoding="utf-8")
    current_payload_hash = hashlib.sha256(payload_text.encode("utf-8")).hexdigest()
    if current_payload_hash != state.get("payload_sha256"):
        raise HarnessError("reply.json changed after review. Re-run the workflow with the intended Chat B payload.")
    clipboard_raw = read_clipboard()
    _clipboard_payload, clipboard_canonical = extract_payload(clipboard_raw)
    clipboard_hash = hashlib.sha256(clipboard_canonical.encode("utf-8")).hexdigest()
    if clipboard_hash != state.get("payload_sha256"):
        raise HarnessError(
            "A different clipboard item is now selected. The pending ready payload has not been run. "
            "Copy the same Chat B JSON again before pressing Ctrl+`, or cancel with --cancel-pending."
        )
    if current_branch() != state.get("branch") or git_output("rev-parse", "HEAD").strip() != state.get("head"):
        raise HarnessError("Git branch or HEAD changed after review. Re-check the diff before running verification.")
    current_tree_hash = tree_snapshot()
    if current_tree_hash != state.get("tree_sha256"):
        # Refresh the review and require another keypress; do not run gates on
        # a tree different from the one the user reviewed.
        payload, _canonical = extract_payload(payload_text)
        new_hash = make_review_file(payload)
        state["tree_sha256"] = new_hash
        atomic_write(STATE, json.dumps(state, indent=2) + "\n")
        try:
            open_mousepad(REVIEW_OUTPUT)
        except HarnessError:
            pass
        notify("SyncUp AI Harness", "The project changed after the review was generated. Review the refreshed ai-review-diff.txt, then press Ctrl+` again to run verification.", "critical")
        return

    # Do not discard the pending state until the command phase has actually run.
    RUN_OUTPUT.unlink(missing_ok=True)
    result = run_process(runner_command("--apply", "--run-only", "--yes"), stream=True)
    output = write_run_output(result, "verification")
    STATE.unlink(missing_ok=True)
    return_output_to_user(output, "Verification", result.returncode)


def find_terminal_command(worker_args: list[str]) -> list[str]:
    """Build a command for a visible terminal supported by common Linux desktops."""
    script = str(Path(__file__).resolve())
    worker = [sys.executable, script, *worker_args]

    xfce = shutil.which("xfce4-terminal")
    if xfce:
        return [xfce, "--title=SyncUp AI Harness", "--execute", *worker]

    xterm_compat = shutil.which("x-terminal-emulator")
    if xterm_compat:
        return [xterm_compat, "-T", "SyncUp AI Harness", "-e", *worker]

    mate = shutil.which("mate-terminal")
    if mate:
        return [mate, "--title=SyncUp AI Harness", "--", *worker]

    gnome = shutil.which("gnome-terminal")
    if gnome:
        return [gnome, "--title=SyncUp AI Harness", "--", *worker]

    xterm = shutil.which("xterm")
    if xterm:
        return [xterm, "-T", "SyncUp AI Harness", "-e", *worker]

    raise HarnessError(
        "No supported terminal emulator was found. Install xfce4-terminal, then retry the shortcut."
    )


def launch_visible_worker(review_mode: bool = False) -> int:
    worker_args = ["--worker"]
    if review_mode:
        worker_args.append("--review-mode")
    command = find_terminal_command(worker_args)
    try:
        subprocess.Popen(
            command,
            cwd=str(PROJECT),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    except OSError as exc:
        raise HarnessError(f"Could not open the progress terminal: {exc}") from exc
    notify(
        "SyncUp AI Harness",
        "Execution started. Watch the SyncUp AI Harness terminal for live progress and command output.",
    )
    return 0


def process_clipboard_payload(review_mode: bool = False) -> int:
    raw = read_clipboard()
    payload, canonical = extract_payload(raw)
    status = payload.get("status")
    if status not in {"preflight", "need_context", "ready", "blocked"}:
        raise HarnessError(f"Unsupported or missing payload status: {status!r}")
    payload_hash = save_payload(canonical)
    log(f"Saved Chat B payload with status={status}")
    progress(f"PAYLOAD RECEIVED: status={status}")

    if status == "preflight":
        if payload.get("files") or payload.get("edits") or str(payload.get("patch", "")).strip() or normalize_commands(payload):
            raise HarnessError("Preflight payload unexpectedly contains actions. Nothing was executed.")
        progress("PREFLIGHT: no files changed and no commands executed")
        notify("SyncUp AI Harness", "Pre-flight received. No files were changed and no commands were run.")
        return 0
    if status == "blocked":
        progress("BLOCKED: Chat B marked this payload as blocked. Nothing was executed.")
        notify("SyncUp AI Harness", "Chat B marked this payload as blocked. Nothing was executed.", "critical")
        return 0
    if status == "need_context":
        handle_context(payload)
        return 0
    if status == "ready":
        handle_ready(payload, payload_hash, review_mode=review_mode)
        return 0
    return 0


def run_worker(review_mode: bool = False, pause_on_exit: bool = True) -> int:
    SESSION_LOG.write_text(
        f"SyncUp AI Harness v1.2.0\nStarted: {datetime.now().astimezone().isoformat(timespec='seconds')}\n\n",
        encoding="utf-8",
    )
    code = 0
    lock_path = Path(tempfile.gettempdir()) / f"syncup-ai-hotkey-{os.getuid()}.lock"
    try:
        with lock_path.open("w", encoding="utf-8") as lock_handle:
            try:
                fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise HarnessError("Another harness operation is already running. Wait for it to finish.")

            if STATE.exists():
                progress("PENDING REVIEW FOUND: continuing the earlier review-mode verification")
                resume_verification()
            else:
                process_clipboard_payload(review_mode=review_mode)
    except KeyboardInterrupt:
        code = 130
        progress("INTERRUPTED: user pressed Ctrl+C. Check Git status and backups before continuing.")
        notify("SyncUp AI Harness interrupted", "Execution was interrupted. Check ai-hotkey-session.log, Git status and .ai-backups before retrying.", "critical")
    except HarnessError as exc:
        code = 1
        progress(f"STOPPED: {exc}")
        notify("SyncUp AI Harness stopped", str(exc), "critical")
    except Exception as exc:
        code = 1
        progress(f"UNEXPECTED ERROR: {type(exc).__name__}: {exc}")
        notify("SyncUp AI Harness error", f"{type(exc).__name__}: {exc}", "critical")
    progress(f"WORKER FINISHED: exit code {code}")
    if pause_on_exit and sys.stdin.isatty():
        try:
            input("\nPress Enter to close this progress window...")
        except EOFError:
            pass
    return code


def main() -> int:
    if not PROJECT.is_dir():
        raise HarnessError(f"Project directory does not exist: {PROJECT}")
    if not (PROJECT / ".git").exists() and run_process(["git", "rev-parse", "--is-inside-work-tree"], timeout=10).returncode != 0:
        raise HarnessError(f"Not a Git repository: {PROJECT}")
    if not RUNNER.is_file():
        raise HarnessError(f"The existing runner is missing: {RUNNER}")

    args = sys.argv[1:]
    if "--worker" in args:
        review_mode = "--review-mode" in args
        return run_worker(review_mode=review_mode, pause_on_exit="--no-pause" not in args)

    if args and args[0] == "--cancel-pending":
        if STATE.exists():
            STATE.unlink()
            notify("SyncUp AI Harness", "Pending verification cancelled. Already-applied file changes were NOT reverted.")
        else:
            notify("SyncUp AI Harness", "There is no pending verification stage.")
        return 0
    if args and args[0] == "--status":
        if STATE.exists():
            notify("SyncUp AI Harness", "A ready payload is awaiting review. Review ai-review-diff.txt; press Ctrl+` to continue.")
        else:
            notify("SyncUp AI Harness", "No pending verification stage.")
        return 0
    if args and args[0] in {"--help", "-h"}:
        print(
            "Usage: syncup-ai-hotkey [--status|--cancel-pending|--review-mode]\n"
            "Default shortcut: opens a visible terminal and automatically runs ready payloads\n"
            "--review-mode: restores the optional two-press Git review checkpoint"
        )
        return 0
    if args and args[0] == "--review-mode":
        return launch_visible_worker(review_mode=True)
    if args:
        raise HarnessError(f"Unknown option: {args[0]}")
    return launch_visible_worker(review_mode=False)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except HarnessError as exc:
        log(f"ERROR {exc}")
        notify("SyncUp AI Harness stopped", str(exc), "critical")
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        raise SystemExit(1)
    except Exception as exc:  # unexpected failures should still be visible
        log(f"UNEXPECTED ERROR {type(exc).__name__}: {exc}")
        notify("SyncUp AI Harness error", f"{type(exc).__name__}: {exc}", "critical")
        print(f"UNEXPECTED ERROR: {type(exc).__name__}: {exc}", file=sys.stderr, flush=True)
        raise
