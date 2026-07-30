from __future__ import annotations

import copy
import importlib.util
import json
import pathlib
import unittest
from typing import Any

ROOT = pathlib.Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "evaluate-retrieval.py"
FIXTURE = ROOT / "evals" / "retrieval-v1.json"

spec = importlib.util.spec_from_file_location("evaluate_retrieval", SCRIPT)
assert spec is not None and spec.loader is not None
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RetrievalEvaluationTests(unittest.TestCase):
    def fixture(self) -> dict[str, Any]:
        return json.loads(FIXTURE.read_text(encoding="utf-8"))

    def test_checked_in_fixture_is_valid(self) -> None:
        fixture = self.fixture()
        module.validate_fixture(fixture)
        self.assertEqual(fixture["schema_version"], 1)
        self.assertGreaterEqual(len(fixture["query_cases"]), 4)
        self.assertGreaterEqual(len(fixture["prime_cases"]), 2)

    def test_duplicate_memory_names_are_rejected(self) -> None:
        fixture = self.fixture()
        fixture["memories"].append(copy.deepcopy(fixture["memories"][0]))
        with self.assertRaisesRegex(module.EvaluationError, "duplicate memory name"):
            module.validate_fixture(fixture)

    def test_invalid_threshold_is_rejected(self) -> None:
        fixture = self.fixture()
        fixture["thresholds"]["mean_recall_at_k_min"] = 1.1
        with self.assertRaisesRegex(module.EvaluationError, "must be between 0 and 1"):
            module.validate_fixture(fixture)

    def test_graph_labels_reads_start_and_expanded_nodes(self) -> None:
        labels = module.graph_labels(
            {
                "start_nodes": [{"label": "start"}],
                "nodes": [{"node": {"label": "expanded"}}],
            }
        )
        self.assertEqual(labels, {"start", "expanded"})


if __name__ == "__main__":
    unittest.main()
