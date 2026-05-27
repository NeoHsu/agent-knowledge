#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MEM_BIN="${MEM_BIN:-$ROOT/target/release/mem}"

if [[ ! -x "$MEM_BIN" ]]; then
  echo "release binary not found or not executable: $MEM_BIN" >&2
  echo "run scripts/build-release.sh first" >&2
  exit 1
fi

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/agent-knowledge-smoke.XXXXXX")"
cleanup() {
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

mkdir -p "$WORKDIR/schema"
cp "$ROOT/schema/memory-schema.sql" "$WORKDIR/schema/memory-schema.sql"

(
  cd "$WORKDIR"
  "$MEM_BIN" init >/dev/null
  "$MEM_BIN" save \
    --name smoke_release \
    --source manual \
    --tags '["smoke:test"]' \
    --content "release smoke searchable content" \
    --force >/dev/null
  "$MEM_BIN" query "searchable" --no-touch | grep -q "smoke_release"
  "$MEM_BIN" reindex >/dev/null
  "$MEM_BIN" query "release smoke" --no-touch | grep -q "smoke_release"
  "$MEM_BIN" export --format json | grep -q "smoke_release"
)

echo "release smoke ok"
