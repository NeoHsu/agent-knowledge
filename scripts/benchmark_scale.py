#!/usr/bin/env python3
"""Deterministic scale benchmark runner with optional interleaved A/B execution."""

from __future__ import annotations

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
from dataclasses import dataclass
from typing import Any, TextIO

REPORT_SCHEMA_VERSION = 2
PROTOCOL_VERSION = 2
PROTOCOL_FILES = ("scripts/benchmark-scale.sh", "scripts/benchmark_scale.py")
OPERATIONS = ("import", "query", "prime", "graph_rebuild", "bundle_export")
BUNDLE_STAGES = (
    "snapshot_ms",
    "validation_ms",
    "hash_ms",
    "archive_ms",
    "install_ms",
)
BOOTSTRAP_RESAMPLES = 2_000


@dataclass(frozen=True)
class BinarySpec:
    label: str
    path: pathlib.Path
    git_commit: str
    git_dirty: bool


@dataclass(frozen=True)
class BenchmarkConfig:
    repo_root: pathlib.Path
    workdir: pathlib.Path
    scales: tuple[int, ...]
    interactive_runs: int
    maintenance_runs: int
    warmup_runs: int
    seed: int


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def protocol_sha256(repo_root: pathlib.Path) -> str:
    """Hash every executable protocol input with unambiguous path framing."""
    digest = hashlib.sha256()
    for relative in PROTOCOL_FILES:
        data = (repo_root / relative).read_bytes()
        encoded = relative.encode("utf-8")
        digest.update(len(encoded).to_bytes(4, "big"))
        digest.update(encoded)
        digest.update(len(data).to_bytes(8, "big"))
        digest.update(data)
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
    binary: pathlib.Path,
    workdir: pathlib.Path,
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
        cwd=workdir,
        stdout=subprocess.PIPE if capture_stdout else subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        check=False,
        text=True,
    )
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(args)}\n"
            f"{completed.stderr}"
        )
    rss_mib = parse_peak_rss(completed.stderr) if measure else None
    return elapsed_ms, rss_mib, completed.stdout if capture_stdout else ""


def nearest_rank(values: list[float], percentile: float) -> float:
    ordered = sorted(values)
    index = max(0, math.ceil(percentile * len(ordered)) - 1)
    return ordered[index]


def median_absolute_deviation(values: list[float]) -> float:
    median = statistics.median(values)
    return statistics.median(abs(value - median) for value in values)


def bootstrap_median_ci(
    values: list[float], seed: int, resamples: int = BOOTSTRAP_RESAMPLES
) -> tuple[float, float] | None:
    if len(values) < 2:
        return None
    rng = random.Random(seed)
    medians = [
        statistics.median(rng.choices(values, k=len(values))) for _ in range(resamples)
    ]
    return nearest_rank(medians, 0.025), nearest_rank(medians, 0.975)


def metric_seed(base_seed: int, scale: int, label: str, metric_name: str) -> int:
    material = f"{base_seed}:{scale}:{label}:{metric_name}".encode()
    return int.from_bytes(hashlib.sha256(material).digest()[:8], "big")


def summarize(
    values: list[float], *, include_p95: bool, bootstrap_seed: int
) -> dict[str, float | None]:
    interval = bootstrap_median_ci(values, bootstrap_seed)
    return {
        "median": statistics.median(values),
        "p95": nearest_rank(values, 0.95)
        if include_p95 and len(values) >= 20
        else None,
        "min": min(values),
        "max": max(values),
        "mad": median_absolute_deviation(values),
        "median_ci95_low": interval[0] if interval else None,
        "median_ci95_high": interval[1] if interval else None,
    }


def metric(
    durations: list[float],
    rss_values: list[float],
    *,
    include_p95: bool,
    input_bytes: int,
    output_bytes: int,
    bootstrap_seed: int,
) -> dict[str, Any]:
    result: dict[str, Any] = summarize(
        durations, include_p95=include_p95, bootstrap_seed=bootstrap_seed
    )
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
            f"import correctness failed for {scale}: memories={count}, "
            f"changelog={changelog}"
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
    required = [
        "BEGIN MNEMARK PRIOR DATA",
        "END MNEMARK PRIOR DATA",
        "benchmark_memory_",
    ]
    if any(value not in output for value in required):
        raise AssertionError("prime output is missing expected benchmark context")


def assert_graph(output: str, scale: int) -> None:
    report = json.loads(output)
    if report.get("status") != "rebuilt" or report.get("nodes", 0) < scale:
        raise AssertionError(f"graph rebuild correctness failed: {report}")


def assert_bundle(
    spec: BinarySpec,
    config: BenchmarkConfig,
    home: pathlib.Path,
    bundle: pathlib.Path,
) -> None:
    _, _, output = command(
        spec.path,
        config.workdir,
        home,
        "bundle",
        "inspect",
        str(bundle),
        capture_stdout=True,
    )
    report = json.loads(output)
    metadata = report.get("bundle", report)
    if metadata.get("version") != 2:
        raise AssertionError("bundle inspect did not report version 2")
    hashes = metadata.get("hashes", {})
    if "memory.db" not in hashes or not str(hashes["memory.db"]).startswith("sha256:"):
        raise AssertionError("bundle inspect did not report memory.db SHA-256")


def benchmark_records(scale: int) -> list[dict[str, Any]]:
    prime_types = ["user", "feedback", "preference", "project"]
    records = []
    for index in range(scale):
        memory_type = "reference" if index % 5 == 0 else prime_types[index % 4]
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
    return records


def shuffled_specs(
    specs: tuple[BinarySpec, ...], rng: random.Random
) -> list[BinarySpec]:
    ordered = list(specs)
    rng.shuffle(ordered)
    return ordered


def append_measurement(
    destination: dict[str, dict[str, list[float]]],
    label: str,
    operation: str,
    elapsed: float,
    rss: float | None,
) -> None:
    destination[label][f"{operation}_times"].append(elapsed)
    if rss is not None:
        destination[label][f"{operation}_rss"].append(rss)


def benchmark_scale(
    config: BenchmarkConfig,
    specs: tuple[BinarySpec, ...],
    scale: int,
    rng: random.Random,
) -> tuple[dict[str, dict[str, Any]], dict[str, Any]]:
    payload = config.workdir / f"memories-{scale}.json"
    payload.write_text(json.dumps(benchmark_records(scale)), encoding="utf-8")
    payload_bytes = payload.stat().st_size

    measurements: dict[str, dict[str, Any]] = {}
    homes: dict[str, pathlib.Path] = {}
    bundle_sizes: dict[str, list[int]] = {}
    bundle_stages: dict[str, dict[str, list[float]]] = {}
    for spec in specs:
        measurements[spec.label] = {
            f"{operation}_{suffix}": []
            for operation in OPERATIONS
            for suffix in ("times", "rss")
        }
        bundle_sizes[spec.label] = []
        bundle_stages[spec.label] = {stage: [] for stage in BUNDLE_STAGES}

    schedule: dict[str, Any] = {
        "import": [],
        "warmup": [],
        "interactive": [],
        "graph_rebuild": [],
        "bundle_export": [],
    }

    for iteration in range(config.maintenance_runs):
        ordered = shuffled_specs(specs, rng)
        schedule["import"].append([spec.label for spec in ordered])
        for spec in ordered:
            home = config.workdir / f"{spec.label}-store-{scale}-import-{iteration}"
            command(spec.path, config.workdir, home, "init")
            elapsed, rss, _ = command(
                spec.path,
                config.workdir,
                home,
                "import",
                str(payload),
                "--summary-only",
                measure=True,
            )
            assert_import(home, scale)
            append_measurement(measurements, spec.label, "import", elapsed, rss)
            homes.setdefault(spec.label, home)

    for _ in range(config.warmup_runs):
        ordered = shuffled_specs(specs, rng)
        schedule["warmup"].append([spec.label for spec in ordered])
        for spec in ordered:
            home = homes[spec.label]
            _, _, output = command(
                spec.path,
                config.workdir,
                home,
                "query",
                "deterministic scale validation",
                "--limit",
                "20",
                capture_stdout=True,
            )
            assert_query(output, scale)
            _, _, output = command(
                spec.path,
                config.workdir,
                home,
                "prime",
                "--budget",
                "4000",
                capture_stdout=True,
            )
            assert_prime(output)

    for _ in range(config.interactive_runs):
        work = [(spec, operation) for spec in specs for operation in ("query", "prime")]
        rng.shuffle(work)
        schedule["interactive"].append(
            [f"{spec.label}:{operation}" for spec, operation in work]
        )
        for spec, operation in work:
            home = homes[spec.label]
            if operation == "query":
                elapsed, rss, output = command(
                    spec.path,
                    config.workdir,
                    home,
                    "query",
                    "deterministic scale validation",
                    "--limit",
                    "20",
                    capture_stdout=True,
                    measure=True,
                )
                assert_query(output, scale)
            else:
                elapsed, rss, output = command(
                    spec.path,
                    config.workdir,
                    home,
                    "prime",
                    "--budget",
                    "4000",
                    capture_stdout=True,
                    measure=True,
                )
                assert_prime(output)
            append_measurement(measurements, spec.label, operation, elapsed, rss)

    for _ in range(config.maintenance_runs):
        ordered = shuffled_specs(specs, rng)
        schedule["graph_rebuild"].append([spec.label for spec in ordered])
        for spec in ordered:
            elapsed, rss, output = command(
                spec.path,
                config.workdir,
                homes[spec.label],
                "graph",
                "rebuild",
                capture_stdout=True,
                measure=True,
            )
            assert_graph(output, scale)
            append_measurement(measurements, spec.label, "graph_rebuild", elapsed, rss)

    for iteration in range(config.maintenance_runs):
        ordered = shuffled_specs(specs, rng)
        schedule["bundle_export"].append([spec.label for spec in ordered])
        for spec in ordered:
            home = homes[spec.label]
            bundle = config.workdir / f"{spec.label}-store-{scale}-{iteration}.tgz"
            elapsed, rss, output = command(
                spec.path,
                config.workdir,
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
            if any(stage not in profile for stage in BUNDLE_STAGES):
                raise AssertionError(f"bundle profile is incomplete: {profile}")
            for stage in BUNDLE_STAGES:
                bundle_stages[spec.label][stage].append(float(profile[stage]))
            assert_bundle(spec, config, home, bundle)
            bundle_sizes[spec.label].append(bundle.stat().st_size)
            append_measurement(measurements, spec.label, "bundle_export", elapsed, rss)

    results: dict[str, dict[str, Any]] = {}
    for spec in specs:
        label = spec.label
        home = homes[label]
        values = measurements[label]
        database_bytes = (home / "memory.db").stat().st_size
        index_bytes = directory_size(home / "index")
        store_bytes = database_bytes + index_bytes
        operation_metrics = {
            "import": metric(
                values["import_times"],
                values["import_rss"],
                include_p95=False,
                input_bytes=payload_bytes,
                output_bytes=store_bytes,
                bootstrap_seed=metric_seed(config.seed, scale, label, "import"),
            ),
            "query": metric(
                values["query_times"],
                values["query_rss"],
                include_p95=True,
                input_bytes=store_bytes,
                output_bytes=0,
                bootstrap_seed=metric_seed(config.seed, scale, label, "query"),
            ),
            "prime": metric(
                values["prime_times"],
                values["prime_rss"],
                include_p95=True,
                input_bytes=database_bytes,
                output_bytes=4000,
                bootstrap_seed=metric_seed(config.seed, scale, label, "prime"),
            ),
            "graph_rebuild": metric(
                values["graph_rebuild_times"],
                values["graph_rebuild_rss"],
                include_p95=False,
                input_bytes=database_bytes,
                output_bytes=database_bytes,
                bootstrap_seed=metric_seed(config.seed, scale, label, "graph_rebuild"),
            ),
            "bundle_export": metric(
                values["bundle_export_times"],
                values["bundle_export_rss"],
                include_p95=False,
                input_bytes=database_bytes,
                output_bytes=max(bundle_sizes[label]),
                bootstrap_seed=metric_seed(config.seed, scale, label, "bundle_export"),
            ),
        }
        operation_metrics["bundle_export"]["stages_ms"] = {
            stage: summarize(
                stage_values,
                include_p95=False,
                bootstrap_seed=metric_seed(
                    config.seed, scale, label, f"bundle_export:{stage}"
                ),
            )
            for stage, stage_values in bundle_stages[label].items()
        }
        results[label] = {
            "scale": scale,
            "sizes": {
                "payload_bytes": payload_bytes,
                "database_bytes": database_bytes,
                "index_bytes": index_bytes,
                "bundle_bytes": max(bundle_sizes[label]),
            },
            "operations": operation_metrics,
        }
    return results, schedule


def repository_state(repo_root: pathlib.Path) -> tuple[str, bool]:
    commit = subprocess.check_output(
        ["git", "-C", str(repo_root), "rev-parse", "HEAD"], text=True
    ).strip()
    status = subprocess.check_output(
        ["git", "-C", str(repo_root), "status", "--porcelain", "--untracked-files=no"],
        text=True,
    )
    return commit, bool(status.strip())


def build_report(
    config: BenchmarkConfig,
    spec: BinarySpec,
    results: list[dict[str, Any]],
    execution_order: list[int],
    sample_schedule: dict[str, Any],
    protocol_hash: str,
    comparison_peer: BinarySpec | None,
) -> dict[str, Any]:
    return {
        "schema_version": REPORT_SCHEMA_VERSION,
        "benchmark_protocol_version": PROTOCOL_VERSION,
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "platform": platform.platform(),
        "python": platform.python_version(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "binary": str(spec.path),
        "binary_sha256": sha256(spec.path),
        # Kept for compatibility with the regression checker; v2 hashes every
        # protocol input rather than only the shell entrypoint.
        "benchmark_script_sha256": protocol_hash,
        "benchmark_protocol_sha256": protocol_hash,
        "benchmark_protocol_files": list(PROTOCOL_FILES),
        "mem_version": subprocess.check_output(
            [str(spec.path), "--version"], text=True
        ).strip(),
        "git_commit": spec.git_commit,
        "git_dirty": spec.git_dirty,
        "run_label": spec.label,
        "comparison_mode": comparison_peer is not None,
        "comparison_peer": (
            {
                "label": comparison_peer.label,
                "binary_sha256": sha256(comparison_peer.path),
                "git_commit": comparison_peer.git_commit,
            }
            if comparison_peer
            else None
        ),
        "cache_model": {
            "import": "fresh initialized store per measured run",
            "interactive": (
                "separate process per run after configured warmups; OS cache retained"
            ),
            "maintenance": "same populated store for graph and bundle repetitions",
            "comparison": (
                "sample-level seeded interleaving across binaries"
                if comparison_peer
                else "single binary"
            ),
        },
        "seed": config.seed,
        "execution_order": execution_order,
        "sample_schedule": sample_schedule,
        "interactive_runs": config.interactive_runs,
        "maintenance_runs": config.maintenance_runs,
        "warmup_runs": config.warmup_runs,
        "statistic_model": {
            "center": "median",
            "dispersion": "median absolute deviation",
            "confidence_interval": (
                f"deterministic nonparametric bootstrap median, "
                f"{BOOTSTRAP_RESAMPLES} resamples"
            ),
            "p95": "nearest-rank, emitted only with at least 20 samples",
        },
        "results": results,
    }


def write_csv(results: list[dict[str, Any]], output: TextIO) -> None:
    writer = csv.writer(output, lineterminator="\n")
    writer.writerow(
        [
            "scale",
            "operation",
            "runs",
            "median_ms",
            "p95_ms",
            "mad_ms",
            "median_ci95_low_ms",
            "median_ci95_high_ms",
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
                    f"{values['mad']:.2f}",
                    ""
                    if values["median_ci95_low"] is None
                    else f"{values['median_ci95_low']:.2f}",
                    ""
                    if values["median_ci95_high"] is None
                    else f"{values['median_ci95_high']:.2f}",
                    f"{values['min']:.2f}",
                    f"{values['max']:.2f}",
                    ""
                    if values["peak_rss_mib"] is None
                    else f"{values['peak_rss_mib']:.2f}",
                    f"{values['input_bytes'] / (1024 * 1024):.2f}",
                    f"{values['output_bytes'] / (1024 * 1024):.2f}",
                ]
            )


def write_report(report: dict[str, Any], destination: pathlib.Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def bool_from_env(name: str, default: bool = False) -> bool:
    raw = os.environ.get(name)
    if raw is None:
        return default
    if raw not in {"0", "1"}:
        raise SystemExit(f"{name} must be 0 or 1")
    return raw == "1"


def config_from_env() -> tuple[BenchmarkConfig, tuple[BinarySpec, ...]]:
    repo_root = pathlib.Path(os.environ["REPO_ROOT"]).resolve()
    candidate_commit, candidate_dirty = repository_state(repo_root)
    candidate = BinarySpec(
        label="candidate",
        path=pathlib.Path(os.environ["MEM_BIN"]).resolve(),
        git_commit=candidate_commit,
        git_dirty=candidate_dirty,
    )
    specs = [candidate]
    baseline_binary = os.environ.get("BASELINE_MEM_BIN", "").strip()
    if baseline_binary:
        baseline_commit = os.environ.get("BASELINE_GIT_COMMIT", "").strip()
        if not baseline_commit:
            raise SystemExit(
                "BASELINE_GIT_COMMIT is required when BASELINE_MEM_BIN is set"
            )
        specs.append(
            BinarySpec(
                label="baseline",
                path=pathlib.Path(baseline_binary).resolve(),
                git_commit=baseline_commit,
                git_dirty=bool_from_env("BASELINE_GIT_DIRTY"),
            )
        )
    config = BenchmarkConfig(
        repo_root=repo_root,
        workdir=pathlib.Path(os.environ["WORKDIR"]).resolve(),
        scales=tuple(int(value) for value in os.environ["SCALES"].split()),
        interactive_runs=int(os.environ["INTERACTIVE_RUNS"]),
        maintenance_runs=int(os.environ["MAINTENANCE_RUNS"]),
        warmup_runs=int(os.environ["WARMUP_RUNS"]),
        seed=int(os.environ["BENCHMARK_SEED"]),
    )
    return config, tuple(specs)


def main() -> int:
    config, specs = config_from_env()
    if not config.scales or any(scale < 20 for scale in config.scales):
        raise SystemExit("SCALES must contain integers of at least 20")

    rng = random.Random(config.seed)
    execution_order = list(config.scales)
    rng.shuffle(execution_order)
    print(f"benchmark execution order: {execution_order}", file=sys.stderr)

    per_label_results: dict[str, dict[int, dict[str, Any]]] = {
        spec.label: {} for spec in specs
    }
    schedules: dict[str, Any] = {}
    for scale in execution_order:
        scale_results, schedule = benchmark_scale(config, specs, scale, rng)
        schedules[str(scale)] = schedule
        for label, result in scale_results.items():
            per_label_results[label][scale] = result

    protocol_hash = protocol_sha256(config.repo_root)
    reports: dict[str, dict[str, Any]] = {}
    for spec in specs:
        peer = next((candidate for candidate in specs if candidate != spec), None)
        ordered_results = [
            per_label_results[spec.label][scale] for scale in config.scales
        ]
        reports[spec.label] = build_report(
            config,
            spec,
            ordered_results,
            execution_order,
            schedules,
            protocol_hash,
            peer,
        )

    write_csv(reports["candidate"]["results"], sys.stdout)
    report_file = os.environ.get("REPORT_FILE", "").strip()
    if report_file:
        report_path = pathlib.Path(report_file)
        write_report(reports["candidate"], report_path)
        print(f"benchmark JSON report: {report_path}", file=sys.stderr)

    if "baseline" in reports:
        baseline_report_file = pathlib.Path(os.environ["BASELINE_REPORT_FILE"])
        write_report(reports["baseline"], baseline_report_file)
        print(
            f"baseline benchmark JSON report: {baseline_report_file}", file=sys.stderr
        )
        baseline_csv_file = os.environ.get("BASELINE_CSV_FILE", "").strip()
        if baseline_csv_file:
            baseline_csv_path = pathlib.Path(baseline_csv_file)
            baseline_csv_path.parent.mkdir(parents=True, exist_ok=True)
            with baseline_csv_path.open("w", encoding="utf-8", newline="") as handle:
                write_csv(reports["baseline"]["results"], handle)
            print(f"baseline benchmark CSV: {baseline_csv_path}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
