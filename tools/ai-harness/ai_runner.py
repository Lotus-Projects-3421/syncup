#!/usr/bin/env python3
import argparse
import datetime
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path.cwd().resolve()
BACKUP_ROOT = ROOT / ".ai-backups"

FORBIDDEN_PATH_PARTS = {".git", ".svn", ".hg", ".ssh", ".gnupg"}
FORBIDDEN_PATH_SUBSTRINGS = [".env", "secrets", "credential", "id_rsa", "id_ed25519", ".config/syncup", "wallet", "token"]

DANGEROUS_COMMAND_SUBSTRINGS = [
    "sudo", "rm ", "rm -", "rmdir", "git push", "git commit", "git reset --hard",
    "git clean", "git checkout", "git restore", "git rebase", "git merge", "git stash",
    "chmod -r", "chown -r", "mkfs", "dd if=", "> /dev/", "curl", "wget", "nc ",
    "ssh", "eval", "exec", ":(){", "shutdown", "reboot", ".config/syncup", ".ssh",
    ".gnupg", ".env", "secrets", "credential", "id_rsa"
]

SAFE_COMMAND_NAMES = {
    "cargo", "rustc", "npm", "pnpm", "yarn", "node", "git", "ls", "cat", "head",
    "tail", "rg", "grep", "find", "pwd", "echo", "python3", "make", "gio", "tree",
    "stat", "file", "diff", "patch"
}

def die(msg: str):
    print(f"ERROR: {msg}", file=sys.stderr)
    sys.exit(1)

def is_subpath(path: Path) -> bool:
    try:
        path.resolve().relative_to(ROOT)
        return True
    except ValueError:
        return False

def validate_path(p: str) -> Path:
    if not p: die("Empty path in payload.")
    if p.startswith("/") or p.startswith("~"): die(f"Absolute or home paths not allowed: {p}")
    parts = Path(p).parts
    if ".." in parts: die(f"Path traversal not allowed: {p}")
    if any(part in FORBIDDEN_PATH_PARTS for part in parts): die(f"Forbidden path component: {p}")
    lowered = p.lower().replace("\\", "/")
    for bad in FORBIDDEN_PATH_SUBSTRINGS:
        if bad in lowered: die(f"Forbidden pattern '{bad}' in path: {p}")
    target = (ROOT / p).resolve()
    if not is_subpath(target): die(f"Path escapes project root: {p}")
    return target

def ensure_git_repo() -> bool:
    return (ROOT / ".git").exists()

def git_dirty() -> bool:
    if not ensure_git_repo(): return False
    result = subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True, cwd=ROOT)
    return bool(result.stdout.strip())

def backup_file(rel: str, target: Path, stamp: str):
    if not target.exists(): return None
    dest = BACKUP_ROOT / stamp / rel
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(target, dest)
    return dest

def validate_command(cmd: str):
    for op in (";", "&", "|", "<", ">", "`", "$("):
        if op in cmd: die(f"Shell operator '{op}' not allowed: {cmd}")
    low = cmd.lower()
    for bad in DANGEROUS_COMMAND_SUBSTRINGS:
        if bad in low: die(f"Dangerous command blocked: {cmd}")
    try:
        argv = shlex.split(cmd)
    except ValueError as exc:
        die(f"Cannot parse command: {cmd}: {exc}")

    env_extra = {}
    while argv and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", argv[0]):
        key, value = argv[0].split("=", 1)
        env_extra[key] = value
        argv.pop(0)
    if not argv: die(f"No executable found: {cmd}")

    exe = Path(argv[0]).name
    if exe not in SAFE_COMMAND_NAMES:
        die(f"Command executable '{exe}' not in allowlist. Blocked.")

    return argv, env_extra

def run_command(cmd: str, log_path: Path) -> int:
    argv, env_extra = validate_command(cmd)
    env = os.environ.copy()
    env.update(env_extra)
    print(f"\n$ {cmd}")
    with log_path.open("a", encoding="utf-8") as log:
        log.write(f"\n$ {cmd}\n")
        try:
            proc = subprocess.Popen(argv, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        except FileNotFoundError:
            msg = f"command not found: {argv[0]}\n"
            print(msg); log.write(msg); log.write("EXIT=127\n")
            return 127
        for line in proc.stdout:
            print(line, end=""); log.write(line)
        proc.wait()
        rc = proc.returncode if proc.returncode is not None else 1
        print(f"EXIT={rc}"); log.write(f"EXIT={rc}\n")
        return rc

def extract_json(text: str):
    match = re.search(r"```(?:json)?\s*(\{.*\})\s*```", text, re.S)
    if match: return json.loads(match.group(1))
    start = text.find("{")
    end = text.rfind("}")
    if start == -1 or end == -1 or end <= start: die("No JSON object found.")
    return json.loads(text[start:end + 1])

def apply_patch(patch: str, dry_run: bool):
    if not patch.strip(): return
    if not ensure_git_repo(): die("Patch requires git.")
    patch_file = ROOT / ".ai-payload.patch"
    patch_file.write_text(patch, encoding="utf-8")
    check = subprocess.run(["git", "apply", "--check", str(patch_file)], cwd=ROOT, capture_output=True, text=True)
    if check.returncode != 0:
        patch_file.unlink(missing_ok=True)
        die(f"Patch failed check:\n{check.stdout}\n{check.stderr}")
    if dry_run:
        patch_file.unlink(missing_ok=True)
        print("Patch dry-run OK.")
        return
    result = subprocess.run(["git", "apply", str(patch_file)], cwd=ROOT, text=True)
    patch_file.unlink(missing_ok=True)
    if result.returncode != 0: die("Patch failed.")
    print("Patch applied.")

def main():
    parser = argparse.ArgumentParser(description="Safe local runner for AI payloads.")
    parser.add_argument("payload", help="JSON payload file.")
    parser.add_argument("--apply", action="store_true", help="Apply changes (default is dry-run).")
    parser.add_argument("--write-only", action="store_true", help="Apply files only.")
    parser.add_argument("--run-only", action="store_true", help="Run commands only.")
    parser.add_argument("--allow-dirty", action="store_true", help="Allow dirty git tree.")
    parser.add_argument("--yes", action="store_true", help="Skip prompts.")
    args = parser.parse_args()

    payload_path = Path(args.payload)
    if not payload_path.exists(): die(f"File not found: {payload_path}")

    text = payload_path.read_text(encoding="utf-8")
    payload = extract_json(text)

    summary = payload.get("summary", "")
    status = payload.get("status", "")
    files = payload.get("files", [])
    edits = payload.get("edits", [])
    patch = payload.get("patch", "")
    commands = payload.get("commands", [])
    if isinstance(commands, str): commands = [commands]

    print(f"AI payload: status={status}, summary={summary}")
    print(f"  files: {len(files)}, edits: {len(edits)}, patch: {'yes' if patch.strip() else 'no'}, commands: {len(commands)}\n")

    if git_dirty() and not args.allow_dirty:
        print("WARNING: git tree is dirty. Commit first or use --allow-dirty.")
        if not args.yes:
            ans = input("Continue anyway? [yes/N] ").strip().lower()
            if ans not in {"yes", "y"}: die("Stopped.")

    planned_files = []
    for item in files:
        if not isinstance(item, dict) or "path" not in item or "content" not in item: die("Bad files entry.")
        rel = str(item["path"]); target = validate_path(rel)
        planned_files.append((rel, target, str(item["content"])))

    planned_edits = []
    for item in edits:
        if not isinstance(item, dict) or not all(k in item for k in ("path", "find", "replace")): die("Bad edits entry.")
        rel = str(item["path"]); target = validate_path(rel)
        planned_edits.append((rel, target, str(item["find"]), str(item["replace"]), bool(item.get("replace_all", False))))

    dry = not args.apply
    if dry:
        print("DRY RUN: no changes will be made.\n")
        if planned_files: print("Would write:", [f[0] for f in planned_files])
        if planned_edits: print("Would edit:", [e[0] for e in planned_edits])
        if patch.strip(): apply_patch(patch, dry_run=True)
        if commands: print("Would run:", commands)
        return

    do_write = not args.run_only
    do_run = not args.write_only

    if do_write and (planned_files or planned_edits or patch.strip()):
        if not args.yes:
            ans = input("Apply file changes? [yes/N] ").strip().lower()
            if ans not in {"yes", "y"}: die("Cancelled.")

        stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
        for rel, target, content in planned_files:
            backup = backup_file(rel, target, stamp)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content, encoding="utf-8")
            print(f"WROTE {rel} (backup: {backup})" if backup else f"WROTE {rel}")

        for rel, target, find, replace, replace_all in planned_edits:
            if not target.exists(): die(f"Missing file for edit: {rel}")
            original = target.read_text(encoding="utf-8")
            count = original.count(find)
            if count == 0: die(f"Find text not found in {rel}")
            if count > 1 and not replace_all: die(f"Multiple occurrences in {rel}, use replace_all.")
            backup = backup_file(rel, target, stamp)
            new_content = original.replace(find, replace) if replace_all else original.replace(find, replace, 1)
            target.write_text(new_content, encoding="utf-8")
            print(f"EDITED {rel} (backup: {backup})" if backup else f"EDITED {rel}")

        if patch.strip(): apply_patch(patch, dry_run=False)

    if do_run and commands:
        log_path = ROOT / "ai-run-output.txt"
        log_path.write_text(f"# AI run {datetime.datetime.now().isoformat()}\n", encoding="utf-8")
        failed = []
        for cmd in commands:
            rc = run_command(cmd, log_path)
            if rc != 0: failed.append((cmd, rc))
        print(f"\nDone. Log: {log_path}")
        if failed:
            print("Failed:", failed)
            sys.exit(1)

if __name__ == "__main__":
    main()