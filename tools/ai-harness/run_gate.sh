#!/usr/bin/env bash
set -u

MANIFEST=src-tauri/Cargo.toml

run() {
  echo ""
  echo "$ $*"
  "$@"
  local rc=$?
  echo "EXIT=$rc"
}

run cargo fmt --manifest-path "$MANIFEST" --check
run cargo check --manifest-path "$MANIFEST"
run cargo clippy --manifest-path "$MANIFEST" --all-targets -- -D warnings
run cargo test --manifest-path "$MANIFEST" --no-fail-fast
run npm run specta:check
