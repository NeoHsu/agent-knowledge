#!/usr/bin/env python3
"""Validate benchmark reports against portable guardrails or a retained baseline."""

from __future__ import annotations

import argparse
import json
import math
import pathlib
import sys
from typing import Any

GUARDRAIL_SCHEMA_VERSION = 1
SUPPORTED_REPORT_SCHEMA_VERSIONS = {1, 2}


def load_json(path: pathlib.Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"cannot read benchmark JSON {path}: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit(f"benchmark JSON must be an object: {path}")
    return value


def finite_number(value: Any, label: str, errors: list[str]) -> float | None:
    if not isinstance(value, (int, float)) or isinstance(value, bool):
        errors.append(f"{label} must be numeric")
        return None
    try:
        number = float(value)
    except (TypeError, ValueError, OverflowError):
        errors.append(f"{label} must be numeric")
        return None
    if not math.isfinite(number) or number < 0:
        errors.append(f"{label} must be finite and non-negative")
        return None
    return number


def validate_report_schema(
    report: dict[str, Any], label: str, errors: list[str]
) -> int | None:
    version = report.get("schema_version")
    if version not in SUPPORTED_REPORT_SCHEMA_VERSIONS:
        supported = ", ".join(
            str(value) for value in sorted(SUPPORTED_REPORT_SCHEMA_VERSIONS)
        )
        errors.append(f"{label} schema_version must be one of: {supported}")
        return None
    if version == 2:
        if report.get("benchmark_protocol_version") != 2:
            errors.append(f"{label}.benchmark_protocol_version must be 2")
        protocol_hash = report.get("benchmark_protocol_sha256")
        if not isinstance(protocol_hash, str) or not protocol_hash:
            errors.append(
                f"{label}.benchmark_protocol_sha256 must be a non-empty string"
            )
        elif protocol_hash != report.get("benchmark_script_sha256"):
            errors.append(
                f"{label} protocol and compatibility script hashes must match"
            )
        if not isinstance(report.get("sample_schedule"), dict):
            errors.append(f"{label}.sample_schedule must be an object")
        if not isinstance(report.get("statistic_model"), dict):
            errors.append(f"{label}.statistic_model must be an object")
    return version


def results_by_scale(
    report: dict[str, Any], errors: list[str]
) -> dict[int, dict[str, Any]]:
    raw_results = report.get("results")
    if not isinstance(raw_results, list) or not raw_results:
        errors.append("report.results must be a non-empty array")
        return {}
    results: dict[int, dict[str, Any]] = {}
    for index, result in enumerate(raw_results):
        if not isinstance(result, dict):
            errors.append(f"report.results[{index}] must be an object")
            continue
        scale = result.get("scale")
        if not isinstance(scale, int) or isinstance(scale, bool) or scale <= 0:
            errors.append(f"report.results[{index}].scale must be a positive integer")
            continue
        if scale in results:
            errors.append(f"report contains duplicate scale {scale}")
            continue
        results[scale] = result
    return results


def check_guardrails(
    report_results: dict[int, dict[str, Any]],
    guardrails: dict[str, Any],
    errors: list[str],
) -> None:
    if guardrails.get("schema_version") != GUARDRAIL_SCHEMA_VERSION:
        errors.append(f"guardrail schema_version must be {GUARDRAIL_SCHEMA_VERSION}")
        return
    configured = guardrails.get("scales")
    if not isinstance(configured, dict):
        errors.append("guardrails.scales must be an object")
        return

    for scale, result in sorted(report_results.items()):
        scale_limits = configured.get(str(scale))
        if not isinstance(scale_limits, dict):
            errors.append(f"no guardrails configured for scale {scale}")
            continue
        operations = result.get("operations")
        if not isinstance(operations, dict):
            errors.append(f"scale {scale} operations must be an object")
            continue
        configured_operations = scale_limits.get("operations")
        if not isinstance(configured_operations, dict):
            errors.append(f"scale {scale} guardrail operations must be an object")
            continue
        for operation, limits in configured_operations.items():
            metrics = operations.get(operation)
            if not isinstance(metrics, dict):
                errors.append(f"scale {scale} missing operation {operation}")
                continue
            if not isinstance(limits, dict):
                errors.append(f"scale {scale} {operation} limits must be an object")
                continue
            median = finite_number(
                metrics.get("median"),
                f"scale {scale} {operation} median",
                errors,
            )
            maximum = finite_number(
                limits.get("median_ms_max"),
                f"scale {scale} {operation} median_ms_max",
                errors,
            )
            if median is not None and maximum is not None and median > maximum:
                errors.append(
                    f"scale {scale} {operation} median {median:.2f} ms exceeds "
                    f"guardrail {maximum:.2f} ms"
                )


def check_baseline(
    report: dict[str, Any],
    report_results: dict[int, dict[str, Any]],
    baseline: dict[str, Any],
    max_regression_percent: float,
    allow_platform_mismatch: bool,
    allow_script_mismatch: bool,
    errors: list[str],
) -> None:
    baseline_errors: list[str] = []
    baseline_results = results_by_scale(baseline, baseline_errors)
    errors.extend(f"baseline: {error}" for error in baseline_errors)
    candidate_version = validate_report_schema(report, "report", errors)
    baseline_version = validate_report_schema(baseline, "baseline", errors)
    if candidate_version != baseline_version:
        errors.append("candidate and baseline report schema versions differ")
    if baseline.get("git_dirty") is not False:
        errors.append("baseline benchmark report must come from a clean Git worktree")

    for field in ["interactive_runs", "maintenance_runs", "warmup_runs"]:
        if report.get(field) != baseline.get(field):
            errors.append(f"candidate and baseline {field} differ")

    if candidate_version == 2 and baseline_version == 2:
        if (
            report.get("comparison_mode") is not True
            or baseline.get("comparison_mode") is not True
        ):
            errors.append(
                "schema v2 baseline comparisons must come from one interleaved run"
            )
        if report.get("sample_schedule") != baseline.get("sample_schedule"):
            errors.append("candidate and baseline sample schedules differ")
        candidate_peer = report.get("comparison_peer")
        baseline_peer = baseline.get("comparison_peer")
        if not isinstance(candidate_peer, dict) or not isinstance(baseline_peer, dict):
            errors.append("interleaved reports must identify their comparison peers")
        else:
            if candidate_peer.get("binary_sha256") != baseline.get("binary_sha256"):
                errors.append(
                    "candidate comparison peer does not match baseline binary"
                )
            if baseline_peer.get("binary_sha256") != report.get("binary_sha256"):
                errors.append(
                    "baseline comparison peer does not match candidate binary"
                )
            if candidate_peer.get("git_commit") != baseline.get("git_commit"):
                errors.append(
                    "candidate comparison peer does not match baseline commit"
                )
            if baseline_peer.get("git_commit") != report.get("git_commit"):
                errors.append(
                    "baseline comparison peer does not match candidate commit"
                )
    if not allow_platform_mismatch and report.get("platform") != baseline.get(
        "platform"
    ):
        errors.append(
            "candidate and baseline platforms differ; pass --allow-platform-mismatch only for an intentional comparison"
        )
    if not allow_script_mismatch and report.get(
        "benchmark_script_sha256"
    ) != baseline.get("benchmark_script_sha256"):
        errors.append(
            "candidate and baseline benchmark scripts differ; pass --allow-script-mismatch only after reviewing the protocol change"
        )

    multiplier = 1.0 + max_regression_percent / 100.0
    for scale, result in sorted(report_results.items()):
        baseline_result = baseline_results.get(scale)
        if baseline_result is None:
            errors.append(f"baseline does not contain scale {scale}")
            continue
        operations = result.get("operations", {})
        baseline_operations = baseline_result.get("operations", {})
        if not isinstance(operations, dict) or not isinstance(
            baseline_operations, dict
        ):
            errors.append(f"scale {scale} operations are malformed")
            continue
        for operation, metrics in operations.items():
            baseline_metrics = baseline_operations.get(operation)
            if not isinstance(metrics, dict) or not isinstance(baseline_metrics, dict):
                errors.append(
                    f"baseline does not contain scale {scale} operation {operation}"
                )
                continue
            candidate_median = finite_number(
                metrics.get("median"), f"scale {scale} {operation} median", errors
            )
            baseline_median = finite_number(
                baseline_metrics.get("median"),
                f"baseline scale {scale} {operation} median",
                errors,
            )
            if candidate_median is None or baseline_median is None:
                continue
            limit = baseline_median * multiplier
            if candidate_median > limit:
                errors.append(
                    f"scale {scale} {operation} median {candidate_median:.2f} ms "
                    f"exceeds {max_regression_percent:.1f}% regression limit "
                    f"{limit:.2f} ms (baseline {baseline_median:.2f} ms)"
                )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", required=True, type=pathlib.Path)
    parser.add_argument("--guardrails", required=True, type=pathlib.Path)
    parser.add_argument("--baseline", type=pathlib.Path)
    parser.add_argument("--max-regression-percent", type=float, default=35.0)
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--allow-platform-mismatch", action="store_true")
    parser.add_argument("--allow-script-mismatch", action="store_true")
    args = parser.parse_args()

    if not 0 <= args.max_regression_percent <= 1000:
        parser.error("--max-regression-percent must be between 0 and 1000")

    report = load_json(args.report)
    guardrails = load_json(args.guardrails)
    errors: list[str] = []
    validate_report_schema(report, "report", errors)
    git_dirty = report.get("git_dirty")
    if args.require_clean and (not isinstance(git_dirty, bool) or git_dirty):
        errors.append("benchmark report must come from a clean Git worktree")
    for field in [
        "binary_sha256",
        "benchmark_script_sha256",
        "mem_version",
        "git_commit",
        "platform",
    ]:
        if not isinstance(report.get(field), str) or not report[field]:
            errors.append(f"report.{field} must be a non-empty string")

    report_results = results_by_scale(report, errors)
    check_guardrails(report_results, guardrails, errors)
    if args.baseline is not None:
        check_baseline(
            report,
            report_results,
            load_json(args.baseline),
            args.max_regression_percent,
            args.allow_platform_mismatch,
            args.allow_script_mismatch,
            errors,
        )

    if errors:
        sys.stderr.write("benchmark regression check failed:\n")
        for error in errors:
            sys.stderr.write(f"- {error}\n")
        return 1

    scales = ", ".join(str(scale) for scale in sorted(report_results))
    suffix = " with retained-baseline comparison" if args.baseline else ""
    print(f"benchmark guardrails ok for scales {scales}{suffix}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
