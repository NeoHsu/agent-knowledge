#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="/home/node/bin:$PATH"

/home/node/.cargo/bin/cargo build --release --manifest-path "$ROOT/Cargo.toml"
"$ROOT/target/release/mem" --version
