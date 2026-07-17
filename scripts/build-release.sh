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

# mise.toml provides Zig as a portable fallback, but cc-rs receives an
# incompatible `arm64-apple-macosx` target from some native macOS dependencies.
# Prefer the host compiler only for this known inherited override; preserve any
# other explicitly selected toolchain.
if [[ "$(uname -s)" == "Darwin" && ("${CC:-}" == "zig cc" || "${CXX:-}" == "zig c++") ]]; then
	echo "build-release: ignoring inherited Zig CC/CXX for native macOS build" >&2
	unset CC CXX
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
