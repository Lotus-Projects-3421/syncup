#!/usr/bin/env bash
set -u

run() {
  echo ""
  echo "$ $*"
  "$@"
  local rc=$?
  echo "EXIT=$rc"
}

run cargo fmt --check
run cargo check
run cargo clippy --all-targets -- -D warnings
run cargo test --no-fail-fast
run npm run specta:check
