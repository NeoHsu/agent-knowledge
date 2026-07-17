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
	echo "recovery drill binary version mismatch: expected mem $expected_version, got $actual_version" >&2
	exit 1
fi

work_root="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
if command -v cygpath >/dev/null 2>&1; then
	work_root="$(cygpath -u "$work_root")"
fi
WORKDIR="$(mktemp -d "$work_root/mnemark-recovery.XXXXXX")"
cleanup() {
	rm -rf "$WORKDIR"
}
trap cleanup EXIT

SOURCE_HOME="$WORKDIR/source"
RESTORED_HOME="$WORKDIR/restored"
RUN_DIR="$WORKDIR/run"
mkdir -p "$SOURCE_HOME" "$RESTORED_HOME" "$RUN_DIR"

native_path() {
	if [[ "${OS:-}" == "Windows_NT" ]] && command -v cygpath >/dev/null 2>&1; then
		cygpath -w "$1"
	else
		printf '%s\n' "$1"
	fi
}

run_mem() {
	local home="$1"
	shift
	(
		cd "$RUN_DIR"
		MNEMARK_HOME="$(native_path "$home")" "$MEM_BIN" "$@"
	)
}

run_mem "$SOURCE_HOME" contract >"$WORKDIR/contract.json"
run_mem "$SOURCE_HOME" init >/dev/null
run_mem "$SOURCE_HOME" save \
	--type reference \
	--name recovery_anchor \
	--source manual \
	--user-confirmed \
	--tags '["drill:recovery"]' \
	--content "Recovery drill anchor retained across a verified bundle restore." \
	--force >/dev/null
run_mem "$SOURCE_HOME" save \
	--type reference \
	--name recovery_target \
	--tags '["drill:recovery"]' \
	--content "Recovery drill graph target retained across restore." \
	--force >/dev/null

cat >"$WORKDIR/recovery-workflow.yaml" <<'YAML'
schema_version: 1
goal: Verify that a portable store can be restored without losing durable state.
triggers:
  - production recovery drill
steps:
  - id: inspect
    check: source and destination stores are isolated temporary paths
    verify: no live store is selected
  - id: restore
    manual: import the verified bundle into the empty destination
    verify: memory, workflow, graph, and artifact checks succeed
stop_conditions:
  - a checksum or correctness check fails
post_run_memory:
  - record any recovery failure as a durable corrective action
YAML
run_mem "$SOURCE_HOME" save \
	--type workflow \
	--name recovery_workflow \
	--tags '["workflow:recovery","intent:recovery-drill"]' \
	--content-file "$WORKDIR/recovery-workflow.yaml" \
	--force >/dev/null
run_mem "$SOURCE_HOME" workflow validate recovery_workflow >/dev/null
run_mem "$SOURCE_HOME" workflow record recovery_workflow \
	--result success \
	--note "pre-backup recovery checkpoint" >/dev/null

mkdir -p "$SOURCE_HOME/artifacts/scripts"
printf '%s\n' '#!/usr/bin/env sh' 'printf "recovery helper ok\\n"' \
	>"$SOURCE_HOME/artifacts/scripts/recovery-helper.sh"
chmod +x "$SOURCE_HOME/artifacts/scripts/recovery-helper.sh"
run_mem "$SOURCE_HOME" artifact add artifacts/scripts/recovery-helper.sh \
	--name recovery-helper \
	--kind script \
	--scope global \
	--executable >/dev/null
run_mem "$SOURCE_HOME" artifact check >/dev/null

cat >"$WORKDIR/semantic.json" <<'JSON'
{"schema_version":1,"edges":[{"source":"recovery_anchor","target":"recovery_target","relation":"evidence_for","confidence":"EXTRACTED","evidence":"The recovery anchor explicitly verifies the recovery target."}]}
JSON
run_mem "$SOURCE_HOME" graph ingest "$WORKDIR/semantic.json" >/dev/null
run_mem "$SOURCE_HOME" graph path recovery_anchor recovery_target --direction outgoing \
	>"$WORKDIR/source-graph.json"
run_mem "$SOURCE_HOME" export --format json >"$WORKDIR/source-memories.json"

BUNDLE="$WORKDIR/store.tgz"
run_mem "$SOURCE_HOME" bundle export "$BUNDLE" >/dev/null
run_mem "$SOURCE_HOME" bundle inspect "$BUNDLE" >"$WORKDIR/bundle-inspect.json"
run_mem "$RESTORED_HOME" bundle import "$BUNDLE" >"$WORKDIR/bundle-import.json"

run_mem "$RESTORED_HOME" query "Recovery drill anchor" --no-touch \
	>"$WORKDIR/restored-query.json"
run_mem "$RESTORED_HOME" workflow show recovery_workflow \
	>"$WORKDIR/restored-workflow.json"
run_mem "$RESTORED_HOME" artifact check >"$WORKDIR/restored-artifacts.json"
run_mem "$RESTORED_HOME" graph path recovery_anchor recovery_target --direction outgoing \
	>"$WORKDIR/restored-graph.json"
run_mem "$RESTORED_HOME" doctor >"$WORKDIR/restored-doctor.json"
run_mem "$RESTORED_HOME" export --format json >"$WORKDIR/restored-memories.json"
cmp "$WORKDIR/source-memories.json" "$WORKDIR/restored-memories.json"

python3 - "$WORKDIR" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
contract = json.loads((root / "contract.json").read_text())
if contract.get("contract_version") != 1:
    raise SystemExit("contract version check failed")
inspect = json.loads((root / "bundle-inspect.json").read_text())
if inspect.get("checksums_verified") is not True:
    raise SystemExit("bundle checksums were not verified")
imported = json.loads((root / "bundle-import.json").read_text())
if imported.get("status") != "imported" or imported.get("mode") != "clean":
    raise SystemExit("clean bundle import check failed")
query = json.loads((root / "restored-query.json").read_text())
if not any(row.get("name") == "recovery_anchor" for row in query):
    raise SystemExit("restored memory query failed")
workflow = json.loads((root / "restored-workflow.json").read_text())
if workflow.get("name") != "recovery_workflow":
    raise SystemExit("restored workflow check failed")
artifacts = json.loads((root / "restored-artifacts.json").read_text())
if artifacts.get("status") != "ok":
    raise SystemExit("restored artifact check failed")
graph = json.loads((root / "restored-graph.json").read_text())
if graph.get("status") != "ok" or not graph.get("edges"):
    raise SystemExit("restored graph path check failed")
doctor = json.loads((root / "restored-doctor.json").read_text())
errors = [check for check in doctor.get("checks", []) if check.get("status") == "error"]
if errors:
    raise SystemExit(f"restored store doctor errors: {errors}")
PY

CORRUPT_BUNDLE="$WORKDIR/store-corrupt.tgz"
python3 - "$BUNDLE" "$CORRUPT_BUNDLE" "$WORKDIR/corrupt" <<'PY'
import pathlib
import shutil
import sys
import tarfile

source = pathlib.Path(sys.argv[1])
destination = pathlib.Path(sys.argv[2])
root = pathlib.Path(sys.argv[3])
root.mkdir()
with tarfile.open(source, "r:gz") as archive:
    for member in archive.getmembers():
        if not member.isfile():
            continue
        relative = pathlib.PurePosixPath(member.name)
        if relative.is_absolute() or ".." in relative.parts:
            raise SystemExit(f"unsafe fixture member: {member.name}")
        target = root.joinpath(*relative.parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        extracted = archive.extractfile(member)
        if extracted is None:
            raise SystemExit(f"cannot extract fixture member: {member.name}")
        with target.open("wb") as output:
            shutil.copyfileobj(extracted, output)

database = root / "memory.db"
payload = bytearray(database.read_bytes())
if not payload:
    raise SystemExit("bundle database is empty")
payload[-1] ^= 0x01
database.write_bytes(payload)
with tarfile.open(destination, "w:gz") as archive:
    paths = sorted(root.rglob("*"), key=lambda path: (len(path.parts), path.as_posix()))
    for path in paths:
        archive.add(
            path,
            arcname=path.relative_to(root).as_posix(),
            recursive=False,
        )
PY

if run_mem "$RESTORED_HOME" bundle inspect "$CORRUPT_BUNDLE" \
	>"$WORKDIR/corrupt-stdout.txt" 2>"$WORKDIR/corrupt-stderr.txt"; then
	echo "corrupt bundle unexpectedly passed inspection" >&2
	exit 1
fi
if ! grep -q "bundle checksum mismatch for memory.db" "$WORKDIR/corrupt-stderr.txt"; then
	echo "corrupt bundle failed for an unexpected reason" >&2
	cat "$WORKDIR/corrupt-stderr.txt" >&2
	exit 1
fi
run_mem "$RESTORED_HOME" query "Recovery drill anchor" --no-touch >/dev/null

run_mem "$SOURCE_HOME" migrate --dry-run >"$WORKDIR/migrate-dry-run.json"
git -C "$SOURCE_HOME" init -b main >/dev/null
git -C "$SOURCE_HOME" config user.email "recovery-drill@example.invalid"
git -C "$SOURCE_HOME" config user.name "Recovery Drill"
git -C "$SOURCE_HOME" config commit.gpgsign false
run_mem "$SOURCE_HOME" sync --dry-run >"$WORKDIR/sync-dry-run.json"
run_mem "$SOURCE_HOME" sync >"$WORKDIR/sync-local.json"

python3 - "$WORKDIR" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
migration = json.loads((root / "migrate-dry-run.json").read_text())
if migration.get("migration_required") is not False:
    raise SystemExit("current-schema migration dry run was not clean")
sync_dry_run = json.loads((root / "sync-dry-run.json").read_text())
if sync_dry_run.get("status") != "dry_run":
    raise SystemExit("sync dry run check failed")
sync = json.loads((root / "sync-local.json").read_text())
if sync.get("status") != "local_only" or sync.get("committed") is not True:
    raise SystemExit("local sync checkpoint check failed")
PY

printf '%s\n' "recovery drill ok"
