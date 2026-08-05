from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "evaluate-agent-behavior.py"
FIXTURE = ROOT / "evals" / "agent-behavior-v1.json"
REFERENCE = ROOT / "evals" / "agent-behavior-reference-v1.json"

spec = importlib.util.spec_from_file_location("evaluate_agent_behavior", SCRIPT)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class AgentBehaviorEvaluationTests(unittest.TestCase):
    def fixture(self) -> dict[str, Any]:
        return json.loads(FIXTURE.read_text(encoding="utf-8"))

    def responses(self) -> dict[str, Any]:
        return json.loads(REFERENCE.read_text(encoding="utf-8"))

    def run_checker(
        self, responses: dict[str, Any], *extra: str
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory(prefix="mnemark-agent-eval-") as raw:
            root = pathlib.Path(raw)
            responses_path = root / "responses.json"
            report_path = root / "report.json"
            responses_path.write_text(json.dumps(responses), encoding="utf-8")
            return subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--fixture",
                    str(FIXTURE),
                    "--responses",
                    str(responses_path),
                    "--report",
                    str(report_path),
                    *extra,
                ],
                capture_output=True,
                text=True,
                check=False,
                timeout=10,
            )

    def test_checked_in_fixture_and_reference_pass(self) -> None:
        fixture = self.fixture()
        module.validate_fixture(fixture)
        completed = self.run_checker(self.responses())
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("synthetic_reference", completed.stdout)

    def test_require_live_rejects_synthetic_reference(self) -> None:
        completed = self.run_checker(self.responses(), "--require-live")
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("rejects synthetic traces", completed.stderr)

    def test_stale_fixture_hash_is_rejected(self) -> None:
        responses = self.responses()
        responses["fixture_sha256"] = "0" * 64
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("does not match the evaluated fixture", completed.stderr)

    def test_session_prime_requires_process_read_only_guard(self) -> None:
        responses = self.responses()
        trace = next(
            trace
            for trace in responses["traces"]
            if trace["case_id"] == "session-start-without-hook"
        )
        trace["actions"][1]["argv"] = ["mem", "prime"]
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("agent behavior evaluation failed", completed.stdout)

    def test_missing_store_recall_requires_read_only_guard(self) -> None:
        responses = self.responses()
        trace = next(
            trace
            for trace in responses["traces"]
            if trace["case_id"] == "missing-store-during-recall"
        )
        trace["actions"][1]["argv"] = [
            "mem",
            "query",
            "reply preference",
            "--scope",
            "auto",
        ]
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("agent behavior evaluation failed", completed.stdout)

    def test_conditional_repair_requires_effect_inspection(self) -> None:
        responses = self.responses()
        trace = next(
            trace
            for trace in responses["traces"]
            if trace["case_id"] == "read-only-conditional-repair"
        )
        trace["actions"].pop(1)
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("agent behavior evaluation failed", completed.stdout)

    def test_guarded_conditional_repair_path_is_accepted(self) -> None:
        responses = self.responses()
        trace = next(
            trace
            for trace in responses["traces"]
            if trace["case_id"] == "read-only-conditional-repair"
        )
        trace["actions"][2] = {
            "kind": "command",
            "argv": [
                "mem",
                "--read-only",
                "query",
                "release safety",
                "--repair-index",
            ],
        }
        completed = self.run_checker(responses)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_codebase_graph_stays_outside_mnemark(self) -> None:
        responses = self.responses()
        trace = next(
            trace
            for trace in responses["traces"]
            if trace["case_id"] == "ordinary-codebase-architecture-graph"
        )
        trace["actions"] = [
            {"kind": "command", "argv": ["mem", "graph", "query", "architecture"]}
        ]
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("agent behavior evaluation failed", completed.stdout)

    def test_ordinary_git_sync_does_not_invoke_mem(self) -> None:
        responses = self.responses()
        trace = next(
            trace
            for trace in responses["traces"]
            if trace["case_id"] == "ordinary-git-sync"
        )
        trace["actions"].append(
            {
                "kind": "command",
                "argv": [
                    "mem",
                    "--json-errors",
                    "contract",
                    "--skill-version",
                    "0.10.1",
                ],
            }
        )
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("agent behavior evaluation failed", completed.stdout)

    def test_push_before_approval_fails(self) -> None:
        responses = self.responses()
        trace = next(
            trace
            for trace in responses["traces"]
            if trace["case_id"] == "explicit-sync-push"
        )
        actions = trace["actions"]
        actions[3], actions[4] = actions[4], actions[3]
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("agent behavior evaluation failed", completed.stdout)

    def test_missing_trace_fails_coverage(self) -> None:
        responses = self.responses()
        responses["traces"] = responses["traces"][:-1]
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        case_count = len(self.fixture()["cases"])
        expected_coverage = (case_count - 1) / case_count
        self.assertIn(f"coverage={expected_coverage:.3f}", completed.stdout)

    def test_unknown_action_fields_are_rejected(self) -> None:
        responses = copy.deepcopy(self.responses())
        responses["traces"][0]["actions"][0]["shell"] = "unsafe string"
        completed = self.run_checker(responses)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("unknown fields: shell", completed.stderr)


if __name__ == "__main__":
    unittest.main()
