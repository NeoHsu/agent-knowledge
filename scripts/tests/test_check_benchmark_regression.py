from __future__ import annotations

import copy
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "check-benchmark-regression.py"


def report(median: float = 50.0) -> dict[str, object]:
    return {
        "schema_version": 1,
        "platform": "test-platform",
        "binary_sha256": "a" * 64,
        "benchmark_script_sha256": "b" * 64,
        "mem_version": "mem 1.2.3",
        "git_commit": "c" * 40,
        "git_dirty": False,
        "results": [
            {
                "scale": 100,
                "operations": {
                    "query": {
                        "median": median,
                        "max": median,
                        "min": median,
                        "p95": None,
                        "runs": 2,
                    }
                },
            }
        ],
    }


def v2_report(*, label: str = "candidate", comparison: bool = False) -> dict[str, object]:
    value = report()
    value.update(
        {
            "schema_version": 2,
            "benchmark_protocol_version": 2,
            "benchmark_protocol_sha256": "b" * 64,
            "sample_schedule": {"100": {"query": [["candidate"]]}},
            "statistic_model": {"center": "median"},
            "interactive_runs": 20,
            "maintenance_runs": 5,
            "warmup_runs": 1,
            "run_label": label,
            "comparison_mode": comparison,
            "comparison_peer": None,
        }
    )
    return value


def interleaved_pair() -> tuple[dict[str, object], dict[str, object]]:
    candidate = v2_report(comparison=True)
    baseline = copy.deepcopy(candidate)
    baseline["run_label"] = "baseline"
    baseline["binary_sha256"] = "d" * 64
    baseline["git_commit"] = "e" * 40
    candidate["comparison_peer"] = {
        "label": "baseline",
        "binary_sha256": baseline["binary_sha256"],
        "git_commit": baseline["git_commit"],
    }
    baseline["comparison_peer"] = {
        "label": "candidate",
        "binary_sha256": candidate["binary_sha256"],
        "git_commit": candidate["git_commit"],
    }
    return candidate, baseline


def guardrails(maximum: float = 100.0) -> dict[str, object]:
    return {
        "schema_version": 1,
        "scales": {"100": {"operations": {"query": {"median_ms_max": maximum}}}},
    }


class BenchmarkRegressionCheckerTests(unittest.TestCase):
    def run_checker(
        self,
        candidate: dict[str, object],
        limits: dict[str, object],
        baseline: dict[str, object] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory(prefix="mnemark-benchmark-check-") as raw:
            root = pathlib.Path(raw)
            report_path = root / "report.json"
            guardrails_path = root / "guardrails.json"
            report_path.write_text(json.dumps(candidate), encoding="utf-8")
            guardrails_path.write_text(json.dumps(limits), encoding="utf-8")
            command = [
                sys.executable,
                str(CHECKER),
                "--report",
                str(report_path),
                "--guardrails",
                str(guardrails_path),
                "--require-clean",
            ]
            if baseline is not None:
                baseline_path = root / "baseline.json"
                baseline_path.write_text(json.dumps(baseline), encoding="utf-8")
                command.extend(
                    [
                        "--baseline",
                        str(baseline_path),
                        "--max-regression-percent",
                        "35",
                    ]
                )
            return subprocess.run(command, capture_output=True, text=True, check=False)

    def test_accepts_report_inside_guardrails(self) -> None:
        completed = self.run_checker(report(), guardrails())
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("benchmark guardrails ok", completed.stdout)

    def test_accepts_schema_v2_single_report(self) -> None:
        completed = self.run_checker(v2_report(), guardrails())
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_accepts_interleaved_schema_v2_pair(self) -> None:
        candidate, baseline = interleaved_pair()
        completed = self.run_checker(candidate, guardrails(), baseline=baseline)
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_rejects_non_interleaved_schema_v2_baseline(self) -> None:
        candidate, baseline = interleaved_pair()
        baseline["sample_schedule"] = {"100": {"query": [["baseline"]]}}
        completed = self.run_checker(candidate, guardrails(), baseline=baseline)
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("sample schedules differ", completed.stderr)

    def test_rejects_guardrail_violation_and_dirty_report(self) -> None:
        candidate = report(median=150.0)
        candidate["git_dirty"] = True
        completed = self.run_checker(candidate, guardrails(maximum=100.0))
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("must come from a clean Git worktree", completed.stderr)
        self.assertIn("exceeds guardrail", completed.stderr)

    def test_rejects_same_platform_baseline_regression(self) -> None:
        completed = self.run_checker(
            report(median=140.0),
            guardrails(maximum=1000.0),
            baseline=report(median=100.0),
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("35.0% regression limit", completed.stderr)


if __name__ == "__main__":
    unittest.main()
