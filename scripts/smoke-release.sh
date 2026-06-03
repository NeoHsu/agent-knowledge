#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MEM_BIN="${MEM_BIN:-$ROOT/target/release/mem}"

if [[ ! -x "$MEM_BIN" ]]; then
  echo "release binary not found or not executable: $MEM_BIN" >&2
  echo "run scripts/build-release.sh first" >&2
  exit 1
fi

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/mnemark-smoke.XXXXXX")"
cleanup() {
  rm -rf "$WORKDIR"
}
trap cleanup EXIT

INSTALL_DIR="$WORKDIR/install"
RUN_DIR="$WORKDIR/run"
HOME_DIR="$WORKDIR/home"
mkdir -p "$INSTALL_DIR" "$RUN_DIR" "$HOME_DIR"
cp "$MEM_BIN" "$INSTALL_DIR/mem"

(
  cd "$RUN_DIR"
  MNEMARK_HOME="$HOME_DIR" "$INSTALL_DIR/mem" init >/dev/null
  [[ -f "$HOME_DIR/memory.db" ]]
  [[ -d "$HOME_DIR/index" ]]
  [[ ! -e "$HOME_DIR/schema/memory-schema.sql" ]]
  config_output="$(MNEMARK_HOME="$HOME_DIR" "$INSTALL_DIR/mem" config show)"
  [[ "$config_output" == *'"store_source": "environment"'* ]]
  [[ "$config_output" == *'"schema": "embedded"'* ]]
  MNEMARK_HOME="$HOME_DIR" "$INSTALL_DIR/mem" save \
    --name smoke_release \
    --source manual \
    --tags '["smoke:test"]' \
    --content "release smoke searchable content" \
    --force >/dev/null
  query_output="$(MNEMARK_HOME="$HOME_DIR" "$INSTALL_DIR/mem" query "searchable" --no-touch)"
  [[ "$query_output" == *smoke_release* ]]
  MNEMARK_HOME="$HOME_DIR" "$INSTALL_DIR/mem" reindex >/dev/null
  query_output="$(MNEMARK_HOME="$HOME_DIR" "$INSTALL_DIR/mem" query "release smoke" --no-touch)"
  [[ "$query_output" == *smoke_release* ]]
  export_output="$(MNEMARK_HOME="$HOME_DIR" "$INSTALL_DIR/mem" export --format json)"
  [[ "$export_output" == *smoke_release* ]]
)

echo "release smoke ok"
