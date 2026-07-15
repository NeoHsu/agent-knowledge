#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="/home/node/bin:$PATH"
if [[ -n "${CARGO:-}" ]]; then
	CARGO_BIN="$CARGO"
elif [[ -x /home/node/.cargo/bin/cargo ]]; then
	CARGO_BIN=/home/node/.cargo/bin/cargo
else
	CARGO_BIN=cargo
fi

"$CARGO_BIN" build --release --locked --manifest-path "$ROOT/Cargo.toml"
MEM_BIN="$ROOT/target/release/mem"
if [[ "${OS:-}" == "Windows_NT" ]]; then
	MEM_BIN+=".exe"
fi
expected_version="$(awk -F'"' '/^version = "/ { print $2; exit }' "$ROOT/Cargo.toml")"
actual_version="$("$MEM_BIN" --version)"
if [[ -z "$expected_version" || "$actual_version" != "mem $expected_version" ]]; then
	echo "release binary version mismatch: expected mem $expected_version, got $actual_version" >&2
	exit 1
fi
printf '%s\n' "$actual_version"
