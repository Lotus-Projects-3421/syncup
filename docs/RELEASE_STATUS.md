# SyncUp V2 — Release Status

**Version:** 2.0.0
**Release date:** 2026-11-27
**Platform:** Linux x86_64
**Branch:** `v2-migration`

This document records the release evidence available during release preparation. It distinguishes automated checks, physical workflow tests, and packaging verification.

## Feature validation

| Feature | Validation evidence | Status |
|---|---|---|
| Application shell and navigation | Application launched successfully | Passed |
| File safety engine | Automated file-operation tests | Passed |
| Android MTP backup | Physical Android device; 610 files copied, zero reported as not copied, with BLAKE2b verification | Passed |
| Wi-Fi backup | User-tested QR pairing and photo/video upload from phone to PC | Passed |
| Google Takeout Organizer | User-tested approximately 1 GB Takeout archive from 2018; photos organized by month | Passed |
| Duplicate Finder | Three identical files tested; two moved to quarantine and hash-verified, then restored | Passed |
| Debian package | Package installed and SyncUp V2 launched from Applications menu | Passed |
| AppImage | Runtime check and application launch succeeded | Passed |

## Automated verification

The recorded Phase 8 verification included:

- `cargo fmt --check`
- `cargo check`
- `cargo clippy`
- `cargo test`
- `npm run specta:check`

The recorded test result was **110 passed and 4 ignored**. The ignored tests were hardware-specific tests.

Frontend checks and production build were also used during packaging.

These results describe recorded local verification. They do not establish that GitHub Actions has passed for the final release commit.

## Release artifacts

The final distribution files are:

- `SyncUp V2_2.0.0_amd64.deb`
- `SyncUp V2_2.0.0_amd64.AppImage`

The older `SyncUp_2.0.0_amd64.deb` was generated before the product-name change and is superseded. It must not be distributed as the final V2 package.

## Known limitations

1. **Linux only:** Windows and macOS packages are not provided.
2. **Wi-Fi encryption:** The local Wi-Fi upload server uses HTTP without TLS. Use only on trusted networks.
3. **Upload resumption:** Interrupted Wi-Fi uploads may need to be restarted.
4. **Metadata support:** Date extraction depends on supported metadata and file formats.
5. **Cross-filesystem moves:** Cross-filesystem operations cannot guarantee the same atomic rename behavior as same-filesystem operations.
6. **Clean-machine coverage:** Installation and launch were verified on the development machine, not on a separate clean Linux installation.

## CI

The repository contains a GitHub Actions workflow for frontend checks, Rust formatting, compilation, linting, and tests. A Specta binding consistency check is included in the release-preparation change.

The workflow must run successfully on GitHub before CI can be marked as release-verified.

## Release checklist

- [x] Core feature implementation completed.
- [x] Physical Android MTP backup tested.
- [x] Physical Wi-Fi upload tested.
- [x] Real Takeout archive tested.
- [x] Duplicate scan, quarantine move, and Undo tested.
- [x] Debian package built, installed, and launched.
- [x] AppImage built and launched.
- [x] Product name changed to SyncUp V2.
- [x] Legacy SyncUp 1.4.1 desktop entry preserved.
- [x] Wi-Fi security limitation documented.
- [ ] Documentation changes committed.
- [ ] CI workflow verified on GitHub.
- [ ] Release tag created and pushed.
- [ ] GitHub release published with final packages and checksums.
