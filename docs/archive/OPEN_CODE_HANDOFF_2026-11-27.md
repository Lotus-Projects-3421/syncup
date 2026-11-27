# SYNCUP V2 — OPEN CODE AUTONOMOUS COMPLETION HANDOFF

## 1. Project Identity and Current State

**Project:** SyncUp V2 — Safe Android photo/video backup and Google Takeout organizer for Linux.

**Status:** Substantially implemented. Application launches and runs on Linux desktop. UI accepted and frozen. Remaining work: physical functional testing, Linux packaging, comprehensive documentation, and release preparation.

**Version:** 2.0.0

**Branch:** `v2-migration`

**Latest Commit:** `68420ad` — "fix: phase 8 dedup - treat PublishPendingDurability as Completed since move succeeded"

**Working Tree:** Clean (as of last verification)

**Repository:** `/home/uday/Music/SyncUp`

---

## 2. Repository and Git History

The repository contains:
- **V2 application** (Rust/Tauri/Svelte) in `src/` and `src-tauri/`
- **Legacy V1 Python/Flet files** (preserved for rollback, do not delete without justification)
- Documentation in `docs/`
- AI harness at `tools/ai-harness/`
- Automatic backups at `.ai-backups/`

**Recent Commit History:**
```
68420ad fix: phase 8 dedup - treat PublishPendingDurability as Completed since move succeeded
b481850 test: phase 8 dedup - strengthen 9 tests with real conditions, deterministic fixtures and honest scope documentation
41fea1e test: phase 8 dedup - implement 21 test bodies with deterministic manifest lookup
43e0e13 fix: phase 8 dedup - create quarantine destination parent before move_verified
9476727 ci: replace python v1 workflow with v2 rust/node checks
b23aaf1 docs: reconcile documentation for v2 rust/tauri rewrite and flag release blockers
b3a69e5 feat: phase 9 slice g - dashboard with health, recent activity and quick actions
```

**Git Policy:**
- Verify `git status` before substantial changes
- Commit coherent milestones
- Never use destructive reset/clean without inspecting affected changes
- Preserve user data and test artifacts outside the repo

---

## 3. Complete Architecture and Technology Context

### Technology Stack
- **Frontend:** Svelte 5 (runes), TypeScript strict, Vite 8, Tailwind CSS 4, shadcn-svelte components
- **Backend:** Rust 2021, Tauri 2, Tokio, Axum (Wi-Fi server), SQLite (rusqlite + r2d2), BLAKE2 hashing
- **IPC:** Tauri Specta-generated TypeScript bindings (`src/lib/ipc.ts` — **never hand-edit**)
- **Platform:** Linux MTP via Gio/GLib, GVFS mount integration
- **Testing:** Cargo test, Svelte check, Clippy with `-D warnings`

### Application Structure
```
src/                          # Svelte frontend
├── routes/                   # 7 application screens
├── lib/
│   ├── components/          # UI components (Card, Button, Badge, etc.)
│   ├── services/            # IPC wrappers (backup, dedup, jobs, wifi, etc.)
│   ├── state/               # Navigation, theme
│   └── ipc.ts               # Generated — DO NOT EDIT
└── App.svelte               # Application shell

src-tauri/                    # Rust backend
├── src/
│   ├── commands/            # Tauri command handlers
│   ├── services/            # Business logic (fileops, backup, wifi, dedup, organizer)
│   ├── domain/              # Pure logic (naming, conflict resolution)
│   ├── storage/             # SQLite schema, migrations, profiles, settings
│   ├── platform/linux/      # MTP discovery via GVFS/GIO
│   └── lib.rs               # Specta builder, app setup, reconciliation
├── tests/                   # Integration tests (147 passing)
└── tauri.conf.json          # Tauri configuration
```

### Critical Architectural Constraints
1. **Never hand-edit `src/lib/ipc.ts`** — regenerate via `npm run gen:bindings` only when backend IPC genuinely changes
2. **Preserve legacy V1 files** unless cleanup is explicitly justified
3. **Avoid unnecessary dependencies**
4. **Preserve file-safety guarantees** (atomic publish, verify before commit, quarantine not delete)
5. **Inspect actual configuration** before choosing commands (e.g., `npm run tauri dev` not `cargo run`)

---

## 4. Completed Phases and All UI Slices

### Phases 0–4: Foundation
- Storage and SQLite with WAL mode
- Database migrations (v0 → v2)
- File operations engine with atomic publish, size/hash verification, destination collision protection
- Shared job registry with progress reporting and cancellation
- Error handling and structured logging

### Phase 5: Linux MTP Phone Backup
- Android device detection via GVFS/GIO
- Device browsing and folder selection
- Backup workflow with job tracking
- File safety and verification
- **Status:** Implemented, real-device validation pending

### Phase 6: Wi-Fi Backup
- Local HTTP server with QR pairing
- 256-bit session token (constant-time comparison, never in URL/logs)
- Upload staging with verification
- Manifest-based publication on Stop
- Cancel vs Stop semantics (cancel discards, stop publishes)
- Session-specific staging directory
- **Status:** Implemented, real Android browser pairing pending
- **Known limitations:** No TLS/HTTPS, no chunked resumable uploads

### Phase 7: Google Takeout Organizer
- ZIP streaming (member-by-member, never full extraction)
- Date precedence: Takeout JSON → filename → EXIF → mtime
- Folder layouts: year, month, day, flat
- Collision handling: skip, overwrite, keep-both
- Idempotent reprocessing
- **Status:** Implemented, small-archive validation pending

### Phase 8: Duplicate Finder
- Scan pipeline: size grouping → quick hash (BLAKE2b-128, head+tail 64KiB) → full hash (BLAKE2b-512)
- Retention ordering: shallowest path → oldest mtime → alphabetical
- Quarantine moves with manifest tracking
- Undo with collision awareness
- Startup reconciliation for Pending/UndoPending states
- **Status:** Implemented and tested (21 integration tests passing)

### Phase 9: UI Slices A–G
- **Slice A:** Foundation, Activity screen, job history
- **Slice B:** Settings, legacy migration UI
- **Slice C:** Organizer UI
- **Slice D:** Phone Backup UI
- **Slice E:** Wi-Fi Backup UI (QR, progress, stop/cancel)
- **Slice F:** Duplicate Finder UI (scan, move, undo, manifest history)
- **Slice G:** Dashboard (health, recent activity, quick actions)
- **Status:** All implemented, UI accepted and frozen

---

## 5. Important Implementation Decisions

### File Safety Invariants (Non-Negotiable)
1. Never destroy/replace an existing destination before the replacement is completely written and verified
2. Use destination-local staging/temp files
3. Verify file content/size before publishing to final destination
4. Preserve existing files on failure
5. Maintain recoverable state for operations that can move/modify files
6. Never silently discard staging data that may contain the only remaining copy
7. Duplicate cleanup is quarantine, not deletion
8. Undo must preserve files when original destination is occupied
9. Ambiguous outcomes must be reported as ambiguous, not success/failure
10. Cancellation is not rollback — work completed before cancellation may remain

### Duplicate Finder Decisions
- No per-file selection (backend processes entire duplicate set)
- Manifest written before first move
- Move cancellation does not roll back already-completed moves
- Manifest retrieval failures reported as uncertain outcomes
- Reattachment to existing dedup job is ambiguous (scan/move/undo share same JobKind)

### Wi-Fi Backup Decisions
- One job per session
- Session-specific staging
- Token never in QR URL or logs
- Server binds to selected interface, no loopback/all-interfaces fallback
- Stop publishes verified staged files; Cancel discards
- Drain timeout preserves staging

### UI Decisions
- No fake progress percentages (backend doesn't expose scan progress)
- No individual duplicate file checkboxes (backend doesn't support it)
- Honest empty states and error messages
- No invented storage statistics

---

## 6. Important Historical Defects and Corrections

### Defect 1: Quarantine Parent Directory Not Created
**Problem:** `run_move` computed quarantine destination path but never created its parent directory. `renameat2` into missing parent returned ENOENT, causing all duplicate moves to fail.

**Symptom:** Every duplicate move recorded as `Failed`, source file never moved, manifest showed failure.

**Fix (commit `43e0e13`):** Added `fs::create_dir_all(parent)` before `move_verified` call in `run_move`.

**Impact:** Duplicate Finder became functional.

### Defect 2: Manifest Lookup After Quarantine Directory Creation
**Problem:** After fix #1, `dedup/` directory contained both `<manifest_id>.json` and `<manifest_id>/` (quarantine tree). Tests using `read_dir().next()` sometimes selected the directory instead of the manifest, causing EISDIR errors.

**Fix (commit `b481850`):** Introduced `manifest_json_path()` helper that filters for `.json` files and asserts exactly one manifest exists.

**Impact:** Tests became deterministic and reliable.

### Defect 3: PublishPendingDurability Recovery
**Problem:** When `move_verified` succeeded (file moved to quarantine) but subsequent `sync_dir()` failed, the operation was recorded as `Failed`. Startup reconciliation only examines `Pending`/`UndoPending` states, not `Failed`. Undo filters for `Completed` only. Result: file stuck in quarantine with no recovery path.

**Fix (commit `68420ad`):** Added specific match arm in `run_move` for `PublishPendingDurability` that treats it as `Completed` since the file movement succeeded.

**Qualification:** Directory fsync failure still means persistence across sudden power loss is not guaranteed, but the file is at its destination and undo can restore it.

**Impact:** Duplicate Finder recovery path restored.

---

## 7. Current Known Limitations

### Platform Limitations
- **Linux only** — Windows and macOS not verified or supported in v2.0
- **iOS unsupported** — no MTP implementation
- **MTP requires GVFS** — phone must be unlocked and in File Transfer mode

### Wi-Fi Backup Limitations
- **No TLS/HTTPS** — plain HTTP on local network only
- **No chunked resumable uploads** — interrupted uploads must restart
- **One session at a time** — no concurrent sessions
- **30-minute session expiry**
- **Limits:** 8 GiB per file, 50 GiB per session

### Duplicate Finder Limitations
- **No per-file selection** — entire duplicate set processed
- **Cross-filesystem moves** use copy+delete, not strictly atomic
- **Scan progress not exposed** — UI shows indeterminate progress only
- **Reattachment ambiguity** — cannot distinguish scan/move/undo for existing jobs

### Organizer Limitations
- **EXIF parsing** limited to JPEG/TIFF; HEIC/WebP/RAW fall back to filename/mtime
- **500,000 files per run** limit
- **50 GiB uncompressed per archive** limit
- **Corrupt archive stops entire run** before any file is copied

### Testing Limitations
- **Real-device MTP validation** not performed
- **Real Android Wi-Fi pairing** not performed
- **Large photo library** duplicate scanning not tested
- **Packaging and clean installation** not verified

---

## 8. Actual Verification Evidence

### Automated Tests
- **Full Rust suite:** 147 passed, 0 failed, 4 hardware-ignored
- **Duplicate Finder integration tests:** 21 passed (previously empty, now meaningful)
- **Clippy:** Passed with `-D warnings`
- **Frontend type check:** `npm run check` passed
- **Frontend build:** `npm run build` passed

### Desktop Launch Verification
**Command:** `npm run tauri dev`

**Observed:**
- Vite dev server started at `http://localhost:1420/`
- Rust backend compiled successfully
- Desktop window titled "SyncUp" opened
- Logs showed: `SyncUp starting version="2.0.0"`
- Database migrated: `previous_version=0 new_version=2`
- Database connection pool ready

**Screenshots confirmed:**
- Dashboard renders with health cards and quick actions
- Phone Backup shows honest "No mounted device found" with GVFS instructions
- Wi-Fi Backup shows red "Not encrypted" warning and session setup
- Organizer shows source/destination inputs and layout controls
- Duplicate Finder shows scan configuration with exclude patterns
- Activity shows empty state with refresh button
- Settings shows appearance controls and legacy migration UI

**UI Status:** Accepted and frozen. No redesign or cosmetic refinement phase.

---

## 9. Successful Desktop Launch Evidence

The application has been launched and visually inspected. All seven routes render correctly:

1. **Dashboard** — Application health (v2.0.0), database version (user_version 2), qualified safety statement, recent activity panel, quick-action buttons
2. **Phone Backup** — Device detection area with refresh button, honest empty state with GVFS mounting instructions
3. **Wi-Fi Backup** — Red security warning, destination folder input, session limits documentation, Start session button
4. **Organizer** — Source folders/ZIP input, destination folder, source type/layout/conflict dropdowns, "This run will" preview
5. **Duplicates** — Folders to scan, exclude patterns, Start Scan button
6. **Activity** — Empty state with activity icon and "No activity yet" message
7. **Settings** — Theme toggle, default window size inputs (900–7680 width, 600–4320 height), legacy migration inputs

**Navigation:** Sidebar navigation works, all routes accessible.

**Theme:** Dark mode default, light/system options available.

**Status:** Application is functional and ready for physical workflow testing.

---

## 10. Remaining Physical Testing Tasks

### Priority 1: Duplicate Finder (No Hardware Required)
**Test fixture:**
```bash
mkdir -p /tmp/syncup-test/a /tmp/syncup-test/b/deep
printf 'same content here' > /tmp/syncup-test/a/img1.jpg
cp /tmp/syncup-test/a/img1.jpg /tmp/syncup-test/b/img1-copy.jpg
cp /tmp/syncup-test/a/img1.jpg /tmp/syncup-test/b/deep/img1-copy2.jpg
printf 'unique one' > /tmp/syncup-test/a/unique1.jpg
printf 'unique two' > /tmp/syncup-test/a/unique2.jpg
```

**Validation steps:**
1. Scan `/tmp/syncup-test`
2. Verify one group of 3 (keep `a/img1.jpg`, two duplicates), two uniques ignored
3. Move duplicates → check Activity shows job complete
4. Undo → verify all three files restored to original paths
5. Run `find /tmp/syncup-test -type f` to confirm file locations

### Priority 2: Takeout Organizer (No Hardware Required)
**Test fixture:** Create a small ZIP with photo files and JSON sidecars, or use a real small Takeout archive.

**Validation steps:**
1. Select source ZIP and destination folder
2. Choose layout (year/month/day)
3. Run organizer
4. Verify folder structure and file naming
5. Confirm originals remain safe

### Priority 3: Phone Backup (Requires Android Phone)
**Prerequisites:**
- Android phone with USB cable
- Phone unlocked and in File Transfer / MTP mode
- Linux GVFS mount visible (check `/run/user/<uid>/gvfs/`)

**Validation steps:**
1. Click "Refresh devices" on Phone Backup screen
2. Verify device appears
3. Browse device folders
4. Select small set of sample photos
5. Choose disposable destination
6. Run backup
7. Verify copied files at destination
8. Confirm existing destination files not overwritten unsafely

### Priority 4: Wi-Fi Backup (Requires Android Phone)
**Prerequisites:**
- Android phone on same local network
- Phone browser capable of scanning QR or entering URL

**Validation steps:**
1. Click "Start session" on Wi-Fi Backup screen
2. Verify QR code appears
3. Scan QR with phone (or enter URL manually)
4. Verify phone browser opens upload interface
5. Select and upload a few photos
6. Click "Stop session" on desktop
7. Verify staged files published to destination
8. Confirm job shows complete in Activity

### Testing Philosophy
- Use small, disposable datasets first
- Never risk user's only copy of important photographs
- If hardware unavailable, document limitation and continue with independent work
- Do not repeatedly test successful workflows
- Fix actual defects and move forward

---

## 11. Packaging and Installation Requirements

### Current Configuration
- **Tauri config:** `src-tauri/tauri.conf.json` specifies bundle targets as "all"
- **Icons:** Present in `src-tauri/icons/` (32x32, 128x128, 128x128@2x, .icns, .ico)
- **Identifier:** `io.github.lotus-projects-3421.syncup`

### Required Steps
1. **Build release frontend:** `npm run build`
2. **Build Tauri release binary:** `npm run tauri build`
3. **Identify package format:** Check `src-tauri/target/release/bundle/` for `.deb`, `.AppImage`, or other Linux packages
4. **Install package:** Use appropriate package manager (dpkg for .deb, chmod + run for .AppImage)
5. **Verify installation:**
   - Application launches from desktop menu or command line
   - Window opens correctly
   - Database/application data directories created in `~/.local/share/` or `~/.config/`
   - Essential navigation functions
   - Installed app not dependent on Vite dev server

### Dependencies
System dependencies required for Tauri on Linux:
- `libwebkit2gtk-4.1-dev`
- `build-essential`
- `curl`
- `wget`
- `file`
- `libxdo-dev`
- `libssl-dev`
- `libayatana-appindicator3-dev`
- `librsvg2-dev`
- `libglib2.0-bin` (for `gio` MTP support)
- `pkg-config`
- `git`

---

## 12. Comprehensive Documentation Objectives

The project needs documentation sufficient for another developer or autonomous agent to understand, operate, debug, maintain, and extend it without this conversation.

### Required Documents

**1. README.md** — Public-facing overview
- What SyncUp is, core capabilities, supported platform
- Prerequisites, installation, running instructions
- Basic workflow instructions, known limitations, development commands

**2. Architecture Documentation** — Technical structure
- Frontend/backend separation, Tauri communication
- Svelte route/component structure
- Rust command/service/domain structure
- SQLite storage, job system, error handling, logging
- File-operation pipeline, MTP architecture, Wi-Fi server architecture
- Organizer architecture, Duplicate Finder architecture
- Startup/shutdown lifecycle, data directories, configuration
- Include Mermaid diagrams where useful

**3. UI Documentation** — Screen reference
- For each of 7 screens: purpose, components, user actions, controls, expected behavior, backend interactions, loading/empty/error states, known limitations
- Current design conventions: theme, navigation, layout, component patterns, status indicators, progress presentation, dialog/confirmation patterns, error messages

**4. Backend and API Reference** — Technical reference
- Tauri commands, service responsibilities, important Rust modules
- IPC data structures, job lifecycle, manifest lifecycle
- Error types, file operations, hash verification, recovery mechanisms
- Database schema and migrations

**5. Data Safety and Recovery Documentation** — Critical for SyncUp
- Non-negotiable data-safety principles (10 invariants)
- Recovery paths for backup, Wi-Fi, organizer, Duplicate Finder
- Crash/interruption scenarios and actual recovery behavior

**6. Development and Maintenance Guide** — Operational reference
- Required toolchain, dependencies, build commands
- Development commands, formatting, Clippy, frontend checks
- Test commands, packaging commands, generated files
- Git workflow, debugging approach
- Copy-paste usable instructions

**7. Standing Engineering Rules** — Future constraints
- Preserve data-safety guarantees, avoid silent data loss
- Do not fake unsupported backend functionality in UI
- Do not invent APIs or fields, do not hand-edit generated IPC
- Preserve legacy V1 files unless cleanup explicitly justified
- Use existing architecture and shared services
- Avoid unnecessary dependencies
- Keep error reporting truthful
- Treat cancellation and recovery carefully
- Verify Git status and commit identity
- Do not run expensive checks unnecessarily
- Prefer focused tests after narrow changes

**8. Known Limitations and Future Roadmap** — Honest status
- Separated into: implemented and verified, implemented but not physically verified, known limitations, unsupported functionality, potential future enhancements
- Do not convert optional future ideas into release blockers

**9. Release and Testing Record** — Final status
- Final version, Git commit, build environment, package format
- Installation procedure, verification performed, hardware tested
- Known limitations, release decision
- Record actual results, not assumed success

---

## 13. Standing Safety Rules

These rules are central to SyncUp and take priority over convenience:

1. **Never destroy or replace an existing destination file before the replacement has been completely written and verified.**
2. **Use destination-local staging/temp files where required.**
3. **Verify file content/size before publishing it to the final destination.**
4. **Preserve existing files on failure.**
5. **Maintain recoverable state for operations that can move or modify files.**
6. **Never silently discard staging data that may contain the only remaining copy.**
7. **Duplicate cleanup is a quarantine operation, not permanent deletion.**
8. **Undo must preserve files when the original destination is occupied.**
9. **An ambiguous outcome must be reported as ambiguous, not presented as success or failure without evidence.**
10. **Cancellation is not rollback. Work completed before cancellation may remain completed.**

For every proposed code change, consider:
- What happens if the application closes halfway through?
- What happens if the disk becomes full?
- What happens if a destination already exists?
- What happens if cancellation arrives during a copy or move?
- What happens if the UI loses contact with the job?
- Can the operation be recovered safely?

---

## 14. Git and Rollback Guidance

### Before Major Work
- Confirm repository path: `/home/uday/Music/SyncUp`
- Confirm branch: `v2-migration`
- Confirm HEAD: `git log -1 --oneline`
- Inspect working-tree changes: `git status --short`
- Preserve existing user changes

### After Coherent Task
- Inspect diff: `git diff --stat`
- Run appropriate checks (targeted tests, Clippy, frontend check as appropriate)
- Commit if change is complete

### Rollback Capability
- Git history provides rollback
- `.ai-backups/` contains automatic source backups
- Never use `git reset --hard` or `git clean -fd` without inspecting affected changes
- Never assume previous commit is still HEAD without checking

### Commit Messages
- Use conventional commits: `feat:`, `fix:`, `docs:`, `test:`, `ci:`
- Include scope where helpful: `fix: phase 8 dedup - ...`
- Keep messages concise but descriptive

---

## 15. Time-Saving Rules

The user has experienced long development payloads and unnecessary testing cycles. Improve this:

### Risk-Based Testing Strategy
- **Documentation-only change:** Do not run Rust tests
- **Small frontend-only correction:** Use `npm run check` only
- **Narrow Rust service correction:** Run relevant targeted tests and Clippy
- **Major integration changes or final release:** Run full relevant verification suite
- **Lengthy stress tests:** Only if concrete risk or genuinely necessary before release

### Execution Time Estimates
- `npm run check`: ~1 minute
- `npm run build`: ~10 seconds
- `cargo check`: ~30 seconds
- `cargo clippy -- -D warnings`: ~1 minute
- `cargo test --test dedup`: ~2 seconds
- Full `cargo test`: ~30 seconds
- `npm run tauri dev` (first run): ~1 minute compile + launch
- `npm run tauri build`: ~5–10 minutes

### Parallel Independent Work
- When safe, run independent checks in parallel
- Do not run duplicate checks that provide no additional evidence
- If a command is likely to take a long time, explain why it is needed

### Avoid Unnecessary Work
- Do not run every possible test after every small change
- Do not repeat previously passed verification gates without relevant code change
- Do not investigate hypothetical problems without evidence
- Do not introduce optional architectural improvements
- Do not expand the roadmap

---

## 16. Clear Completion Criteria

The agent should aim for a release candidate when:

1. ✅ The application launches normally
2. ✅ All main screens are usable
3. ⏳ Essential workflows have been physically tested where hardware is available
4. ✅ Genuine critical data-safety defects are resolved
5. ⏳ Linux release packaging succeeds
6. ⏳ The installed package launches
7. ⏳ Essential documentation is complete and accurate
8. ⏳ Known limitations are recorded
9. ⏳ The final Git state is understandable and recoverable

**Do not demand:**
- Perfect UI polish
- Every possible edge-case test
- Cross-platform support in v2.0
- Additional features
- Architectural perfection
- Zero minor bugs

**When acceptance criteria are met, stop. Do not manufacture another development phase.**

---

## 17. Recommended First Task for OpenCode

**Task:** Physical functional testing of Duplicate Finder with disposable data.

**Rationale:**
- No hardware required
- Exercises the exact move → manifest → undo → recovery path we just hardened
- Validates the PublishPendingDurability fix in practice
- Builds confidence before moving to hardware-dependent tests
- Quick to execute (~5 minutes including fixture creation)

**Steps:**
1. Create test fixture at `/tmp/syncup-test/` (commands in Section 10)
2. Launch app: `npm run tauri dev`
3. Navigate to Duplicates screen
4. Enter `/tmp/syncup-test` in "Folders to scan"
5. Click "Start Scan"
6. Verify scan results show one group of 3 duplicates, two uniques ignored
7. Click "Move duplicates" (or equivalent confirmation button)
8. Check Activity screen shows job complete
9. Click "Undo" on the completed job
10. Run `find /tmp/syncup-test -type f` to verify all files restored
11. Report results

**If successful:** Proceed to Organizer testing, then packaging.

**If hardware available:** Also test Phone Backup and Wi-Fi Backup with real Android phone.

**If hardware unavailable:** Document limitation, proceed to packaging and documentation.

---

# FILES TO PROVIDE TO OPEN CODE

Provide these files as initial context. Prioritize in this order:

## Essential (Must Provide)
1. **This handoff document** — Complete project context
2. **`docs/PLAN.md`** — Original product constraints and testing/platform expectations
3. **`docs/PROGRESS.md`** — Current progress state (may be stale, OpenCode should verify)
4. **`docs/RELEASE_STATUS.md`** — Release readiness tracker (if exists)

## Important (Should Provide)
5. **`README.md`** — Current public-facing documentation (likely stale, needs update)
6. **`docs/ARCHITECTURE.md`** — Current architecture documentation (likely describes V1, needs update)
7. **`src-tauri/Cargo.toml`** — Rust dependencies and build configuration
8. **`package.json`** — Frontend dependencies and scripts
9. **`src-tauri/tauri.conf.json`** — Tauri configuration

## Reference (Provide If Available)
10. **`SyncUp v2 — Operational Playbook, Final Edition.txt`** — Development workflow and standing rules
11. **`tools/ai-harness/`** — AI development harness (if OpenCode needs to understand previous workflow)
12. **`.github/workflows/ci.yml`** — CI configuration

## Do NOT Provide (OpenCode Can Inspect Directly)
- Source code files (OpenCode has repository access)
- Generated files like `src/lib/ipc.ts`
- Build artifacts in `target/` or `dist-web/`
- Legacy V1 Python files (unless specifically needed for migration testing)

---

# FINAL INSTRUCTION

This is the final transition from ChatGPT-guided development to autonomous completion.

We are not restarting SyncUp.

We are not planning another large feature phase.

We are finishing the existing product, testing it on the actual machine, documenting it properly and preparing a usable Linux release.

**OpenCode has enough context to take over confidently, work independently and bring SyncUp V2 to completion without repeating previous development loops.**

The application works. The UI is accepted. The critical recovery defect is fixed. Now: test it, package it, document it, release it.
