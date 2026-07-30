#!/usr/bin/env python3
"""Run deterministic retrieval-quality cases against an isolated mnemark store."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import platform
import re
import subprocess
import sys
import tempfile
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_FIXTURE = ROOT / "evals" / "retrieval-v1.json"
DEFAULT_REPORT = ROOT / "target" / "retrieval-eval.json"


class EvaluationError(ValueError):
    """Raised when the fixture, command, or output violates the eval contract."""


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_object(path: pathlib.Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvaluationError(f"cannot read {label} {path}: {error}") from error
    if not isinstance(value, dict):
        raise EvaluationError(f"{label} must be a JSON object: {path}")
    return value


def require_list(value: Any, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise EvaluationError(f"{label} must be an array")
    return value


def require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise EvaluationError(f"{label} must be a non-empty string")
    return value


def require_rate(value: Any, label: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise EvaluationError(f"{label} must be a number between 0 and 1")
    rate = value * 1.0
    if not 0 <= rate <= 1:
        raise EvaluationError(f"{label} must be between 0 and 1")
    return rate


def validate_fixture(fixture: dict[str, Any]) -> None:
    if fixture.get("schema_version") != 1:
        raise EvaluationError("retrieval fixture schema_version must be 1")
    require_string(fixture.get("name"), "fixture.name")
    memories = require_list(fixture.get("memories"), "fixture.memories")
    query_cases = require_list(fixture.get("query_cases"), "fixture.query_cases")
    prime_cases = require_list(fixture.get("prime_cases"), "fixture.prime_cases")
    thresholds = fixture.get("thresholds")
    if not isinstance(thresholds, dict):
        raise EvaluationError("fixture.thresholds must be an object")
    if not memories or not query_cases or not prime_cases:
        raise EvaluationError(
            "fixture must include memories, query_cases, and prime_cases"
        )

    memory_names: set[str] = set()
    for index, memory in enumerate(memories):
        if not isinstance(memory, dict):
            raise EvaluationError(f"memories[{index}] must be an object")
        name = require_string(memory.get("name"), f"memories[{index}].name")
        if name in memory_names:
            raise EvaluationError(f"duplicate memory name in fixture: {name}")
        memory_names.add(name)
        require_string(memory.get("type"), f"memories[{index}].type")
        require_string(memory.get("scope"), f"memories[{index}].scope")
        require_string(memory.get("source"), f"memories[{index}].source")
        require_string(memory.get("content"), f"memories[{index}].content")
        tags = require_list(memory.get("tags"), f"memories[{index}].tags")
        if not all(isinstance(tag, str) and tag for tag in tags):
            raise EvaluationError(f"memories[{index}].tags must contain strings")

    case_ids: set[str] = set()
    for group_name, cases in (
        ("query_cases", query_cases),
        ("prime_cases", prime_cases),
    ):
        for index, case in enumerate(cases):
            if not isinstance(case, dict):
                raise EvaluationError(f"{group_name}[{index}] must be an object")
            case_id = require_string(case.get("id"), f"{group_name}[{index}].id")
            if case_id in case_ids:
                raise EvaluationError(f"duplicate evaluation case id: {case_id}")
            case_ids.add(case_id)

    for key in (
        "query_case_pass_rate_min",
        "mean_recall_at_k_min",
        "mean_reciprocal_rank_min",
        "prime_case_pass_rate_min",
        "graph_evidence_rate_min",
    ):
        require_rate(thresholds.get(key), f"thresholds.{key}")


def run_mem(
    binary: pathlib.Path,
    home: pathlib.Path,
    run_dir: pathlib.Path,
    *args: str,
) -> str:
    command = [str(binary), "--home", str(home), *args]
    environment = os.environ.copy()
    environment.pop("MNEMARK_HOME", None)
    completed = subprocess.run(
        command,
        cwd=run_dir,
        env=environment,
        capture_output=True,
        text=True,
        encoding="utf-8",
        check=False,
        timeout=60,
    )
    if completed.returncode != 0:
        rendered = " ".join(args)
        raise EvaluationError(
            f"mem command failed ({completed.returncode}): {rendered}\n"
            f"stdout={completed.stdout}\nstderr={completed.stderr}"
        )
    return completed.stdout


def seed_store(
    binary: pathlib.Path,
    home: pathlib.Path,
    run_dir: pathlib.Path,
    fixture: dict[str, Any],
) -> None:
    run_mem(binary, home, run_dir, "init")
    for memory in fixture["memories"]:
        args = [
            "save",
            "--type",
            memory["type"],
            "--name",
            memory["name"],
            "--scope",
            memory["scope"],
            "--source",
            memory["source"],
            "--tags",
            json.dumps(memory["tags"], ensure_ascii=False, separators=(",", ":")),
            "--content",
            memory["content"],
            "--force",
        ]
        if memory["source"] == "manual":
            args.append("--user-confirmed")
        run_mem(binary, home, run_dir, *args)

    semantic_edges = fixture.get("semantic_edges", [])
    if semantic_edges:
        edge_file = run_dir / "semantic-edges.json"
        edge_file.write_text(
            json.dumps(
                {"schema_version": 1, "edges": semantic_edges},
                ensure_ascii=False,
                indent=2,
            ),
            encoding="utf-8",
        )
        run_mem(binary, home, run_dir, "graph", "ingest", str(edge_file))


def parse_json_output(output: str, label: str) -> Any:
    try:
        return json.loads(output)
    except json.JSONDecodeError as error:
        raise EvaluationError(f"{label} did not emit valid JSON: {error}") from error


def evaluate_query_case(
    binary: pathlib.Path,
    home: pathlib.Path,
    run_dir: pathlib.Path,
    case: dict[str, Any],
) -> dict[str, Any]:
    case_id = require_string(case.get("id"), "query case id")
    query = require_string(case.get("query"), f"{case_id}.query")
    scope = require_string(case.get("scope"), f"{case_id}.scope")
    limit = case.get("limit")
    if not isinstance(limit, int) or limit < 1:
        raise EvaluationError(f"{case_id}.limit must be a positive integer")
    relevant = require_list(case.get("relevant"), f"{case_id}.relevant")
    if not relevant or not all(isinstance(name, str) and name for name in relevant):
        raise EvaluationError(f"{case_id}.relevant must contain memory names")
    forbidden = require_list(case.get("forbidden", []), f"{case_id}.forbidden")

    args = [
        "query",
        query,
        "--scope",
        scope,
        "--limit",
        str(limit),
        "--format",
        "json",
    ]
    if case.get("fuzzy", False):
        args.append("--fuzzy")
    rows = parse_json_output(run_mem(binary, home, run_dir, *args), case_id)
    if not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows):
        raise EvaluationError(f"{case_id} query output must be an array of objects")
    names = [row.get("name") for row in rows if isinstance(row.get("name"), str)]
    relevant_set = set(relevant)
    retrieved_relevant = [name for name in names if name in relevant_set]
    recall = len(set(retrieved_relevant)) / len(relevant_set)
    first_rank = next(
        (index for index, name in enumerate(names, start=1) if name in relevant_set),
        None,
    )
    reciprocal_rank = 0.0 if first_rank is None else 1.0 / first_rank
    expected_top = case.get("expected_top")
    top_ok = expected_top is None or (bool(names) and names[0] == expected_top)
    forbidden_found = sorted({name for name in names if name in set(forbidden)})
    minimum_recall = require_rate(
        case.get("recall_at_k_min", 1.0), f"{case_id}.recall_at_k_min"
    )
    passed = recall >= minimum_recall and top_ok and not forbidden_found
    return {
        "id": case_id,
        "query": query,
        "limit": limit,
        "fuzzy": bool(case.get("fuzzy", False)),
        "retrieved": names,
        "relevant": relevant,
        "recall_at_k": round(recall, 6),
        "reciprocal_rank": round(reciprocal_rank, 6),
        "expected_top": expected_top,
        "top_match": top_ok,
        "forbidden_found": forbidden_found,
        "passed": passed,
    }


def graph_labels(graph_context: dict[str, Any]) -> set[str]:
    labels: set[str] = set()
    for node in graph_context.get("start_nodes", []):
        if isinstance(node, dict) and isinstance(node.get("label"), str):
            labels.add(node["label"])
    for item in graph_context.get("nodes", []):
        if not isinstance(item, dict):
            continue
        node = item.get("node")
        if isinstance(node, dict) and isinstance(node.get("label"), str):
            labels.add(node["label"])
    return labels


def evaluate_prime_case(
    binary: pathlib.Path,
    home: pathlib.Path,
    run_dir: pathlib.Path,
    case: dict[str, Any],
) -> tuple[dict[str, Any], int, int]:
    case_id = require_string(case.get("id"), "prime case id")
    raw_focus = case.get("focus")
    focus = None if raw_focus is None else require_string(raw_focus, f"{case_id}.focus")
    scope = require_string(case.get("scope"), f"{case_id}.scope")
    budget = case.get("budget")
    per_section = case.get("per_section")
    if not isinstance(budget, int) or budget < 1:
        raise EvaluationError(f"{case_id}.budget must be a positive integer")
    if not isinstance(per_section, int) or per_section < 1:
        raise EvaluationError(f"{case_id}.per_section must be a positive integer")

    args = [
        "prime",
        "--scope",
        scope,
        "--budget",
        str(budget),
        "--per-section",
        str(per_section),
        "--format",
        "json",
    ]
    if focus is not None:
        args[1:1] = ["--focus", focus]
    output = run_mem(binary, home, run_dir, *args)
    report = parse_json_output(output, case_id)
    if not isinstance(report, dict):
        raise EvaluationError(f"{case_id} prime output must be an object")
    sections = report.get("sections")
    raw_graph_context = report.get("graph_context")
    if not isinstance(sections, dict):
        raise EvaluationError(f"{case_id} prime output is missing sections")
    if focus is None:
        if raw_graph_context is not None:
            raise EvaluationError(
                f"{case_id} plain prime unexpectedly emitted graph context"
            )
        graph_context: dict[str, Any] = {}
    elif isinstance(raw_graph_context, dict):
        graph_context = raw_graph_context
    else:
        raise EvaluationError(f"{case_id} focused prime is missing graph_context")

    section_names = {
        entry["name"]
        for entries in sections.values()
        if isinstance(entries, list)
        for entry in entries
        if isinstance(entry, dict) and isinstance(entry.get("name"), str)
    }
    labels = graph_labels(graph_context)
    edges = graph_context.get("edges", [])
    if not isinstance(edges, list):
        raise EvaluationError(f"{case_id}.graph_context.edges must be an array")

    expected_sections = set(
        require_list(
            case.get("expected_section_names", []), f"{case_id}.expected_section_names"
        )
    )
    expected_graph = set(
        require_list(
            case.get("expected_graph_names", []), f"{case_id}.expected_graph_names"
        )
    )
    expected_relations = set(
        require_list(
            case.get("expected_relations_with_evidence", []),
            f"{case_id}.expected_relations_with_evidence",
        )
    )
    relations_with_evidence: set[str] = set()
    for edge in edges:
        if not isinstance(edge, dict):
            continue
        relation = edge.get("relation")
        evidence = edge.get("evidence")
        if (
            isinstance(relation, str)
            and isinstance(evidence, str)
            and bool(evidence.strip())
        ):
            relations_with_evidence.add(relation)
    sections_missing = sorted(expected_sections - section_names)
    graph_names_missing = sorted(expected_graph - labels)
    relations_missing_evidence = sorted(expected_relations - relations_with_evidence)
    graph_status_ok = focus is None or graph_context.get("status") == "ok"
    status_ok = report.get("status") == "ok" and graph_status_ok
    passed = (
        status_ok
        and not sections_missing
        and not graph_names_missing
        and not relations_missing_evidence
    )
    result = {
        "id": case_id,
        "focus": focus,
        "section_names": sorted(section_names),
        "graph_names": sorted(labels),
        "graph_relations_with_evidence": sorted(relations_with_evidence),
        "sections_missing": sections_missing,
        "graph_names_missing": graph_names_missing,
        "relations_missing_evidence": relations_missing_evidence,
        "passed": passed,
    }
    return (
        result,
        len(expected_relations & relations_with_evidence),
        len(expected_relations),
    )


def repository_state(root: pathlib.Path) -> tuple[str | None, bool | None]:
    try:
        commit = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            check=True,
            timeout=10,
        ).stdout.strip()
        dirty = bool(
            subprocess.run(
                ["git", "-C", str(root), "status", "--porcelain"],
                capture_output=True,
                text=True,
                encoding="utf-8",
                check=True,
                timeout=10,
            ).stdout.strip()
        )
        return commit, dirty
    except (OSError, subprocess.SubprocessError):
        return None, None


def workspace_version(root: pathlib.Path) -> str | None:
    try:
        text = (root / "Cargo.toml").read_text(encoding="utf-8")
    except OSError:
        return None
    match = re.search(r'^version = "([^"]+)"$', text, flags=re.MULTILINE)
    return match.group(1) if match else None


def write_report(report: dict[str, Any], destination: pathlib.Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    temporary.write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    temporary.replace(destination)


def main() -> int:
    parser = argparse.ArgumentParser()
    default_binary = (
        ROOT / "target" / "release" / ("mem.exe" if os.name == "nt" else "mem")
    )
    parser.add_argument("--mem-bin", type=pathlib.Path, default=default_binary)
    parser.add_argument("--fixture", type=pathlib.Path, default=DEFAULT_FIXTURE)
    parser.add_argument("--report", type=pathlib.Path, default=DEFAULT_REPORT)
    parser.add_argument(
        "--allow-version-mismatch",
        action="store_true",
        help="allow an intentional binary/workspace version mismatch",
    )
    args = parser.parse_args()

    binary = args.mem_bin.resolve()
    fixture_path = args.fixture.resolve()
    if not binary.is_file():
        sys.stderr.write(
            f"retrieval evaluation failed: mem binary not found: {binary}\n"
        )
        return 1

    try:
        fixture = load_object(fixture_path, "retrieval fixture")
        validate_fixture(fixture)
        mem_version = subprocess.run(
            [str(binary), "--version"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            check=True,
            timeout=10,
        ).stdout.strip()
        expected_version = workspace_version(ROOT)
        if (
            not args.allow_version_mismatch
            and expected_version is not None
            and mem_version != f"mem {expected_version}"
        ):
            raise EvaluationError(
                f"binary version mismatch: expected mem {expected_version}, got {mem_version}"
            )

        with tempfile.TemporaryDirectory(prefix="mnemark-retrieval-eval-") as raw:
            work_root = pathlib.Path(raw)
            home = work_root / "store"
            run_dir = work_root / "run"
            run_dir.mkdir()
            seed_store(binary, home, run_dir, fixture)
            query_results = [
                evaluate_query_case(binary, home, run_dir, case)
                for case in fixture["query_cases"]
            ]
            prime_results: list[dict[str, Any]] = []
            evidence_found = 0
            evidence_expected = 0
            for case in fixture["prime_cases"]:
                result, found, expected = evaluate_prime_case(
                    binary, home, run_dir, case
                )
                prime_results.append(result)
                evidence_found += found
                evidence_expected += expected

        query_count = len(query_results)
        prime_count = len(prime_results)
        metrics = {
            "query_case_pass_rate": sum(case["passed"] for case in query_results)
            / query_count,
            "mean_recall_at_k": sum(case["recall_at_k"] for case in query_results)
            / query_count,
            "mean_reciprocal_rank": sum(
                case["reciprocal_rank"] for case in query_results
            )
            / query_count,
            "prime_case_pass_rate": sum(case["passed"] for case in prime_results)
            / prime_count,
            "graph_evidence_rate": (
                1.0 if evidence_expected == 0 else evidence_found / evidence_expected
            ),
        }
        metrics = {key: round(value, 6) for key, value in metrics.items()}
        thresholds = fixture["thresholds"]
        threshold_results = {
            metric: metrics[metric.removesuffix("_min")]
            >= require_rate(minimum, f"thresholds.{metric}")
            for metric, minimum in thresholds.items()
        }
        passed = all(threshold_results.values())
        commit, dirty = repository_state(ROOT)
        report = {
            "schema_version": 1,
            "fixture": fixture["name"],
            "fixture_sha256": sha256(fixture_path),
            "binary_sha256": sha256(binary),
            "mem_version": mem_version,
            "platform": platform.platform(),
            "python_version": platform.python_version(),
            "git_commit": commit,
            "git_dirty": dirty,
            "query_cases": query_results,
            "prime_cases": prime_results,
            "metrics": metrics,
            "thresholds": thresholds,
            "threshold_results": threshold_results,
            "passed": passed,
        }
        write_report(report, args.report.resolve())
    except (EvaluationError, OSError, subprocess.SubprocessError) as error:
        sys.stderr.write(f"retrieval evaluation failed: {error}\n")
        return 1

    print(
        "retrieval evaluation "
        f"{'passed' if passed else 'failed'}: "
        f"query_pass={metrics['query_case_pass_rate']:.3f}, "
        f"recall={metrics['mean_recall_at_k']:.3f}, "
        f"mrr={metrics['mean_reciprocal_rank']:.3f}, "
        f"prime_pass={metrics['prime_case_pass_rate']:.3f}, "
        f"graph_evidence={metrics['graph_evidence_rate']:.3f}"
    )
    print(f"report: {args.report.resolve()}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
