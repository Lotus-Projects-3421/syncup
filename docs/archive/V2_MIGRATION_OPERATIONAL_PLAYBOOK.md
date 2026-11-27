# SyncUp v2 — Operational Playbook, Final Edition

This is the third and final revision. It incorporates every correction from the last review: the `tempfile` bug in Phase 4, the HTTPS pinning reality, the `pipefail` shell issue, the duplicate-manifest crash window, the dev-vs-production privacy split, the git recovery safety, the source abstraction gap, and every smaller correction. **Freeze this document. Do not revise it again.** The next meaningful milestone is a successful Phase 0 audit, not a fourth prompt-engineering round.

---

## Part 0 — Summary of changes in this edition

| # | Area | Final decision |
|---|---|---|
| 1 | Phase 4 file publication | `NamedTempFile::persist(final)` publishes; then `fsync(dest_dir)` on POSIX. On Windows use `ReplaceFileW` or `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` because `persist()`'s behaviour on replace differs per platform — verify against the current `tempfile` crate version before implementing. **Directory fsync happens *after* publish, not before.** |
| 2 | Wi-Fi transport | Ship **HTTP with a scoped, short-lived, single-device session** as the initial v2.0 transport. Make the residual risk explicit in the UI and docs. Native pinning is deferred to a possible v2.x Android companion app. |
| 3 | Wi-Fi tokens | CSPRNG, ≥128 bits, in-memory only, never in URLs or logs, one session = one device, visible stop-session control, reject expired, do **not** rely on Origin as authentication. |
| 4 | A4 test baseline | `set -o pipefail` + `tee` to a log; extract the summary from the full log, not the tail. |
| 5 | Duplicate manifest | Full operation list written **before** first move, fsynced, per-op IDs with `pending/completed/failed` state, reconciled on startup, collision-aware idempotent undo. |
| 6 | Privacy | Two distinct rule sets: **development/test** (never touch real data) vs **production migration** (app may, only with user approval, on a snapshot). |
| 7 | Git recovery | `git stash -u -m "..."` for set-aside; `git restore --worktree -- <path>` for path-targeted revert. Never `git checkout -- .`, never `git reset --hard`, never `git clean -fd` without a dry-run. |
| 8 | File-ops interface | `Source` abstraction (file or stream) + validated `Destination` + explicit verification mode. All features use the same engine. No per-feature writers. |
| 9 | Failure handling | Explicit sequence: **agent reports and stops → human reviews → human authorizes specific fix → agent fixes only that → re-runs gate.** No unsupervised fix loops. |
| 10 | Standing Rules | Expanded with all new safety requirements. Resolved contradictions: Python moves only via Phase 1 plan and a human commit; production migration is a separate, user-initiated app workflow; agent never commits, but may write PROGRESS.md. |

---

## Part 1 — Principles

1. **Audit in place.** Do not move, rename, or "tidy" the Python project until Phase 1's plan explicitly says so, and then only through a human-run `git mv` in one commit.
2. **The old Python app is the behavioural oracle.** Every non-trivial behaviour it implements correctly must be preserved. Deviations are documented, not silent.
3. **The coding agent never sees real user data.** Development and tests use synthetic fixtures. Production migration is a separate, user-initiated app workflow.
4. **Destructive actions require a human gate.** DB migration, file replacement, duplicate movement, any `git` operation — human-visible, human-approved.
5. **`PLAN.md` is the contract.** Coding sessions execute it. If the plan is wrong, stop and revise the plan in web chat first.
6. **Verification means raw output.** "Passed" without paste is "not run".
7. **Failure means stop, not guess.** A failed gate triggers a review cycle, not an unsupervised fix attempt.
8. **Never fabricate.** Every shipped feature carries an honest status: *verified / implemented-not-tested / partial / not implemented*.

---

## Part 2 — One-time setup

### 2.1 System packages (Zorin / Ubuntu — verify against the current Tauri 2 prerequisites before running)

```bash
sudo apt update
sudo apt install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  pkg-config git
```

**Do not** blindly `apt install gio`. `gio` is part of `libglib2.0-bin` on most Debian-family systems. Verify:

```bash
command -v gio && gio --version
```

If `gio` is missing, install `libglib2.0-bin`. Record its version — it matters for Linux MTP and the audit will need it.

### 2.2 Toolchains — record exact versions

```bash
# Rust: current stable
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
rustup update stable
rustc --version    # record: e.g. rustc 1.8x.y
cargo --version    # record

# Node: current LTS (the "LTS" line on nodejs.org, not "Current")
curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.1/install.sh | bash
source ~/.bashrc
nvm install --lts
node --version     # record: e.g. v22.x
npm --version      # record
```

**Write these four version strings into `docs/PROGRESS.md` under "Pinned toolchain".** After Phase 2 succeeds, add `rust-toolchain.toml` and `.nvmrc` pinning these same versions so builds are reproducible.

Do not install a global `tauri-cli`. Tauri 2 uses the project-local `@tauri-apps/cli` dev dependency added in Phase 2.

### 2.3 Repo preparation — no moves

```bash
cd ~/path/to/syncup

git status                          # MUST be clean; commit or stash first
git branch -a                       # inspect before creating anything
git checkout -b v2-migration        # ONLY if it does not already exist

# Inspect existing docs/ and tests/fixtures/ before creating files
ls -la docs/ 2>/dev/null || true
ls -la tests/ 2>/dev/null || true

# Save the spec verbatim — do NOT overwrite if docs/ already has files
$EDITOR docs/SPEC.md                # paste the full spec

git add docs/SPEC.md
git commit -m "docs: add SyncUp v2 engineering specification (frozen)"
```

Create tracking stubs — **only if the file does not already exist**:

```bash
[ -f docs/AUDIT.md ]    || cat > docs/AUDIT.md <<'EOF'
# SyncUp v1.4.1 — Project Audit

> Generated during Phase 0. Appended in chunks A1–A4. Do not edit manually.
EOF

[ -f docs/PLAN.md ]     || cat > docs/PLAN.md <<'EOF'
# SyncUp v2 — Migration Plan

> Generated during Phase 1. Do not edit manually.
EOF

[ -f docs/PROGRESS.md ] || cat > docs/PROGRESS.md <<'EOF'
# SyncUp v2 — Progress Log

## Current phase: 0 (Audit)
## Completed phases: —
## Last commit: —

## Pinned toolchain
- rustc: (fill in)
- cargo: (fill in)
- node: (fill in)
- npm: (fill in)

## Handoff for next session
- Paste this section at the top of the next OpenCode session.
- State: nothing implemented yet.
- Next action: run Audit chunk A1.
EOF

git add docs/
git commit -m "docs: add audit, plan, and progress tracking files"
```

### 2.4 Sanitized fixtures folder

```bash
mkdir -p tests/fixtures/legacy
[ -f tests/fixtures/legacy/README.md ] || cat > tests/fixtures/legacy/README.md <<'EOF'
# Sanitized legacy fixtures

Contains synthetic or redacted copies of:
- settings.json
- profile_<id>.json
- state_<id>.json
- state_<id>.db (schema-accurate, fake rows)

NEVER place real user data here. No paths from ~/.config/SyncUp/.
EOF

git add tests/fixtures/
git commit -m "test: add sanitized legacy fixture folder"
```

---

## Part 3 — Standing Rules (paste at the top of **every** OpenCode prompt)

This block is canonical. Copy it verbatim every time. It supersedes any conflicting instruction in a phase prompt.

```
=== STANDING RULES — SyncUp v2 migration ===

AUTHORITY
- docs/SPEC.md is the source of truth for requirements.
- docs/PLAN.md is the contract for implementation. Do not re-plan.
- docs/PROGRESS.md holds the current state. Read it before acting.

PROTECTED FILES
- The Python app (main.py, core/, views/, tests/, assets/, packaging/, scripts/,
  SyncUp.spec, build.sh, pyproject.toml, requirements.txt, README.md, CHANGELOG.md)
  is the behavioural oracle. Do NOT move, rename, or delete any of it.
  Relocation is allowed ONLY as an explicit, human-approved step in the Phase 1 plan,
  executed by the human with `git mv`, not by you.

USER DATA
- Development and testing: NEVER inspect, read, print, or modify ~/.config/SyncUp/
  or any real user configuration. Use only synthetic fixtures under tests/fixtures/.
- Never expose real user data, tokens, credentials, or private paths in any output,
  log, or documentation.
- Production migration (the finished application) is a separate, user-initiated
  workflow. You do not implement ad-hoc access to real data — you implement the
  application-level migration feature, which itself runs only on user approval.

GIT
- Never commit, push, tag, or create branches. The human controls Git.
- Never discard tracked or untracked changes (no `git reset --hard`, no
  `git checkout -- .`, no `git clean` without dry-run approval).
- If a context limit is approaching, update docs/PROGRESS.md and stop.
  The human decides whether to create a WIP commit.

SCOPE DISCIPLINE
- Inspect every file's current contents before editing it.
- Before touching >1 file or any DB schema or any file-publication logic:
  show the plan and wait for "go".
- Out-of-scope work is refused. If the plan is ambiguous, STOP and say so.

CODE QUALITY
- TypeScript: strict, no `any`, no non-null `!` without justification, no `// @ts-ignore`.
- Rust: no `unwrap()` / `expect()` / `panic!()` in non-test production paths.
  Use Result and a structured IpcError.
- No CPU-heavy work on async runtime threads. Blocking work uses dedicated tasks.
- File publication must be followed by the platform-appropriate durability step
  BEFORE the database records success.
- Duplicate operations must be recoverable after a crash, including a crash between
  moving a file and updating its manifest.
- Never claim a cross-platform feature works based only on Linux tests.

VERIFICATION
- Never fabricate results. If you didn't run a command, write "not run".
  If a test failed, paste the failure verbatim.
- A failed verification gate means STOP and report the exact failure.
  Do not attempt unrelated fixes without explicit human approval.
- Raw output of every verification command must be pasted in chat.

SAFETY INVARIANTS (definitional — these define SyncUp)
- Never delete a user photo or video.
- Never remove or replace a destination file before the new copy is written AND
  verified.
- Never mark a file "backed up" before verified atomic publish + durability + DB
  commit, in that order.
- Never expose an unrestricted upload endpoint.
- SQLite backups use SQLite's backup mechanism (rusqlite Connection::backup or
  VACUUM INTO), never a plain file copy. WAL is possible; handle it.
=== END STANDING RULES ===
```

---

## Part 4 — Prompt library

### 4.1 PROMPT A — Audit (four sessions, each appends to `docs/AUDIT.md`)

Do not audit the whole repo in one session. Use DeepSeek V4.1 Flash (200K) unless you have *verified* that your OpenCode honours LongCat's larger window; chunking makes the model choice non-critical.

After each chunk: `git add docs/AUDIT.md && git commit -m "docs: audit chunk AN"`. Open a **new** session for the next chunk.

---

#### PROMPT A1 — Structure, entry point, dependencies

```
[PASTE STANDING RULES BLOCK]

TASK — PHASE 0, AUDIT CHUNK A1 of 4. Do NOT modify any application file.
Your only output is an APPENDED section in docs/AUDIT.md plus a short chat summary.

Scope for this chunk ONLY:

1. Paste raw output of: git status, git log --oneline -20, git branch -a.
2. Print a directory tree of repo root, depth 2, excluding .git/, node_modules/,
   .venv/, __pycache__/, build/, dist/.
3. Identify the actual Python entry point. Read ONLY that file fully (typically main.py).
   Summarize: how the app starts, initializes, launches the UI.
4. Read pyproject.toml (if present) and requirements.txt. List every dependency with its
   pinned version and classify it: UI / DB / HTTP / image-metadata / QR / EXIF /
   packaging / other.
5. Read README.md and CHANGELOG.md (top 50 lines). Extract: claimed feature list,
   known issues, and the exact latest version string.
6. Locate test configuration (pytest.ini, tox.ini, pyproject [tool.pytest],
   .github/workflows/*.yml that run tests). Do NOT run tests — that is A4.
7. Locate packaging configuration (SyncUp.spec, build.sh, .github/workflows for releases).
   Summarize how the old app is built and shipped.

Write to docs/AUDIT.md:
## A1.1 Repository state
## A1.2 Entry point
## A1.3 Dependencies
## A1.4 Claimed features (README/CHANGELOG)
## A1.5 Test configuration
## A1.6 Build/packaging configuration

In chat, report:
- Confirmed app version string.
- Confirmed entry point file.
- Any discrepancy between README claims and files you actually saw.
- Anything you could not find.

Do not proceed to A2. Do not edit any file other than docs/AUDIT.md.
```

---

#### PROMPT A2 — Core logic and safety invariants

```
[PASTE STANDING RULES BLOCK]

CONTEXT: docs/AUDIT.md already contains A1.x. Read it first, then continue.

TASK — PHASE 0, AUDIT CHUNK A2 of 4. Do NOT modify any application file.

Scope (core business logic only; do not touch views/ or tests/ here):

Read every file inside core/ (or equivalent — find it first). For each file, in 2–4 lines:
- Its responsibility.
- Public functions it exposes (name + one-line purpose).
- Any invariants or safety checks it enforces.

Then specifically locate and QUOTE WITH FILE:LINE, if they exist:
1. Atomic file replacement: where a temp file is created, written, verified,
   fsync'd, and renamed. If it uses plain shutil.copy/copy2 without temp+rename,
   SAY SO explicitly.
2. Content verification: hashing (which algorithm? which library?), size checks,
   any compare step.
3. Backup-state writes: where files are marked "backed up" in DB or JSON, and
   whether that write happens AFTER the file is safely published.
4. Conflict handling: Keep Both / Overwrite / Skip — where implemented, and the
   code path each takes.
5. Duplicate finder: hash strategy (quick vs full), algorithm names, library.
6. Takeout / organizer: JSON parsing, EXIF library, date precedence chain.
7. Naming rules: how destination filename is computed; whether it is idempotent.
8. Cancellation: how a running operation is stopped; at which boundaries it checks.

Write to docs/AUDIT.md:
## A2.1 Core module inventory (table: file | responsibility | public functions)
## A2.2 Safety invariants as implemented (with file:line quotes)
## A2.3 Weaknesses and risks in core logic
## A2.4 Behaviours that must be preserved in the rewrite (bulleted, testable)

In chat, report:
- The 3 most important invariants to preserve.
- The 3 most concerning weaknesses.
- Anything you could not find.

Do not proceed to A3. Do not edit any file other than docs/AUDIT.md.
```

---

#### PROMPT A3 — Database schema, config formats, migration surface

```
[PASTE STANDING RULES BLOCK]

CONTEXT: docs/AUDIT.md has A1 and A2. Read them first.

TASK — PHASE 0, AUDIT CHUNK A3 of 4. Do NOT modify any application file.

CRITICAL PRIVACY RULE: Do NOT read ~/.config/SyncUp/. Do NOT read any real
settings.json, profile_*.json, or state_*.db. Determine formats ONLY from source code
that reads/writes them. If you need an example, write a synthetic one clearly labeled
"SYNTHETIC". Also: identify the exact directory where legacy JSON files live (relative
to the SQLite DB path) so Phase 3 knows what to import.

Scope:

1. SQLite schema. Find where the DB is created / migrated (search CREATE TABLE,
   executescript, schema version constants, migration functions). Paste the FULL DDL
   of every table (columns, types, constraints, defaults) and every CREATE INDEX.
   Describe any schema-versioning mechanism (PRAGMA user_version, migrations table).
2. WAL mode. Search for PRAGMA journal_mode and sqlite3 connection setup. Report
   whether WAL is enabled and where. This determines the safe backup method.
3. Config file formats. From source code, derive the exact JSON shape of:
   - settings.json
   - profile_<id>.json
   - state_<id>.json (legacy)
   For each: list every key, its type, default, and what writes it. Provide SYNTHETIC
   examples clearly labeled.
4. File-location report. Document the exact directory path where these files live
   relative to the DB, so the migration plan can locate them in production.
5. Migration surface. Find any existing migration code (JSON → SQLite, or schema
   upgrades). Describe what it does and whether it is transactional, idempotent,
   and safe on interruption.
6. Backup-state queries. Identify the exact queries that (a) check "is this file
   already backed up" and (b) insert a "backed up" record. Quote them with file:line.
7. Naming/dedup state. If there is a table or structure for processed files,
   describe its columns, indexes, and whether they support the lookups the app does.

Write to docs/AUDIT.md:
## A3.1 SQLite schema (full DDL)
## A3.2 WAL and journal mode findings
## A3.3 Configuration file formats (synthetic examples)
## A3.4 Legacy file locations (for migration)
## A3.5 Existing migration logic
## A3.6 Backup-state queries (file:line)
## A3.7 Data-safety risks for v2 migration

In chat, report:
- Is WAL enabled? (yes/no/unknown)
- Total tables, and the two most-used lookups.
- The two riskiest aspects of migrating the legacy DB.
- Anything you could not determine.

Do not proceed to A4. Do not edit any file other than docs/AUDIT.md.
```

---

#### PROMPT A4 — Views, Wi-Fi server, tests (with **correct** baseline run)

```
[PASTE STANDING RULES BLOCK]

CONTEXT: docs/AUDIT.md has A1–A3. Read them first.

TASK — PHASE 0, AUDIT CHUNK A4 of 4. Do NOT modify any application file.

Scope:

1. Views. For each file in views/ (or equivalent): one-line responsibility, one-line
   note on user-visible features exposed. Do NOT list Flet boilerplate.
2. Wi-Fi server. Find the FastAPI/HTTP server module. Read it fully. Report:
   - Route list with methods and purposes.
   - Whether any authentication/token/session mechanism exists (quote the code or
     state "absent").
   - Whether HTTPS is used (say so plainly if it is not).
   - Upload handling: destination directory, filename sanitization, size limits,
     path-traversal protections (quote or say absent).
   - Manifest-comparison flow.
   - Session lifecycle: who starts it, how it stops, timeout behaviour.
3. Cross-platform usage. Search for sys.platform, platform.system(), Windows/macOS
   specific code. Report what is Linux-only today.
4. Tests. Enumerate test files and test function names. THEN run the suite with
   pipefail preserved so failures are not masked:

     set -o pipefail
     python -m pytest -q 2>&1 | tee /tmp/syncup-pytest-baseline.log | tail -40

   If the project uses a different test command, use the one in README/CI, still
   with `set -o pipefail` and `tee`.
   If deps are missing: say so explicitly and list them. Do NOT install anything
   without asking.
   Extract the SUMMARY LINE from the FULL log (grep for "passed" / "failed" / "error"
   lines), not from the last 40 lines. Record it verbatim as the BASELINE.
5. Feature inventory. Table: Feature | Files | Evidence it works (test? manual?
   unknown?) | Notes. Cover: USB/MTP, Wi-Fi, Takeout organizer, duplicate finder,
   settings, profiles, theming, window persistence.

Write to docs/AUDIT.md:
## A4.1 Views inventory
## A4.2 Wi-Fi server audit (with security gaps)
## A4.3 Platform-specific code
## A4.4 Test baseline (raw summary line + full log path)
## A4.5 Feature inventory (final table)
## A4.6 Top 10 migration risks

In chat, report:
- The exact baseline summary line verbatim (or "tests did not run, reason: …").
- The three biggest Wi-Fi security gaps.
- Anything you could not determine.

End of audit. Do not proceed to Phase 1.
```

---

### 4.2 PROMPT B — Architecture and migration plan (web chat)

Paste this into a large-context web chat (DeepSeek web / LongCat web), then follow with the full SPEC and AUDIT.

```
You are a Senior Cross-Platform Application Engineer (Tauri 2, Rust, Svelte 5, TS,
Tailwind v4, shadcn-svelte, Vite). I am migrating a Python/Flet application,
SyncUp v1.4.1, into a Tauri 2 + Rust + Svelte 5 rewrite, v2.0. Your output is
docs/PLAN.md — an operational plan I will follow phase by phase.

You are given:
(1) The full specification (docs/SPEC.md).
(2) The complete audit of the current code (docs/AUDIT.md).

Rules:
- Use the audit as the sole source of truth about the current code. If the audit
  does not say something, do not invent it. Mark "unknown, needs follow-up".
- Produce the SIMPLEST architecture that satisfies the spec. No unnecessary
  abstractions.
- Never fabricate crate names, version numbers, or APIs. If unsure, write
  "verify at install time".
- Reference file paths relative to repo root.
- Output docs/PLAN.md as a single markdown code block.

docs/PLAN.md MUST contain, in this order:

§1 Target architecture overview (backend service map, frontend structure,
   IPC boundary, storage layer). One paragraph each.

§2 Target folder tree with one-line purpose per folder.

§3 Migration matrix: every legacy module from AUDIT §A2.1 → new Rust/Svelte
   location → status (preserve / rewrite / drop / new), with a "preserve with
   reason" column so behaviours that must not change are explicitly tagged.

§4 Typed IPC design. Every Tauri command with: name, input struct, output type,
   IpcError codes. Note: tauri-specta generates TS bindings; Rust commands are
   the source of truth; hand-edited bindings are forbidden. Include a CI check
   that regenerates bindings and diffs them.

§5 SQLite schema v2.
   - Full DDL, indexes, PRAGMA user_version bump.
   - Migration plan that:
     * Uses rusqlite's Connection::backup OR `VACUUM INTO` to produce a
       WAL-safe snapshot. NEVER a plain file copy — state in one sentence why
       (WAL can hold recent commits outside the .db file).
     * Verifies the snapshot with PRAGMA integrity_check. Refuse to proceed if
       not "ok".
     * Preserves the original file; never modifies it.
     * Imports legacy settings/profiles/JSON state in a single transaction.
     * Is idempotent and safe on interruption.
     * Documents a rollback procedure.
   - Explicit thread-safety plan for DB access from Tauri State:
     r2d2_sqlite pool OR Mutex<Connection>, with justification.

§6 File-safety invariants as pseudocode, covering:
   - Source abstraction: an enum (or trait) supporting BOTH an ordinary file
     path AND a byte stream (USB/MTP and Wi-Fi uploads supply streams).
   - Destination: validated final path + explicit filename + conflict mode.
   - Procedure per file:
       a. Validate source, destination, and disk space.
       b. Create NamedTempFile in the DESTINATION directory.
       c. Stream-copy (buffered, no whole-file read).
       d. flush(); fsync(temp file).
       e. Verify size (always); verify content (BLAKE2b) when mode requires.
       f. Publish atomically:
            POSIX: NamedTempFile::persist(final). Confirm the current tempfile
                   crate version's exact persist semantics.
            Windows: ReplaceFileW or MoveFileExW(MOVEFILE_REPLACE_EXISTING |
                   MOVEFILE_WRITE_THROUGH). Explain why fs::rename and
                   persist() alone are insufficient for durability on Windows.
       g. fsync(destination directory) on POSIX. Document the Windows
         durability strategy.
       h. If publish fails: existing destination untouched.
       i. If publish succeeds but durability fails: file is at final path;
         state = "publish-pending-durability"; caller decides recovery.
       j. Database update ONLY after durability step completes.
   - Cancellation: checked at file boundaries; temp file cleaned up.
   - Symlink policy: refuse to follow untrusted symlinks; document the check.
   - Concurrent jobs targeting the same destination: serialized or
     conflict-detected; no two writers to the same path.
   - Crash recovery: orphan temp files identified and cleaned on startup.

§7 Wi-Fi security model. Final decision (no "TBD"):
   - Choose HTTP with a scoped, short-lived, single-device session as the v2.0
     transport. Justify the choice (native pinning is impractical without a
     native Android client). State the residual risk plainly: bearer tokens
     and traffic are observable to any device that can monitor the LAN. Do
     NOT describe this as secure against that threat.
   - Token: CSPRNG, ≥128 bits, in-memory only, never persisted, never in
     URLs or logs, single-use per session, invalidated on expiry, explicit
     stop, or app exit.
   - One device per session. Additional devices are rejected until the
     session is stopped.
   - Per-session staging directory; isolated between sessions.
   - Filename sanitization: reject `..`, absolute paths, path separators,
     control characters, reserved names.
   - Request size cap and per-session total cap; rate limit.
   - Host/Origin checks on every request, but NOT as the sole authentication
     mechanism — the bearer token is the authentication.
   - Bind to a specific LAN interface by default. 0.0.0.0 requires an explicit
     user opt-in with a warning.
   - Visible stop-session control in the UI. On app exit, cancel in-flight
     uploads and remove the staging directory.
   - Test requirements listed explicitly: expired session rejected, wrong
     token rejected, missing token rejected, path traversal rejected,
     oversized rejected, origin mismatch rejected, rate limit enforced,
     session isolation, cleanup on shutdown, token never logged, session
     reuse rejected.

§8 Implementation plan for phases 2–10. For each phase:
   - Scope (in/out).
   - Exact files to create/modify.
   - Verification commands (state that each command must exist — e.g. verify
     scripts in package.json before assuming `npm run X`).
   - Definition of done.
   - What must NOT be deferred to a later phase.

§9 Testing plan: unit, integration, manual, per phase. For every test that
   touches legacy data, state "uses copies / fixtures, never real data".

§10 Platform matrix. Linux: required and verified. Windows/macOS: planned,
    code-ready but not verified until built and run on those OSes. Android:
    evaluated — likely APK of a companion client only if scope demands it,
    otherwise "not supported in v2.0".

§11 Known limitations and explicit "not in v2.0" list.

§12 Rollback and recovery: how to abandon the migration, keep using the old
    app, and restore any touched state.

--- SPEC ---
[PASTE docs/SPEC.md IN FULL]

--- AUDIT ---
[PASTE docs/AUDIT.md IN FULL]
```

**Critique before accepting.** Checklist:

- §5 uses `Connection::backup` or `VACUUM INTO`, never a file copy.
- §6 specifies `persist()` + directory fsync order correctly, and names the Windows API.
- §6 defines a source abstraction that handles streams (MTP/Wi-Fi).
- §7 makes a concrete HTTP decision with explicit residual risk.
- §7 forbids tokens in URLs and logs.
- §8 places Phase 4 (file engine) before Phases 5–8.
- §10 does not claim Windows/macOS as verified.

If wrong, re-prompt with "revise §X: …". Commit only when satisfied:

```bash
git add docs/PLAN.md
git commit -m "docs: phase 1 architecture and migration plan"
```

**This is the last chance to move the Python app** — and only if PLAN.md says so, and only via a human-run `git mv` in one commit. Otherwise leave it in place.

---

### 4.3 PROMPT C — Implementation (one fresh session per phase)

```
[PASTE STANDING RULES BLOCK]

CONTEXT — READ BEFORE ACTING
- docs/SPEC.md sections relevant to this phase.
- docs/PLAN.md §N (this phase). Do not re-plan.
- docs/PROGRESS.md "Handoff". This is the state you inherit.
- The current contents of every file you will modify.

If PLAN.md is ambiguous for this phase, STOP and say so. Do not invent scope.

CURRENT PHASE: [N — name from PLAN.md]
GOAL: [one sentence from PLAN.md]
IN SCOPE:
[bullets from PLAN.md §8 phase N "in scope"]
OUT OF SCOPE (do not touch):
[bullets from PLAN.md §8 phase N "out of scope"]
FILES TO CREATE OR MODIFY:
[list from PLAN.md]
VERIFICATION GATE (all must run; paste raw output for each):
[commands from PLAN.md §8 phase N]

PROCEDURE
1. Pre-flight: read files; print "current state / what I'll change" summary.
2. For >1 file, any DB schema, or any file-publication logic: show the planned
   diff and WAIT for my "go".
3. Implement in small steps. After each step, run the fastest relevant check
   (cargo check / npm run check / tsc --noEmit) and fix before continuing.
4. Run the full gate at the end. Paste RAW output per command.
5. If any gate command fails: STOP. Report the exact failure. Do not attempt a
   fix until I authorize a specific one. Do not change unrelated code to make a
   failing test pass.
6. Update docs/PROGRESS.md:
   - Current phase = this phase.
   - Add to "Completed phases" ONLY if every gate command passed.
   - Handoff section for the next session: what exists now, what does not, any
     gotchas, and which files the next session must read first.
7. STOP. Do not start the next phase. Do not commit.
```

Phase fill-ins follow. Each carries the specific requirements baked in.

---

#### C2 — Phase 2: Foundation & App Shell

```
GOAL: A Tauri 2 + Svelte 5 + TS strict + Tailwind v4 + shadcn-svelte app that
launches on Linux with a navigation shell, theme system, typed-IPC skeleton,
error/log foundation, and a DB connection + PRAGMA user_version scaffold.
No application features wired yet.

IN SCOPE
- Scaffold Tauri 2 at the repo root (src/, src-tauri/) without touching Python files.
- Add @tauri-apps/cli as a devDependency. Verify scripts in package.json before
  assuming `npm run tauri` exists.
- Vite + Svelte 5 + TS strict (tsconfig: strict, noImplicitAny; disallow `any`).
- Tailwind CSS v4 via the official Vite plugin. shadcn-svelte initialized.
- tauri-specta: two trivial commands (app_version, health_check). Add
  `npm run gen:bindings`. Bindings path: src/lib/ipc.ts.
- Add a check script that regenerates bindings and diffs them
  (`git diff --exit-code src/lib/ipc.ts`) — CI uses the same script.
- App shell: sidebar nav (Dashboard, Phone Backup, Wi-Fi Backup, Organizer,
  Duplicates, Activity, Settings), theme dark/light/system, system fonts, blue
  accent. Accessible controls, keyboard nav.
- Rust: IpcError, logging, AppState containing a DbPool (r2d2_sqlite) opened at
  the platform data dir. PRAGMA user_version only — no tables yet.
- Tauri capabilities: minimal, per-window.
- Add rust-toolchain.toml and .nvmrc pinning the exact versions recorded in
  PROGRESS.md under "Pinned toolchain".

OUT OF SCOPE
- No feature logic, no file engine, no migrations, no Wi-Fi server.

VERIFICATION GATE
- npm run check                (must be defined in package.json; paste raw output)
- npm run build
- cargo fmt --check
- cargo check
- cargo clippy -- -D warnings
- cargo test
- npm run gen:bindings && git diff --exit-code src/lib/ipc.ts   (must be clean)
- npm run tauri dev            (launches; report window title and console errors)
```

---

#### C3 — Phase 3: Storage & Legacy Data Migration

```
GOAL: Schema v2 present in a NEW database, with a verified migration path from a
COPY of the legacy SQLite DB and legacy JSON state. Original DB is never modified.

IN SCOPE
- Create new DB at the platform data dir. Apply schema v2 DDL + PRAGMA user_version = 2.
- Migration command migrate_legacy_import(source_db_path, source_json_dir):
  1. Open source DB read-only.
  2. Produce a WAL-safe snapshot into a timestamped file using rusqlite's
     Connection::backup (or VACUUM INTO). NEVER a file copy. Include a code
     comment stating why (WAL can hold recent commits outside the .db file).
  3. PRAGMA integrity_check on the snapshot. Refuse to proceed if not "ok".
  4. Open snapshot read-only, read rows.
  5. Insert into v2 DB in a single transaction.
  6. Verify counts match source, or document skipped items.
  7. Log each step with a stable prefix so tests can assert.
- source_json_dir is read from AUDIT §A3.4. The caller (Phase 9 UI) passes it
  after user confirmation; do NOT hardcode paths.
- migrate_legacy_import is idempotent: detect "already imported" via a marker in
  v2 DB and skip.
- Import legacy settings.json / profile_<id>.json / state_<id>.json in the same
  transaction, or split into independently idempotent transactions per PLAN.md §5.
- Read commands: get_migration_status, get_settings, list_profiles.
- Unit tests using synthetic fixtures under tests/fixtures/legacy/.

OUT OF SCOPE
- Do not delete or overwrite the source DB. Do not move files. Do not access
  ~/.config/SyncUp/ during development or tests.

VERIFICATION GATE
- cargo test  (paste all test names; must include: fresh_import,
  idempotent_rerun, interrupted_import_resume, corrupt_snapshot_rejected,
  wal_source_handled, source_unchanged_after_migration)
- cargo clippy -- -D warnings
- Manual: run against a synthetic fixture DB. Capture the migration log. Confirm
  source file's mtime and SHA-256 are unchanged before and after (paste both).
```

---

#### C4 — Phase 4: File Safety Engine and Jobs

```
GOAL: The atomic, verified file-operation and job foundation that Phases 5–8
build on. This is the most important phase in the project. Take the time.

IN SCOPE

File-op interface (all features use this — no per-feature writers):
- Define a source abstraction:
    pub enum Source {
        File(PathBuf),
        Stream { reader: Box<dyn Read + Send>, size_hint: Option<u64> },
    }
- Define a validated destination:
    pub struct Destination {
        final_path: PathBuf,          // validated: no .., no symlink to outside, no reserved names
        conflict_mode: ConflictMode,  // KeepBoth | Overwrite | Skip
        expected_size: Option<u64>,   // required for Stream sources
        verify: VerifyMode,           // SizeOnly | SizeAndHash
    }
- Public entry point:
    pub async fn copy_verified(src: Source, dest: Destination, cancel: CancellationToken)
        -> Result<Published, FileOpError>
- `Published` only exists when the file is at its final path AND the platform
  durability step completed.

Copy procedure per file (implement exactly this order):
  1. Validate source, destination, free space.
  2. NamedTempFile::new_in(dest_dir) — temp file in destination directory.
  3. Stream copy (buffered).
  4. flush(); fsync(temp_file).
  5. Verify size. If VerifyMode::SizeAndHash, compute BLAKE2b of temp file and
     compare with source hash (for File sources, hash the source; for Stream
     sources, hash while streaming and compare at end).
  6. Publish:
       POSIX: NamedTempFile::persist(final_path). Confirm current tempfile crate
              version's persist semantics. Comment what persist() does and does
              not guarantee.
       Windows: ReplaceFileW or MoveFileExW(MOVEFILE_REPLACE_EXISTING |
              MOVEFILE_WRITE_THROUGH). fs::rename and persist() alone are
              insufficient for durability on Windows — write this in a comment.
  7. fsync(destination directory) on POSIX. Document Windows durability
     strategy in a module-level comment.
  8. If publish fails: existing destination untouched; return an error; temp
     file cleaned up.
  9. If publish succeeds but durability (fsync) fails: return a distinct
     "PublishPendingDurability" error whose payload includes the final_path.
     Caller must NOT update the DB in this state — it must retry durability
     or surface to the user.
 10. Only after durability: return Ok(Published { final_path, ... }). Caller
     updates DB.

Concurrency:
- Two concurrent jobs targeting the same destination path must be prevented.
  Use a per-destination-path lock (a global map of path → Mutex, held in State).
- Same-run collisions for KeepBoth: deterministic counter (name (1).ext) that
  does not depend on filesystem state.

Safety:
- Refuse to follow symlinks that point outside approved destination roots.
- Preflight disk-space check.
- Map disk-full, permission-denied, and I/O errors to distinct IpcError codes.

Cancellation:
- CancellationToken checked at file boundaries. Temp file removed on cancel.
- Cancellation never corrupts an existing destination.

Jobs:
- JobId (UUID), typed progress events (bounded: max 10/sec), status and cancel
  commands. Job registry in State.

Crash recovery:
- On startup, scan destination root for orphan temp files matching the engine's
  pattern; log and remove them. Do not touch non-matching files.

OUT OF SCOPE
- No USB, Wi-Fi, organizer, or duplicate features. No new UI screens beyond the
  shell.

VERIFICATION GATE
- cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test
- Tests MUST include (name them in the output):
    copy_success_file_source
    copy_success_stream_source
    copy_size_mismatch_rejected
    copy_content_mismatch_rejected
    overwrite_existing_replaces_verified
    overwrite_existing_preserved_on_publish_failure
    overwrite_existing_preserved_on_verify_failure
    disk_full_error_mapped
    permission_denied_mapped
    cancel_mid_copy_leaves_no_temp
    cancel_preserves_existing_dest
    symlink_outside_root_rejected
    concurrent_same_destination_serialized
    same_run_collision_deterministic
    orphan_temp_cleanup_on_startup
    publish_pending_durability_state_reported
  Plus a cfg(windows)-only test compiled but not run on Linux — state this
  explicitly.
- Report exact test count and names.
```

---

#### C5 — Phase 5: Phone Backup (Linux MTP)

```
GOAL: USB/MTP backup on Linux at parity with the Python app, built on the
Phase 4 engine.

IN SCOPE
- platform/linux: device discovery and folder listing via `gio` invoked with
  argument arrays (no shell strings). Lazy-mount trigger.
- File streaming out of GVFS piped into Source::Stream of the Phase 4 engine.
- Device profiles stored in the v2 DB (Phase 3).
- Media filter (image/video extensions, configurable per profile).
- Date-based sorting; already-backed-up and changed-file detection using
  size + mtime first, content hash only when needed.
- Overlapping selected folders handled once per file.
- Free-space preflight.
- Job + progress + cancellation wired through the engine.

OUT OF SCOPE
- No Windows or macOS device code. Non-Linux builds return an explicit
  Unsupported error pointing to the Wi-Fi workflow.

VERIFICATION GATE
- cargo test && cargo clippy -- -D warnings && npm run check
- Manual, with a real Android phone (unlocked, USB File Transfer):
  - list device, list folders, back up ≥3 files, cancel mid-run, rerun to
    confirm skip.
  - Test actual GVFS/MTP behaviour end to end — not mocked gio output.
- Report exactly what was tested on real hardware vs simulated. If no phone was
  available, say so; the phase is not "complete" until a real device test has
  been done.
```

---

#### C6 — Phase 6: Wi-Fi Backup

```
GOAL: QR-paired LAN backup using HTTP with a scoped, short-lived, single-device
session, as decided in PLAN.md §7. Built on the Phase 4 engine. Residual risk
is documented; we do not overclaim security.

IN SCOPE
- HTTP server bound to a specific LAN interface (default: default-route
  interface; 0.0.0.0 requires explicit user opt-in with a visible warning).
- Session: CSPRNG token ≥128 bits, in-memory only, single-device, scoped.
  Invalidated on expiry, explicit stop, or process exit.
- Token NEVER placed in URLs, NEVER logged. It travels in an HTTP header
  (e.g. X-SyncUp-Session) or as a POST body field, not as a query parameter.
- Per-session staging directory: <data>/wifi_staging/<session_id>/, isolated.
- Filename sanitization: reject `..`, absolute paths, separators, control
  characters, reserved names. Validate against a per-session allowlist built
  from the manifest.
- Manifest endpoint: client posts {rel_path,size,mtime}; server returns
  missing/changed list.
- Upload endpoint: enforces per-request cap and per-session total cap; rate
  limited. Feeds Source::Stream into the Phase 4 engine.
- Host/Origin checks on every request. Not the sole authentication — the
  bearer token is.
- Mobile HTML/JS served from Rust (no build step). Works in Android Chrome.
- QR encodes {url, session_hint}. NEVER encodes the token — the token is
  delivered via the mobile page after a user gesture, or via the session
  handshake described in PLAN.md §7.
- Commands: start_wifi_session, stop_wifi_session, wifi_status. Typed progress.
- Visible stop control in the UI.
- Safe shutdown: on app exit, cancel in-flight uploads, remove staging dir.

OUT OF SCOPE
- No HTTPS in v2.0. Document the threat model in the UI: "Not encrypted. Use
  only on a network you trust."

VERIFICATION GATE
- cargo test  (paste test names; must include:
    expired_session_rejected
    wrong_token_rejected
    missing_token_rejected
    token_never_logged
    token_never_in_url
    path_traversal_rejected
    filename_reserved_rejected
    oversized_request_rejected
    origin_mismatch_rejected
    rate_limit_enforced
    session_isolation_staging
    cleanup_on_shutdown
    manifest_returns_correct_diff
    upload_publishes_via_engine
    retry_resume_after_disconnect
    second_device_rejected_single_session)
- cargo clippy -- -D warnings
- Manual with a real Android phone on your LAN: scan QR, upload ≥3 files,
  confirm landing and DB state, rerun to confirm skip. Report findings.
- Explicitly restate in the report: transport is HTTP, residual risk is LAN
  monitoring; the token is not exposed in URLs or logs.
```

---

#### C7 — Phase 7: Takeout Organizer

```
GOAL: Google Takeout ZIPs and loose folders organised with correct date
precedence, idempotently, using the Phase 4 engine.

IN SCOPE
- Streaming ZIP read (zip crate — do not extract to disk).
- Recursive folder scan.
- JSON sidecar parsing (.json and .supplemental-metadata.json).
- EXIF date read (kamadak-exif or equivalent — verify the crate at install).
- Date precedence (Takeout): Takeout JSON > filename > EXIF > mtime.
  (Phone): filename > EXIF > mtime.
- Naming rules idempotent (running twice must not double-prefix or add a copy
  suffix unless content differs).
- Collision handling via Phase 4 conflict modes.
- Job + progress + cancellation.

OUT OF SCOPE
- No repair for corrupted archives beyond clear error messages.

VERIFICATION GATE
- cargo test  (paste test names; must include:
    takeout_json_precedence
    exif_precedence
    filename_fallback
    mtime_fallback
    zip_nested_folders
    zip_sidecar_parsed
    corrupt_zip_handled
    idempotent_rerun_no_new_files
    collision_kept_both
    collision_overwrite
    collision_skip)
- cargo clippy -- -D warnings
- Manual: run against one real Takeout zip (on a copy) and a loose folder.
  Verify the output tree and idempotent rerun.
```

---

#### C8 — Phase 8: Duplicate Finder

```
GOAL: Safe duplicate detection and reversible move — no permanent deletes.
Crash-safe manifest with per-operation state.

IN SCOPE
- Candidate scan: size, then quick head/tail hash, then full BLAKE2b.
- Duplicate grouping with shallowest-first retention preference per PLAN.md.
- Preview command returns groups + proposed action; UI confirms before any move.
- Move procedure:
  1. Write the COMPLETE operation list to the manifest BEFORE the first move:
     each op has {op_id, source, dest, expected_size, expected_hash, status:"pending"}.
  2. fsync the manifest file and its directory.
  3. For each op, in order:
       a. Move the file (rename, same filesystem; cross-filesystem copy +
          Phase 4 engine).
       b. Update op status to "completed" (or "failed: <reason>").
       c. fsync manifest.
  4. If the process crashes between (a) and (b), status remains "pending";
     startup reconciliation resolves it (see below).
- Startup reconciliation:
  - For each "pending" op: check whether source still exists and dest exists.
    If dest exists and its content hash matches expected: mark completed.
    If source exists and dest does not: mark failed (not moved).
    If both exist: mark "ambiguous" and surface to user — do not silently pick.
    If neither exists: mark "lost" and surface.
- Undo command:
  - Iterate completed ops in reverse order.
  - Before moving back, verify the original path does not exist (or is empty).
    If a file has appeared there since, refuse that op, mark "undo-blocked",
    and continue with others. Never overwrite.
  - Update op status to "undone" and fsync manifest.
  - Idempotent: re-running undo skips ops already at "undone".
- Never delete. Move requires explicit confirmed=true from caller.

OUT OF SCOPE
- No permanent deletion path, even behind a flag, in v2.0.

VERIFICATION GATE
- cargo test  (paste test names; must include:
    same_size_different_content_not_grouped
    exact_dupes_grouped
    quick_hash_collision_caught_by_full_hash
    preview_no_side_effects
    manifest_written_before_first_move
    manifest_fsync_after_each_move
    crash_between_move_and_manifest_reconciled
    partial_move_failure_recoverable
    undo_restores_all
    undo_idempotent
    undo_collision_aware_skips_blocked_op)
- cargo clippy -- -D warnings
- Manual: /tmp copy, preview → move → undo. Verify originals restored.
- Crash-window manual test: kill the process during a move (SIGKILL). Restart.
  Confirm reconciliation classifies correctly.
```

---

#### C9 — Phase 9: UI Completion

```
GOAL: Every screen wired to real commands. No mock data, no fake buttons.

IN SCOPE
- Screens: Dashboard, Phone Backup, Wi-Fi Backup, Organizer, Duplicates,
  Activity, Settings.
- Each screen: loading / empty / success / error / cancelled states; progress;
  cancel button wired to the real command.
- shadcn-svelte components; keyboard navigation; ARIA labels; focus management;
  theme switching; window-size persistence.
- Wi-Fi screen: visible warning that HTTP transport is not encrypted, and a
  visible "Stop session" control.
- If a backend command is not ready, the screen shows "Not implemented yet".
  No placeholder data.
- Remove any disabled/fake controls introduced during scaffolding.

OUT OF SCOPE
- No new backend commands unless the plan says so. If a UI need is unmet by
  backend, STOP and raise it.

VERIFICATION GATE
- npm run check && npm run build && cargo clippy -- -D warnings
- Manual pass through every screen. In the report, list every button and its
  real effect. Produce the status table:
    Feature | Status (verified / implemented-not-tested / not implemented).
```

---

#### C10 — Phase 10: Verification, Packaging, Release Prep

```
GOAL: Packaged Linux build verified end-to-end on a clean profile; CI prepared
for other platforms; documentation updated honestly.

IN SCOPE
- Configure tauri.conf.json bundle.targets explicitly (e.g. ["appimage", "deb"]).
  Do NOT assume defaults produce both.
- npm run tauri build. Run the produced artifacts on a clean user account or VM.
- Verify: startup, icon, data dir creation, migration from a COPY of real
  legacy data (user-initiated, user-approved), MTP listing, Wi-Fi upload,
  organizer, duplicate finder + undo, cancellation, clean shutdown.
- CI: .github/workflows for check (fmt, clippy, test, tsc, build, binding
  regen diff) and Linux build. Stubs (not executed) for Windows/macOS builds
  until tested. No secrets in repo; use Actions secrets for signing.
- Update README.md, CHANGELOG.md, docs/ARCHITECTURE.md, docs/MIGRATION.md,
  docs/DATA_STORAGE.md, docs/PLATFORM_SUPPORT.md, docs/TESTING.md.
- Produce docs/RELEASE_STATUS.md: honest feature classification table plus an
  explicit "not supported in v2.0" list.

OUT OF SCOPE
- Do not claim Windows/macOS support without having built and run on those OSes.
  Do not claim Wi-Fi is "secure" without Phase 6 tests and residual-risk
  documentation in place.

VERIFICATION GATE
- All CI-equivalent commands pass locally.
- Packaged app runs from a clean profile, does a real migration of a COPY of
  real legacy data with user approval, and completes one end-to-end backup and
  one end-to-end duplicate-find+undo.
- docs/RELEASE_STATUS.md has zero "assumed working" entries.
```

---

## Part 5 — Step-by-step procedure

```
SETUP
 1. §2.1 system packages. Verify `gio` separately; record version.
 2. §2.2 Rust and Node. Record exact versions in PROGRESS.md.
 3. §2.3 repo prep: branch check, docs stubs, no moves.
 4. §2.4 fixtures folder.
 5. Confirm the branch is v2-migration and only your prep commits exist.

PHASE 0 — AUDIT (four sessions)
 6. OpenCode A1: paste Standing Rules + PROMPT A1. Commit docs/AUDIT.md.
 7. OpenCode A2: paste Standing Rules + PROMPT A2. Commit.
 8. OpenCode A3: paste Standing Rules + PROMPT A3. Commit.
 9. OpenCode A4: paste Standing Rules + PROMPT A4. Commit.
    ★ Baseline is known. If tests could not run, record why in PROGRESS.md.
    ★ If A4 could not complete because deps are missing: install deps, then
      rerun A4's test section only in a fresh session with a shortened prompt.

PHASE 1 — PLAN (web chat)
10. Paste PROMPT B + full SPEC + full AUDIT into web chat. Save docs/PLAN.md.
11. Critique against §4.2 checklist. Revise if needed. Commit.
    ★ If PLAN.md authorizes relocating Python files: do it yourself with git mv
      in one commit and update AUDIT/PROGRESS path references in a follow-up.

PHASES 2–10 (one OpenCode session per phase)
12. For each phase N in 2..10:
    a. git status must be clean. Commit or stash first.
    b. NEW OpenCode session.
    c. Paste Standing Rules + PROMPT C with correct phase fill-in.
    d. Review pre-flight; respond "go" only if it matches PLAN.md.
    e. For multi-file / DB / file-op changes: review the diff, then approve.
    f. Let it run the gate. Read raw output.
    g. If gate passes:
         git add <explicit files listed by the phase>
         git commit -m "feat: phase N <name>"
         Update PROGRESS.md "Current phase" to N+1.
    h. If gate fails: see §5.1.

§5.1 — Failure handling sequence (authoritative)
   1. Agent reports failure and STOPS.
   2. Human reads the raw failure output.
   3. Human decides: (a) authorize a specific fix, (b) revert to last commit
      and try a smaller slice, or (c) revise PLAN.md in web chat and retry.
   4. If (a): "You are authorized to fix ONLY <specific problem>. Do not
      touch anything else. Re-run the gate."
   5. Agent fixes only that. Re-runs gate.
   6. If gate passes: commit. If it still fails: repeat the sequence, not a
      free-form fix loop.

PHASE 10+ — RELEASE
13. Read docs/RELEASE_STATUS.md.
14. Anything not marked "Implemented + verified" is NOT v2.0. Either fix, or
    move to the "not supported in v2.0" list in README.
15. git tag v2.0.0-rc1 only after that classification is honest.
```

**Between phases: four commands, thirty seconds.**

```bash
git status                          # only expected files
git diff --stat                     # sanity on what's about to be committed
git add <explicit paths>            # never -A
git commit -m "feat: phase N <name>"
```

Then update `PROGRESS.md` handoff, and open a **new** session.

---

## Part 6 — Failure playbook

| Situation | Action |
|---|---|
| Agent asks to modify `main.py` / `core/` etc. | Refuse. Re-paste Standing Rules. Python is read-only reference unless PLAN.md says otherwise. |
| Agent asks to read `~/.config/SyncUp/` | Refuse. Synthetic fixtures only. Production migration is a separate user-initiated app workflow. |
| Agent wants to `git reset --hard`, `git checkout -- .`, or `git clean -fd` | Refuse. See §7. |
| Agent wants to `git commit` | Refuse. You commit, not the agent. Agent writes PROGRESS.md; you review and commit. |
| Gate fails once | Follow §5.1 sequence. No unsupervised fix loops. |
| Gate fails twice | Stop the session. Revise `PLAN.md` in web chat to split the phase; retry smaller. |
| Context meter > ~70% | Stop. Agent updates PROGRESS.md. **You** decide whether to commit WIP to a `wip/phase-N` branch. New session. |
| Agent hallucinates a crate or API | "Quote the docs.rs URL." If it cannot, drop the dependency and re-plan. |
| Agent claims done without output | "Paste the raw output of the gate, or write 'not run'." |
| `cargo build` fails on Zorin | Verify `libwebkit2gtk-4.1-dev` and `libsoup-3.0-dev` are installed. |
| Rust build thrashes RAM on old PC | `CARGO_BUILD_JOBS=2`; close browsers during builds. First build is slow; do not kill it. |
| Phase 4 atomic replace tests fail on Windows in CI | Expected — they are `cfg(windows)`. They must compile on Linux, run on Windows only. State this in the report. |
| Wi-Fi phase: token appears in a log line during testing | Treat as a bug. Fix, retest. Token logging is a test failure. |
| Duplicate phase: crash mid-move in manual test | Expected behaviour is reconciliation on restart. Verify it. If it silently loses the file, the phase fails. |

---

## Part 7 — Safe git recovery

When a session goes wrong:

```bash
# 1. See what changed
git status
git diff --stat
git diff

# 2. Keep the parts you want to preserve
git add path/to/keep.rs
git commit -m "wip: preserve working pieces from phase N attempt"

# 3. Set the rest aside (recoverable)
git stash push -u -m "phase N attempt 1 - discarded"
# -u includes untracked files in the stash (recoverable via `git stash pop`)

# 4. If you've confirmed you want to discard SPECIFIC tracked file changes only
#    (after inspecting the diff), restore path by path:
git restore --worktree -- src-tauri/src/services/fileops.rs
git restore --worktree -- src/lib/components/Foo.svelte
# NEVER `git checkout -- .` or `git restore .` — those hit everything.

# 5. If you think you want to delete untracked files, DRY-RUN FIRST:
git clean -nd          # list what would be deleted
# Then delete specific paths only, or nothing at all.

# 6. NEVER:
#      git reset --hard
#      git checkout -- .
#      git restore .
#      git clean -fd (without reviewing -nd first)
```

`git stash` keeps the discarded work recoverable via `git stash list` / `git stash show -p stash@{0}`. A `reset --hard` does not.

---

## Part 8 — The one conceptual point to keep

SyncUp v2 is a **source-guided architectural migration**, not "a new UI over the old app". The Python implementation is the behavioural oracle for every non-trivial behaviour: date precedence, conflict handling, already-backed-up detection, filename naming, duplicate grouping. Where the audit says the old code is correct, the new code reproduces that behaviour. Where the audit says the old code is unsafe (no Wi-Fi auth, plain file copy for DB backup, non-crash-safe duplicate manifest), the plan documents the intentional change and its justification in `PLAN.md §3` (migration matrix). Nothing changes silently.

---

## What to do right now

1. **Do not move or rename anything in the repo.** Run §2.1–§2.4 exactly.
2. Confirm `docs/SPEC.md`, `docs/AUDIT.md`, `docs/PLAN.md`, `docs/PROGRESS.md` exist and are stubs. Confirm `PROGRESS.md` contains the pinned toolchain placeholder to fill in during setup.
3. Open your first OpenCode session and paste **Standing Rules + PROMPT A1**. Do not rush it. If A1 takes two attempts, that's fine.
4. **Do not start Phase 2 until `PLAN.md` is written, critiqued against the §4.2 checklist, and committed.**

The audit is the foundation. If `AUDIT.md` cannot state clearly how the existing Python code handles backup-state comparison, date selection, duplicate detection and file conflicts, the plan is not ready and the rewrite is not ready. Take the time on Phase 0.

**Freeze this document here. Ship the audit.**
