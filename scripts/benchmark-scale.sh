#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MEM_BIN="${MEM_BIN:-$ROOT/target/release/mem}"
SCALES="${SCALES:-100 1000 10000}"

if [[ ! -x "$MEM_BIN" ]]; then
	echo "release binary not found or not executable: $MEM_BIN" >&2
	echo "run scripts/build-release.sh first" >&2
	exit 1
fi

WORKDIR="$(mktemp -d "${TMPDIR:-/tmp}/mnemark-benchmark.XXXXXX")"
trap 'rm -rf "$WORKDIR"' EXIT

MEM_BIN="$MEM_BIN" WORKDIR="$WORKDIR" SCALES="$SCALES" python3 - <<'PY'
import json
import os
import pathlib
import subprocess
import time

binary = os.environ["MEM_BIN"]
root = pathlib.Path(os.environ["WORKDIR"])
scales = [int(value) for value in os.environ["SCALES"].split()]


def run(home: pathlib.Path, *args: str) -> float:
    env = os.environ.copy()
    env["MNEMARK_HOME"] = str(home)
    started = time.perf_counter()
    subprocess.run(
        [binary, *args],
        env=env,
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=True,
        text=True,
    )
    return (time.perf_counter() - started) * 1000.0


print("scale,import_ms,query_ms,prime_ms,graph_rebuild_ms,bundle_export_ms")
for scale in scales:
    home = root / f"store-{scale}"
    payload = root / f"memories-{scale}.json"
    records = [
        {
            "type": "reference",
            "name": f"benchmark_memory_{index:05d}",
            "description": f"deterministic benchmark row {index}",
            "content": (
                "Trigger: benchmark retrieval. "
                f"Action: load deterministic row {index} for scale validation. "
                "Why: measure local agent-memory latency without embeddings."
            ),
            "tags": [f"benchmark:bucket-{index % 20}", "domain:performance"],
            "scope": "global",
            "source": "agent",
            "confidence": "medium",
        }
        for index in range(scale)
    ]
    payload.write_text(json.dumps(records), encoding="utf-8")
    run(home, "init")
    import_ms = run(home, "import", str(payload))
    query_ms = run(home, "query", "deterministic scale validation", "--limit", "20")
    prime_ms = run(home, "prime", "--budget", "4000")
    graph_ms = run(home, "graph", "rebuild")
    bundle_ms = run(home, "bundle", "export", str(root / f"store-{scale}.tgz"))
    print(
        f"{scale},{import_ms:.2f},{query_ms:.2f},{prime_ms:.2f},"
        f"{graph_ms:.2f},{bundle_ms:.2f}"
    )
PY
