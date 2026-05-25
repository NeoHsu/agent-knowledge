#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$HOME/.claude/projects}"
TODAY="$(date -u +%Y-%m-%d)"

if [[ ! -d "$ROOT" ]]; then
  exit 0
fi

find "$ROOT" -type f \( -name '*.jsonl' -o -name '*.json' \) -newermt "$TODAY" -print0 |
  sort -z |
  while IFS= read -r -d '' file; do
    printf '\n## %s\n\n' "$file"
    sed -n '1,240p' "$file"
  done
