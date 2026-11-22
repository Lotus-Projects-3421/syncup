#!/usr/bin/env bash
set -euo pipefail

MAX_BYTES="${AI_MAX_FILE_BYTES:-250000}"

if [ $# -eq 0 ]; then
  echo "Usage: $0 <file-or-directory> [more files or directories...]" >&2
  exit 1
fi

echo "# AI Context Snapshot"
echo ""
echo "- Date: $(date -Is)"
echo "- Host: $(hostname)"
echo "- Working directory: $(pwd)"
echo "- User: $(whoami)"
echo ""

if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "## Git state"
  echo ""
  echo "Branch: $(git branch --show-current || true)"
  echo ""
  echo "### git status --short"
  echo '```bash'
  git status --short || true
  echo '```'
  echo ""
  echo "### git log -n 5 --oneline"
  echo '```bash'
  git log -n 5 --oneline || true
  echo '```'
  echo ""
fi

print_file() {
  local f="$1"
  local size
  size="$(stat -c%s "$f")"

  echo ""
  echo "### File: $f"
  echo ""
  echo '```'"${f##*.}"
  if [ "$size" -gt "$MAX_BYTES" ]; then
    head -c "$MAX_BYTES" "$f"
    echo ""
    echo "...TRUNCATED: file is $size bytes, showed first $MAX_BYTES bytes."
  else
    cat "$f"
  fi
  echo '```'
  echo ""
}

print_directory() {
  local d="$1"
  echo ""
  echo "### Directory listing: $d"
  echo ""
  echo '```text'
  find "$d" -type f | sort | head -n 300
  echo '```'
  echo ""
}

echo "## Requested context"
echo ""

for item in "$@"; do
  if [ -d "$item" ]; then
    print_directory "$item"
  elif [ -f "$item" ]; then
    print_file "$item"
  else
    echo ""
    echo "### Missing path: $item"
    echo ""
  fi
done
