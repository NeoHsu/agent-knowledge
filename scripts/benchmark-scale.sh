#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MEM_BIN="${MEM_BIN:-$ROOT/target/release/mem}"
BASELINE_MEM_BIN="${BASELINE_MEM_BIN:-}"
SCALES="${SCALES:-100 1000 10000}"
INTERACTIVE_RUNS="${INTERACTIVE_RUNS:-20}"
MAINTENANCE_RUNS="${MAINTENANCE_RUNS:-5}"
WARMUP_RUNS="${WARMUP_RUNS:-1}"
BENCHMARK_SEED="${BENCHMARK_SEED:-42}"
REPORT_FILE="${REPORT_FILE:-}"
BASELINE_REPORT_FILE="${BASELINE_REPORT_FILE:-}"
BASELINE_CSV_FILE="${BASELINE_CSV_FILE:-}"

if [[ ! -x "$MEM_BIN" ]]; then
	echo "release binary not found or not executable: $MEM_BIN" >&2
	echo "run scripts/build-release.sh first" >&2
	exit 1
fi

expected_version="$(awk -F'"' '/^version = "/ { print $2; exit }' "$ROOT/Cargo.toml")"
actual_version="$("$MEM_BIN" --version)"
if [[ "${ALLOW_VERSION_MISMATCH:-0}" != "1" && (-z "$expected_version" || "$actual_version" != "mem $expected_version") ]]; then
	echo "benchmark binary version mismatch: expected mem $expected_version, got $actual_version" >&2
	echo "rebuild first, or set ALLOW_VERSION_MISMATCH=1 for an intentional cross-version comparison" >&2
	exit 1
fi

for value in "$INTERACTIVE_RUNS" "$MAINTENANCE_RUNS" "$WARMUP_RUNS" "$BENCHMARK_SEED"; do
	if [[ ! "$value" =~ ^[0-9]+$ ]]; then
		echo "benchmark run counts and seed must be non-negative integers" >&2
		exit 1
	fi
done
if ((INTERACTIVE_RUNS < 1 || MAINTENANCE_RUNS < 1)); then
	echo "INTERACTIVE_RUNS and MAINTENANCE_RUNS must be at least 1" >&2
	exit 1
fi

if [[ -n "$BASELINE_MEM_BIN" ]]; then
	if [[ ! -x "$BASELINE_MEM_BIN" ]]; then
		echo "baseline binary not found or not executable: $BASELINE_MEM_BIN" >&2
		exit 1
	fi
	if [[ -z "${BASELINE_GIT_COMMIT:-}" ]]; then
		echo "BASELINE_GIT_COMMIT is required for interleaved comparison" >&2
		exit 1
	fi
	if [[ -z "$BASELINE_REPORT_FILE" ]]; then
		if [[ "$REPORT_FILE" == *.json ]]; then
			BASELINE_REPORT_FILE="${REPORT_FILE%.json}.baseline.json"
		else
			echo "BASELINE_REPORT_FILE is required when BASELINE_MEM_BIN is set" >&2
			exit 1
		fi
	fi
fi

work_root="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
if command -v cygpath >/dev/null 2>&1; then
	work_root="$(cygpath -u "$work_root")"
fi
WORKDIR="$(mktemp -d "$work_root/mnemark-benchmark.XXXXXX")"
trap 'rm -rf "$WORKDIR"' EXIT

MEM_BIN="$MEM_BIN" \
	BASELINE_MEM_BIN="$BASELINE_MEM_BIN" \
	BASELINE_GIT_COMMIT="${BASELINE_GIT_COMMIT:-}" \
	BASELINE_GIT_DIRTY="${BASELINE_GIT_DIRTY:-0}" \
	BASELINE_REPORT_FILE="$BASELINE_REPORT_FILE" \
	BASELINE_CSV_FILE="$BASELINE_CSV_FILE" \
	REPO_ROOT="$ROOT" \
	WORKDIR="$WORKDIR" \
	SCALES="$SCALES" \
	INTERACTIVE_RUNS="$INTERACTIVE_RUNS" \
	MAINTENANCE_RUNS="$MAINTENANCE_RUNS" \
	WARMUP_RUNS="$WARMUP_RUNS" \
	BENCHMARK_SEED="$BENCHMARK_SEED" \
	REPORT_FILE="$REPORT_FILE" \
	python3 "$ROOT/scripts/benchmark_scale.py"
