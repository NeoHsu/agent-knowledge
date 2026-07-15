#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
default_mem_bin="$ROOT/target/release/mem"
if [[ "${OS:-}" == "Windows_NT" ]]; then
	default_mem_bin+=".exe"
fi
MEM_BIN="${MEM_BIN:-$default_mem_bin}"

if [[ ! -x "$MEM_BIN" ]]; then
	echo "release binary not found or not executable: $MEM_BIN" >&2
	echo "run scripts/build-release.sh first" >&2
	exit 1
fi

expected_version="$(awk -F'"' '/^version = "/ { print $2; exit }' "$ROOT/Cargo.toml")"
actual_version="$("$MEM_BIN" --version)"
if [[ -z "$expected_version" || "$actual_version" != "mem $expected_version" ]]; then
	echo "release binary version mismatch: expected mem $expected_version, got $actual_version" >&2
	echo "run scripts/build-release.sh before smoke validation" >&2
	exit 1
fi

temp_root="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
if command -v cygpath >/dev/null 2>&1; then
	temp_root="$(cygpath -u "$temp_root")"
fi
WORKDIR="$(mktemp -d "$temp_root/mnemark-smoke.XXXXXX")"
cleanup() {
	rm -rf "$WORKDIR"
}
trap cleanup EXIT

INSTALL_DIR="$WORKDIR/install"
RUN_DIR="$WORKDIR/run"
HOME_DIR="$WORKDIR/home"
mkdir -p "$INSTALL_DIR" "$RUN_DIR" "$HOME_DIR"
runtime_home="$HOME_DIR"
if [[ "${OS:-}" == "Windows_NT" ]]; then
	runtime_home="$(cygpath -w "$HOME_DIR")"
fi
install_bin="$INSTALL_DIR/mem"
if [[ "$MEM_BIN" == *.exe ]]; then
	install_bin+=".exe"
fi
cp "$MEM_BIN" "$install_bin"

(
	cd "$RUN_DIR"
	MNEMARK_HOME="$runtime_home" "$install_bin" init >/dev/null
	[[ -f "$HOME_DIR/memory.db" ]]
	[[ -d "$HOME_DIR/index" ]]
	[[ ! -e "$HOME_DIR/schema/memory-schema.sql" ]]
	config_output="$(MNEMARK_HOME="$runtime_home" "$install_bin" config show)"
	[[ "$config_output" == *'"store_source": "environment"'* ]]
	[[ "$config_output" == *'"schema": "embedded"'* ]]
	MNEMARK_HOME="$runtime_home" "$install_bin" save \
		--name smoke_release \
		--source manual \
		--user-confirmed \
		--tags '["smoke:test"]' \
		--content "release smoke searchable content" \
		--force >/dev/null
	query_output="$(MNEMARK_HOME="$runtime_home" "$install_bin" query "searchable")"
	[[ "$query_output" == *smoke_release* ]]
	MNEMARK_HOME="$runtime_home" "$install_bin" reindex >/dev/null
	query_output="$(MNEMARK_HOME="$runtime_home" "$install_bin" query "release smoke" --explain-score)"
	[[ "$query_output" == *smoke_release* ]]
	[[ "$query_output" == *retrieval_score* ]]
	MNEMARK_HOME="$runtime_home" "$install_bin" save \
		--name smoke_cjk \
		--content "release 部署流程 checklist" \
		--force >/dev/null
	cjk_output="$(MNEMARK_HOME="$runtime_home" "$install_bin" query "releaze 部署流成" --fuzzy)"
	[[ "$cjk_output" == *smoke_cjk* ]]
	set +e
	json_error="$(MNEMARK_HOME="$runtime_home" "$install_bin" --json-errors query "smoke" --limit 10001 2>&1)"
	json_status=$?
	set -e
	((json_status != 0))
	[[ "$json_error" == *'"status":"error"'* ]]
	[[ "$json_error" == *'"code":"command_failed"'* ]]
	MNEMARK_HOME="$runtime_home" "$install_bin" save \
		--name smoke_target \
		--content "graph traversal target content" \
		--force >/dev/null
	cat >"$WORKDIR/semantic.json" <<'JSON'
{"schema_version":1,"edges":[{"source":"smoke_release","target":"smoke_target","relation":"depends_on","confidence":"EXTRACTED","evidence":"The smoke release explicitly depends on the target."}]}
JSON
	MNEMARK_HOME="$runtime_home" "$install_bin" graph ingest "$WORKDIR/semantic.json" >/dev/null
	graph_output="$(MNEMARK_HOME="$runtime_home" "$install_bin" graph path smoke_release smoke_target --direction outgoing)"
	[[ "$graph_output" == *'"status": "ok"'* ]]
	export_output="$(MNEMARK_HOME="$runtime_home" "$install_bin" export --format json)"
	[[ "$export_output" == *smoke_release* ]]
	MNEMARK_HOME="$runtime_home" "$install_bin" bundle export "$WORKDIR/store.tgz" >/dev/null
	bundle_output="$(MNEMARK_HOME="$runtime_home" "$install_bin" bundle inspect "$WORKDIR/store.tgz")"
	[[ "$bundle_output" == *'"version": 2'* ]]
	[[ "$bundle_output" == *'sha256:'* ]]
)

echo "release smoke ok"
