#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALLOW_DIRTY="${ALLOW_DIRTY:-0}"
RUN_BENCHMARK="${RUN_BENCHMARK:-1}"
REQUIRE_AUX_TOOLS="${REQUIRE_AUX_TOOLS:-1}"
BENCHMARK_SCALES="${BENCHMARK_SCALES:-100 1000}"
REPORT_FILE="${REPORT_FILE:-$ROOT/target/production-benchmark.json}"
CSV_FILE="${CSV_FILE:-$ROOT/target/production-benchmark.csv}"
GUARDRAILS_FILE="${GUARDRAILS_FILE:-$ROOT/scripts/benchmark-guardrails.json}"
BASELINE_FILE="${BASELINE_FILE:-}"
MAX_REGRESSION_PERCENT="${MAX_REGRESSION_PERCENT:-35}"
RELEASE_TAG="${RELEASE_TAG:-}"

for value in "$ALLOW_DIRTY" "$RUN_BENCHMARK" "$REQUIRE_AUX_TOOLS"; do
	if [[ "$value" != "0" && "$value" != "1" ]]; then
		echo "ALLOW_DIRTY, RUN_BENCHMARK, and REQUIRE_AUX_TOOLS must be 0 or 1" >&2
		exit 1
	fi
done

expected_version="$(awk -F'"' '/^version = "/ { print $2; exit }' Cargo.toml)"
if [[ -z "$expected_version" ]]; then
	echo "cannot determine workspace version from Cargo.toml" >&2
	exit 1
fi

if [[ -z "$RELEASE_TAG" && "$ALLOW_DIRTY" != "1" ]]; then
	RELEASE_TAG="v$expected_version"
fi

metadata_args=()
if [[ "$ALLOW_DIRTY" == "1" ]]; then
	metadata_args+=(--allow-dirty)
	echo "warning: ALLOW_DIRTY=1; this run cannot qualify a release commit" >&2
fi
if [[ -n "$RELEASE_TAG" ]]; then
	metadata_args+=(--release-tag "$RELEASE_TAG")
fi
python3 scripts/check-release-metadata.py "${metadata_args[@]}"
python3 scripts/check-source-hygiene.py

for command in cargo python3 git; do
	if ! command -v "$command" >/dev/null 2>&1; then
		echo "required command is unavailable: $command" >&2
		exit 1
	fi
done
if ! command -v cargo-audit >/dev/null 2>&1; then
	echo "cargo-audit is required; install it before running the production gate" >&2
	exit 1
fi

if command -v shellcheck >/dev/null 2>&1; then
	shellcheck scripts/*.sh
elif [[ "$REQUIRE_AUX_TOOLS" == "1" ]]; then
	echo "shellcheck is required when REQUIRE_AUX_TOOLS=1" >&2
	exit 1
else
	echo "warning: shellcheck is unavailable; shell lint was skipped" >&2
fi
if command -v actionlint >/dev/null 2>&1; then
	actionlint .github/workflows/*.yml
elif [[ "$REQUIRE_AUX_TOOLS" == "1" ]]; then
	echo "actionlint is required when REQUIRE_AUX_TOOLS=1" >&2
	exit 1
else
	echo "warning: actionlint is unavailable; workflow lint was skipped" >&2
fi

printf '%s\n' '== release helper tests =='
python3 -m unittest discover -s scripts/tests -p 'test_*.py'
printf '%s\n' '== formatting =='
cargo fmt --all -- --check
printf '%s\n' '== clippy =='
env -u CC -u CXX cargo clippy --workspace --locked --all-targets -- -D warnings
printf '%s\n' '== tests =='
env -u CC -u CXX cargo test --workspace --locked
printf '%s\n' '== dependency audit =='
cargo audit --deny warnings
printf '%s\n' '== dependency provenance and licenses =='
python3 scripts/check-dependency-policy.py
printf '%s\n' '== release build =='
scripts/build-release.sh
printf '%s\n' '== release smoke and recovery drill =='
scripts/smoke-release.sh

if [[ "$RUN_BENCHMARK" == "1" ]]; then
	mkdir -p "$(dirname "$REPORT_FILE")" "$(dirname "$CSV_FILE")"
	printf '%s\n' "== bounded benchmark: $BENCHMARK_SCALES =="
	SCALES="$BENCHMARK_SCALES" \
		INTERACTIVE_RUNS="${INTERACTIVE_RUNS:-2}" \
		MAINTENANCE_RUNS="${MAINTENANCE_RUNS:-1}" \
		WARMUP_RUNS="${WARMUP_RUNS:-0}" \
		REPORT_FILE="$REPORT_FILE" \
		scripts/benchmark-scale.sh | tee "$CSV_FILE"

	benchmark_args=(
		--report "$REPORT_FILE"
		--guardrails "$GUARDRAILS_FILE"
	)
	if [[ "$ALLOW_DIRTY" != "1" ]]; then
		benchmark_args+=(--require-clean)
	fi
	if [[ -n "$BASELINE_FILE" ]]; then
		benchmark_args+=(
			--baseline "$BASELINE_FILE"
			--max-regression-percent "$MAX_REGRESSION_PERCENT"
		)
	fi
	python3 scripts/check-benchmark-regression.py "${benchmark_args[@]}"
fi

printf '%s\n' "release readiness ok for mem $expected_version"
if [[ "$ALLOW_DIRTY" == "1" ]]; then
	printf '%s\n' 'development validation only: rerun from a clean worktree before tagging'
fi
