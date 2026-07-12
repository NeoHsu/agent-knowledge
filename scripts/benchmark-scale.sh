#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MEM_BIN="${MEM_BIN:-$ROOT/target/release/mem}"
SCALES="${SCALES:-100 1000 10000}"
INTERACTIVE_RUNS="${INTERACTIVE_RUNS:-20}"
MAINTENANCE_RUNS="${MAINTENANCE_RUNS:-3}"
WARMUP_RUNS="${WARMUP_RUNS:-1}"
BENCHMARK_SEED="${BENCHMARK_SEED:-42}"
REPORT_FILE="${REPORT_FILE:-}"

if [[ ! -x "$MEM_BIN" ]]; then
	echo "release binary not found or not executable: $MEM_BIN" >&2
	echo "run scripts/build-release.sh first" >&2
	exit 1
fi

for value in "$INTERACTIVE_RUNS" "$MAINTENANCE_RUNS" "$WARMUP_RUNS"; do
	if [[ ! "$value" =~ ^[0-9]+$ ]]; then
		echo "benchmark run counts must be non-negative integers" >&2
		exit 1
	fi
done
if ((INTERACTIVE_RUNS < 1 || MAINTENANCE_RUNS < 1)); then
	echo "INTERACTIVE_RUNS and MAINTENANCE_RUNS must be at least 1" >&2
	exit 1
fi

work_root="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
if command -v cygpath >/dev/null 2>&1; then
	work_root="$(cygpath -u "$work_root")"
fi
WORKDIR="$(mktemp -d "$work_root/mnemark-benchmark.XXXXXX")"
trap 'rm -rf "$WORKDIR"' EXIT

MEM_BIN="$MEM_BIN" \
	REPO_ROOT="$ROOT" \
	WORKDIR="$WORKDIR" \
	SCALES="$SCALES" \
	INTERACTIVE_RUNS="$INTERACTIVE_RUNS" \
	MAINTENANCE_RUNS="$MAINTENANCE_RUNS" \
	WARMUP_RUNS="$WARMUP_RUNS" \
	BENCHMARK_SEED="$BENCHMARK_SEED" \
	REPORT_FILE="$REPORT_FILE" \
	python3 - <<'PY'
import csv
import hashlib
import json
import math
import os
import pathlib
import platform
import random
import sqlite3
import statistics
import subprocess
import sys
import time
from typing import Any

binary = pathlib.Path(os.environ["MEM_BIN"]).resolve()
repo_root = pathlib.Path(os.environ["REPO_ROOT"])
root = pathlib.Path(os.environ["WORKDIR"])
scales = [int(value) for value in os.environ["SCALES"].split()]
interactive_runs = int(os.environ["INTERACTIVE_RUNS"])
maintenance_runs = int(os.environ["MAINTENANCE_RUNS"])
warmup_runs = int(os.environ["WARMUP_RUNS"])
seed = int(os.environ["BENCHMARK_SEED"])
report_file = os.environ["REPORT_FILE"]

if not scales or any(scale < 20 for scale in scales):
    raise SystemExit("SCALES must contain integers of at least 20")


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def directory_size(path: pathlib.Path) -> int:
    if not path.exists():
        return 0
    return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())


def parse_peak_rss(stderr: str) -> float | None:
    for line in stderr.splitlines():
        stripped = line.strip()
        if "Maximum resident set size (kbytes):" in stripped:
            return float(stripped.rsplit(":", 1)[1].strip()) / 1024.0
        if stripped.endswith("maximum resident set size"):
            return float(stripped.split()[0]) / (1024.0 * 1024.0)
    return None


def command(
    home: pathlib.Path,
    *args: str,
    capture_stdout: bool = False,
    measure: bool = False,
) -> tuple[float, float | None, str]:
    env = os.environ.copy()
    env["MNEMARK_HOME"] = str(home)
    invocation = [str(binary), *args]
    if measure and pathlib.Path("/usr/bin/time").exists():
        if sys.platform == "darwin":
            invocation = ["/usr/bin/time", "-l", *invocation]
        elif sys.platform.startswith("linux"):
            invocation = ["/usr/bin/time", "-v", *invocation]
    started = time.perf_counter()
    completed = subprocess.run(
        invocation,
        env=env,
        cwd=root,
        stdout=subprocess.PIPE if capture_stdout else subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
    )
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(args)}\n{completed.stderr}"
        )
    rss_mib = parse_peak_rss(completed.stderr) if measure else None
    return elapsed_ms, rss_mib, completed.stdout if capture_stdout else ""


def nearest_rank(values: list[float], percentile: float) -> float:
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def summarize(values: list[float], include_p95: bool = True) -> dict[str, float | None]:
    return {
        "median": statistics.median(values),
        "p95": nearest_rank(values, 0.95) if include_p95 and len(values) >= 20 else None,
        "min": min(values),
        "max": max(values),
    }


def metric(
    durations: list[float],
    rss_values: list[float],
    *,
    include_p95: bool,
    input_bytes: int,
    output_bytes: int,
) -> dict[str, Any]:
    result: dict[str, Any] = summarize(durations, include_p95=include_p95)
    result.update(
        {
            "runs": len(durations),
            "peak_rss_mib": max(rss_values) if rss_values else None,
            "input_bytes": input_bytes,
            "output_bytes": output_bytes,
        }
    )
    return result


def assert_import(home: pathlib.Path, scale: int) -> None:
    with sqlite3.connect(home / "memory.db") as connection:
        count = connection.execute("SELECT COUNT(*) FROM memories").fetchone()[0]
        changelog = connection.execute("SELECT COUNT(*) FROM changelog").fetchone()[0]
    if count != scale or changelog != scale:
        raise AssertionError(
            f"import correctness failed for {scale}: memories={count}, changelog={changelog}"
        )


def assert_query(output: str, scale: int) -> None:
    rows = json.loads(output)
    if len(rows) != min(20, scale):
        raise AssertionError(f"query returned {len(rows)} rows for scale {scale}")
    if any(not row["name"].startswith("benchmark_memory_") for row in rows):
        raise AssertionError("query returned an unexpected memory")


def assert_prime(output: str) -> None:
    if len(output) > 4000:
        raise AssertionError(f"prime exceeded hard budget: {len(output)}")
    required = ["BEGIN MNEMARK PRIOR DATA", "END MNEMARK PRIOR DATA", "benchmark_memory_"]
    if any(value not in output for value in required):
        raise AssertionError("prime output is missing expected benchmark context")


def assert_graph(output: str, scale: int) -> None:
    report = json.loads(output)
    if report.get("status") != "rebuilt" or report.get("nodes", 0) < scale:
        raise AssertionError(f"graph rebuild correctness failed: {report}")


def assert_bundle(home: pathlib.Path, bundle: pathlib.Path) -> None:
    _, _, output = command(home, "bundle", "inspect", str(bundle), capture_stdout=True)
    report = json.loads(output)
    metadata = report.get("bundle", report)
    if metadata.get("version") != 2:
        raise AssertionError("bundle inspect did not report version 2")
    hashes = metadata.get("hashes", {})
    if "memory.db" not in hashes or not str(hashes["memory.db"]).startswith("sha256:"):
        raise AssertionError("bundle inspect did not report memory.db SHA-256")


def benchmark_scale(scale: int) -> dict[str, Any]:
    payload = root / f"memories-{scale}.json"
    prime_types = ["user", "feedback", "preference", "project"]
    records = []
    for index in range(scale):
        memory_type = "reference" if index % 5 == 0 else prime_types[index % len(prime_types)]
        records.append(
            {
                "type": memory_type,
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
        )
    payload.write_text(json.dumps(records), encoding="utf-8")

    import_times: list[float] = []
    import_rss: list[float] = []
    canonical_home: pathlib.Path | None = None
    for iteration in range(maintenance_runs):
        home = root / f"store-{scale}-import-{iteration}"
        command(home, "init")
        elapsed, rss, _ = command(home, "import", str(payload), measure=True)
        assert_import(home, scale)
        import_times.append(elapsed)
        if rss is not None:
            import_rss.append(rss)
        canonical_home = canonical_home or home
    assert canonical_home is not None
    home = canonical_home

    for _ in range(warmup_runs):
        _, _, output = command(
            home,
            "query",
            "deterministic scale validation",
            "--limit",
            "20",
            capture_stdout=True,
        )
        assert_query(output, scale)
        _, _, output = command(home, "prime", "--budget", "4000", capture_stdout=True)
        assert_prime(output)

    query_times: list[float] = []
    query_rss: list[float] = []
    prime_times: list[float] = []
    prime_rss: list[float] = []
    for _ in range(interactive_runs):
        elapsed, rss, output = command(
            home,
            "query",
            "deterministic scale validation",
            "--limit",
            "20",
            capture_stdout=True,
            measure=True,
        )
        assert_query(output, scale)
        query_times.append(elapsed)
        if rss is not None:
            query_rss.append(rss)

        elapsed, rss, output = command(
            home,
            "prime",
            "--budget",
            "4000",
            capture_stdout=True,
            measure=True,
        )
        assert_prime(output)
        prime_times.append(elapsed)
        if rss is not None:
            prime_rss.append(rss)

    graph_times: list[float] = []
    graph_rss: list[float] = []
    for _ in range(maintenance_runs):
        elapsed, rss, output = command(
            home, "graph", "rebuild", capture_stdout=True, measure=True
        )
        assert_graph(output, scale)
        graph_times.append(elapsed)
        if rss is not None:
            graph_rss.append(rss)

    bundle_times: list[float] = []
    bundle_rss: list[float] = []
    bundle_sizes: list[int] = []
    bundle_stages: dict[str, list[float]] = {
        stage: []
        for stage in ["snapshot_ms", "validation_ms", "hash_ms", "archive_ms", "install_ms"]
    }
    for iteration in range(maintenance_runs):
        bundle = root / f"store-{scale}-{iteration}.tgz"
        elapsed, rss, output = command(
            home,
            "bundle",
            "export",
            str(bundle),
            "--profile",
            capture_stdout=True,
            measure=True,
        )
        export_report = json.loads(output)
        profile = export_report.get("profile", {})
        if any(stage not in profile for stage in bundle_stages):
            raise AssertionError(f"bundle profile is incomplete: {profile}")
        for stage in bundle_stages:
            bundle_stages[stage].append(float(profile[stage]))
        assert_bundle(home, bundle)
        bundle_times.append(elapsed)
        bundle_sizes.append(bundle.stat().st_size)
        if rss is not None:
            bundle_rss.append(rss)

    database_bytes = (home / "memory.db").stat().st_size
    index_bytes = directory_size(home / "index")
    store_bytes = database_bytes + index_bytes
    bundle_metric = metric(
        bundle_times,
        bundle_rss,
        include_p95=False,
        input_bytes=database_bytes,
        output_bytes=max(bundle_sizes),
    )
    bundle_metric["stages_ms"] = {
        stage: summarize(values, include_p95=False)
        for stage, values in bundle_stages.items()
    }
    return {
        "scale": scale,
        "sizes": {
            "payload_bytes": payload.stat().st_size,
            "database_bytes": database_bytes,
            "index_bytes": index_bytes,
            "bundle_bytes": max(bundle_sizes),
        },
        "operations": {
            "import": metric(
                import_times,
                import_rss,
                include_p95=False,
                input_bytes=payload.stat().st_size,
                output_bytes=store_bytes,
            ),
            "query": metric(
                query_times,
                query_rss,
                include_p95=True,
                input_bytes=store_bytes,
                output_bytes=0,
            ),
            "prime": metric(
                prime_times,
                prime_rss,
                include_p95=True,
                input_bytes=database_bytes,
                output_bytes=4000,
            ),
            "graph_rebuild": metric(
                graph_times,
                graph_rss,
                include_p95=False,
                input_bytes=database_bytes,
                output_bytes=database_bytes,
            ),
            "bundle_export": bundle_metric,
        },
    }


execution_order = scales.copy()
random.Random(seed).shuffle(execution_order)
print(f"benchmark execution order: {execution_order}", file=sys.stderr)
results_by_scale = {scale: benchmark_scale(scale) for scale in execution_order}
results = [results_by_scale[scale] for scale in scales]

git_status = subprocess.check_output(
    ["git", "-C", str(repo_root), "status", "--porcelain", "--untracked-files=no"],
    text=True,
)
metadata = {
    "schema_version": 1,
    "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    "platform": platform.platform(),
    "python": platform.python_version(),
    "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
    "binary": str(binary),
    "binary_sha256": sha256(binary),
    "benchmark_script_sha256": sha256(repo_root / "scripts" / "benchmark-scale.sh"),
    "mem_version": subprocess.check_output([str(binary), "--version"], text=True).strip(),
    "git_commit": subprocess.check_output(
        ["git", "-C", str(repo_root), "rev-parse", "HEAD"], text=True
    ).strip(),
    "git_dirty": bool(git_status.strip()),
    "cache_model": {
        "import": "fresh initialized store per measured run",
        "interactive": "separate process per run after configured warmups; OS cache retained",
        "maintenance": "same populated store for graph and bundle repetitions",
    },
    "seed": seed,
    "execution_order": execution_order,
    "interactive_runs": interactive_runs,
    "maintenance_runs": maintenance_runs,
    "warmup_runs": warmup_runs,
    "results": results,
}

writer = csv.writer(sys.stdout, lineterminator="\n")
writer.writerow(
    [
        "scale",
        "operation",
        "runs",
        "median_ms",
        "p95_ms",
        "min_ms",
        "max_ms",
        "peak_rss_mib",
        "input_mib",
        "output_mib",
    ]
)
for result in results:
    for operation, values in result["operations"].items():
        writer.writerow(
            [
                result["scale"],
                operation,
                values["runs"],
                f"{values['median']:.2f}",
                "" if values["p95"] is None else f"{values['p95']:.2f}",
                f"{values['min']:.2f}",
                f"{values['max']:.2f}",
                ""
                if values["peak_rss_mib"] is None
                else f"{values['peak_rss_mib']:.2f}",
                f"{values['input_bytes'] / (1024 * 1024):.2f}",
                f"{values['output_bytes'] / (1024 * 1024):.2f}",
            ]
        )

if report_file:
    report_path = pathlib.Path(report_file)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(f"benchmark JSON report: {report_path}", file=sys.stderr)
PY
