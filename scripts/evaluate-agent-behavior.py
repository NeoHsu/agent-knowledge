#!/usr/bin/env python3
"""Score captured agent action traces against mnemark's fail-closed policy cases."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import sys
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_FIXTURE = ROOT / "evals" / "agent-behavior-v1.json"
DEFAULT_REPORT = ROOT / "target" / "agent-behavior-eval.json"
MATCHER_FIELDS = {
    "kind",
    "argv_prefix",
    "argv_contains",
    "phase",
    "decision",
    "approval",
}
ACTION_FIELDS = {"kind", "argv", "phase", "decision", "approval", "outcome"}
ASSERTION_TYPES = {
    "required",
    "forbidden",
    "ordered",
    "first_command",
    "approval_before",
    "exactly_one_of",
}


class EvaluationError(ValueError):
    """Raised when an eval fixture or captured trace is malformed."""


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


def require_string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise EvaluationError(f"{label} must be a non-empty string")
    return value


def require_list(value: Any, label: str) -> list[Any]:
    if not isinstance(value, list):
        raise EvaluationError(f"{label} must be an array")
    return value


def validate_string_array(value: Any, label: str) -> list[str]:
    items = require_list(value, label)
    if not items or not all(isinstance(item, str) and item for item in items):
        raise EvaluationError(f"{label} must contain non-empty strings")
    return items


def validate_matcher(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvaluationError(f"{label} must be an object")
    unknown = set(value) - MATCHER_FIELDS
    if unknown:
        raise EvaluationError(
            f"{label} has unknown fields: {', '.join(sorted(unknown))}"
        )
    kind = require_string(value.get("kind"), f"{label}.kind")
    if kind not in {"command", "approval", "decision"}:
        raise EvaluationError(f"{label}.kind must be command, approval, or decision")
    if "argv_prefix" in value:
        validate_string_array(value["argv_prefix"], f"{label}.argv_prefix")
    if "argv_contains" in value:
        validate_string_array(value["argv_contains"], f"{label}.argv_contains")
    for field in ("phase", "decision", "approval"):
        if field in value:
            require_string(value[field], f"{label}.{field}")
    if kind == "command" and not ({"argv_prefix", "argv_contains"} & set(value)):
        raise EvaluationError(
            f"{label} command matcher needs argv_prefix or argv_contains"
        )
    if kind == "approval" and "approval" not in value:
        raise EvaluationError(f"{label} approval matcher needs approval")
    if kind == "decision" and "decision" not in value:
        raise EvaluationError(f"{label} decision matcher needs decision")
    return value


def validate_fixture(fixture: dict[str, Any]) -> None:
    if fixture.get("schema_version") != 1:
        raise EvaluationError("agent behavior fixture schema_version must be 1")
    require_string(fixture.get("name"), "fixture.name")
    cases = require_list(fixture.get("cases"), "fixture.cases")
    if not cases:
        raise EvaluationError("fixture.cases must not be empty")
    case_ids: set[str] = set()
    for case_index, case in enumerate(cases):
        if not isinstance(case, dict):
            raise EvaluationError(f"cases[{case_index}] must be an object")
        case_id = require_string(case.get("id"), f"cases[{case_index}].id")
        if case_id in case_ids:
            raise EvaluationError(f"duplicate case id: {case_id}")
        case_ids.add(case_id)
        require_string(case.get("prompt"), f"cases[{case_index}].prompt")
        assertions = require_list(
            case.get("assertions"), f"cases[{case_index}].assertions"
        )
        if not assertions:
            raise EvaluationError(f"case {case_id} must contain assertions")
        assertion_ids: set[str] = set()
        for assertion_index, assertion in enumerate(assertions):
            label = f"case {case_id} assertion {assertion_index}"
            if not isinstance(assertion, dict):
                raise EvaluationError(f"{label} must be an object")
            assertion_id = require_string(assertion.get("id"), f"{label}.id")
            if assertion_id in assertion_ids:
                raise EvaluationError(
                    f"duplicate assertion id in {case_id}: {assertion_id}"
                )
            assertion_ids.add(assertion_id)
            assertion_type = require_string(assertion.get("type"), f"{label}.type")
            if assertion_type not in ASSERTION_TYPES:
                raise EvaluationError(f"unsupported assertion type: {assertion_type}")
            if assertion_type in {"required", "first_command"}:
                validate_matcher(assertion.get("matcher"), f"{label}.matcher")
            elif assertion_type in {"forbidden", "ordered", "exactly_one_of"}:
                matchers = require_list(assertion.get("matchers"), f"{label}.matchers")
                if not matchers:
                    raise EvaluationError(f"{label}.matchers must not be empty")
                for matcher_index, matcher in enumerate(matchers):
                    validate_matcher(matcher, f"{label}.matchers[{matcher_index}]")
            elif assertion_type == "approval_before":
                validate_matcher(assertion.get("approval"), f"{label}.approval")
                validate_matcher(assertion.get("command"), f"{label}.command")


def validate_subject(value: Any, require_live: bool) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvaluationError("responses.subject must be an object")
    kind = require_string(value.get("kind"), "responses.subject.kind")
    if kind not in {"live_agent", "synthetic"}:
        raise EvaluationError("responses.subject.kind must be live_agent or synthetic")
    if require_live and kind != "live_agent":
        raise EvaluationError("--require-live rejects synthetic traces")
    required = ["platform", "adapter"]
    if kind == "live_agent":
        required.extend(["model", "skill_version", "cli_version"])
    for field in required:
        require_string(value.get(field), f"responses.subject.{field}")
    return value


def validate_action(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvaluationError(f"{label} must be an object")
    unknown = set(value) - ACTION_FIELDS
    if unknown:
        raise EvaluationError(
            f"{label} has unknown fields: {', '.join(sorted(unknown))}"
        )
    kind = require_string(value.get("kind"), f"{label}.kind")
    if kind == "command":
        validate_string_array(value.get("argv"), f"{label}.argv")
    elif kind == "approval":
        require_string(value.get("approval"), f"{label}.approval")
    elif kind == "decision":
        require_string(value.get("decision"), f"{label}.decision")
    else:
        raise EvaluationError(f"{label}.kind must be command, approval, or decision")
    for field in ("phase", "outcome"):
        if field in value:
            require_string(value[field], f"{label}.{field}")
    return value


def validate_responses(
    responses: dict[str, Any],
    fixture_case_ids: set[str],
    expected_fixture_sha256: str,
    require_live: bool,
) -> tuple[dict[str, Any], dict[str, list[dict[str, Any]]]]:
    if responses.get("schema_version") != 1:
        raise EvaluationError("responses schema_version must be 1")
    fixture_sha256 = require_string(
        responses.get("fixture_sha256"), "responses.fixture_sha256"
    )
    if fixture_sha256 != expected_fixture_sha256:
        raise EvaluationError(
            "responses fixture_sha256 does not match the evaluated fixture"
        )
    subject = validate_subject(responses.get("subject"), require_live)
    traces = require_list(responses.get("traces"), "responses.traces")
    by_case: dict[str, list[dict[str, Any]]] = {}
    for trace_index, trace in enumerate(traces):
        if not isinstance(trace, dict):
            raise EvaluationError(f"traces[{trace_index}] must be an object")
        case_id = require_string(trace.get("case_id"), f"traces[{trace_index}].case_id")
        if case_id not in fixture_case_ids:
            raise EvaluationError(f"trace references unknown case: {case_id}")
        if case_id in by_case:
            raise EvaluationError(f"duplicate trace for case: {case_id}")
        actions = require_list(trace.get("actions"), f"trace {case_id}.actions")
        by_case[case_id] = [
            validate_action(action, f"trace {case_id}.actions[{index}]")
            for index, action in enumerate(actions)
        ]
    return subject, by_case


def matches(action: dict[str, Any], matcher: dict[str, Any]) -> bool:
    if action.get("kind") != matcher.get("kind"):
        return False
    for field in ("phase", "decision", "approval"):
        if field in matcher and action.get(field) != matcher[field]:
            return False
    argv = action.get("argv")
    if "argv_prefix" in matcher:
        prefix = matcher["argv_prefix"]
        if not isinstance(argv, list) or argv[: len(prefix)] != prefix:
            return False
    if "argv_contains" in matcher:
        required = matcher["argv_contains"]
        if not isinstance(argv, list) or not all(item in argv for item in required):
            return False
    return True


def matching_indices(
    actions: list[dict[str, Any]], matcher: dict[str, Any]
) -> list[int]:
    return [index for index, action in enumerate(actions) if matches(action, matcher)]


def evaluate_assertion(
    assertion: dict[str, Any], actions: list[dict[str, Any]]
) -> tuple[bool, str]:
    assertion_type = assertion["type"]
    if assertion_type == "required":
        indices = matching_indices(actions, assertion["matcher"])
        return bool(indices), f"matched action indices {indices}"
    if assertion_type == "forbidden":
        found = sorted(
            {
                index
                for matcher in assertion["matchers"]
                for index in matching_indices(actions, matcher)
            }
        )
        return not found, f"forbidden action indices {found}"
    if assertion_type == "first_command":
        first = next(
            (
                index
                for index, action in enumerate(actions)
                if action.get("kind") == "command"
            ),
            None,
        )
        passed = first is not None and matches(actions[first], assertion["matcher"])
        return passed, f"first command index {first}"
    if assertion_type == "ordered":
        selected: list[int] = []
        cursor = 0
        for matcher in assertion["matchers"]:
            found = next(
                (
                    index
                    for index in range(cursor, len(actions))
                    if matches(actions[index], matcher)
                ),
                None,
            )
            if found is None:
                return False, f"matched ordered prefix {selected}"
            selected.append(found)
            cursor = found + 1
        return True, f"matched action indices {selected}"
    if assertion_type == "approval_before":
        approvals = matching_indices(actions, assertion["approval"])
        commands = matching_indices(actions, assertion["command"])
        passed = bool(approvals and commands and min(approvals) < min(commands))
        return passed, f"approval indices {approvals}; command indices {commands}"
    if assertion_type == "exactly_one_of":
        matches_found = [
            (matcher_index, action_index)
            for matcher_index, matcher in enumerate(assertion["matchers"])
            for action_index in matching_indices(actions, matcher)
        ]
        return len(matches_found) == 1, f"matcher/action pairs {matches_found}"
    raise EvaluationError(f"unsupported assertion type: {assertion_type}")


def evaluate(
    fixture: dict[str, Any], traces: dict[str, list[dict[str, Any]]]
) -> tuple[list[dict[str, Any]], dict[str, float], bool]:
    case_results: list[dict[str, Any]] = []
    assertion_total = 0
    assertion_passed = 0
    for case in fixture["cases"]:
        case_id = case["id"]
        actions = traces.get(case_id)
        if actions is None:
            results = [
                {
                    "id": assertion["id"],
                    "type": assertion["type"],
                    "passed": False,
                    "evidence": "trace missing",
                }
                for assertion in case["assertions"]
            ]
        else:
            results = []
            for assertion in case["assertions"]:
                passed, evidence = evaluate_assertion(assertion, actions)
                results.append(
                    {
                        "id": assertion["id"],
                        "type": assertion["type"],
                        "passed": passed,
                        "evidence": evidence,
                    }
                )
        assertion_total += len(results)
        assertion_passed += sum(result["passed"] for result in results)
        case_results.append(
            {
                "id": case_id,
                "trace_present": actions is not None,
                "action_count": 0 if actions is None else len(actions),
                "assertions": results,
                "passed": all(result["passed"] for result in results),
            }
        )
    metrics = {
        "case_pass_rate": sum(case["passed"] for case in case_results)
        / len(case_results),
        "assertion_pass_rate": assertion_passed / assertion_total,
        "trace_coverage": sum(case["trace_present"] for case in case_results)
        / len(case_results),
    }
    metrics = {key: round(value, 6) for key, value in metrics.items()}
    passed = all(value == 1.0 for value in metrics.values())
    return case_results, metrics, passed


def write_report(report: dict[str, Any], destination: pathlib.Path) -> None:
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    temporary.replace(destination)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--responses", type=pathlib.Path, required=True)
    parser.add_argument("--fixture", type=pathlib.Path, default=DEFAULT_FIXTURE)
    parser.add_argument("--report", type=pathlib.Path, default=DEFAULT_REPORT)
    parser.add_argument(
        "--require-live",
        action="store_true",
        help="reject synthetic reference traces when retaining real agent evidence",
    )
    args = parser.parse_args()

    fixture_path = args.fixture.resolve()
    responses_path = args.responses.resolve()
    try:
        fixture = load_object(fixture_path, "agent behavior fixture")
        validate_fixture(fixture)
        case_ids = {case["id"] for case in fixture["cases"]}
        fixture_sha256 = sha256(fixture_path)
        responses = load_object(responses_path, "agent responses")
        subject, traces = validate_responses(
            responses, case_ids, fixture_sha256, args.require_live
        )
        case_results, metrics, passed = evaluate(fixture, traces)
        report = {
            "schema_version": 1,
            "fixture": fixture["name"],
            "fixture_sha256": fixture_sha256,
            "responses_sha256": sha256(responses_path),
            "subject": subject,
            "evidence_kind": (
                "live_agent_trace"
                if subject["kind"] == "live_agent"
                else "synthetic_reference"
            ),
            "cases": case_results,
            "metrics": metrics,
            "passed": passed,
        }
        write_report(report, args.report.resolve())
    except (EvaluationError, OSError) as error:
        sys.stderr.write(f"agent behavior evaluation failed: {error}\n")
        return 1

    print(
        "agent behavior evaluation "
        f"{'passed' if passed else 'failed'} ({report['evidence_kind']}): "
        f"cases={metrics['case_pass_rate']:.3f}, "
        f"assertions={metrics['assertion_pass_rate']:.3f}, "
        f"coverage={metrics['trace_coverage']:.3f}"
    )
    print(f"report: {args.report.resolve()}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
