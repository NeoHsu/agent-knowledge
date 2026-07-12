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
"$MEM_BIN" --version
