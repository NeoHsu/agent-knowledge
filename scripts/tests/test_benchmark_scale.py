from __future__ import annotations

import csv
import importlib.util
import io
import pathlib
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "scripts" / "benchmark_scale.py"
SPEC = importlib.util.spec_from_file_location("benchmark_scale", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
benchmark_scale = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = benchmark_scale
SPEC.loader.exec_module(benchmark_scale)


class BenchmarkScaleTests(unittest.TestCase):
    def test_summary_reports_robust_dispersion_and_deterministic_interval(self) -> None:
        values = [10.0, 11.0, 12.0, 13.0, 100.0]
        first = benchmark_scale.summarize(values, include_p95=False, bootstrap_seed=42)
        second = benchmark_scale.summarize(values, include_p95=False, bootstrap_seed=42)

        self.assertEqual(first, second)
        self.assertEqual(first["median"], 12.0)
        self.assertEqual(first["mad"], 1.0)
        self.assertIsNone(first["p95"])
        self.assertLessEqual(first["median_ci95_low"], first["median"])
        self.assertGreaterEqual(first["median_ci95_high"], first["median"])

    def test_p95_requires_twenty_samples(self) -> None:
        nineteen = benchmark_scale.summarize(
            [float(value) for value in range(19)],
            include_p95=True,
            bootstrap_seed=1,
        )
        twenty = benchmark_scale.summarize(
            [float(value) for value in range(20)],
            include_p95=True,
            bootstrap_seed=1,
        )

        self.assertIsNone(nineteen["p95"])
        self.assertEqual(twenty["p95"], 18.0)

    def test_protocol_hash_covers_every_framed_input(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            for relative in benchmark_scale.PROTOCOL_FILES:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(relative, encoding="utf-8")
            original = benchmark_scale.protocol_sha256(root)

            changed = root / benchmark_scale.PROTOCOL_FILES[-1]
            changed.write_text("changed", encoding="utf-8")
            self.assertNotEqual(original, benchmark_scale.protocol_sha256(root))

    def test_csv_includes_variance_and_confidence_columns(self) -> None:
        operation = {
            "runs": 5,
            "median": 12.0,
            "p95": None,
            "mad": 1.0,
            "median_ci95_low": 10.0,
            "median_ci95_high": 14.0,
            "min": 9.0,
            "max": 15.0,
            "peak_rss_mib": 20.0,
            "input_bytes": 1024 * 1024,
            "output_bytes": 2 * 1024 * 1024,
        }
        output = io.StringIO()
        benchmark_scale.write_csv(
            [{"scale": 100, "operations": {"query": operation}}], output
        )
        rows = list(csv.DictReader(io.StringIO(output.getvalue())))

        self.assertEqual(rows[0]["mad_ms"], "1.00")
        self.assertEqual(rows[0]["median_ci95_low_ms"], "10.00")
        self.assertEqual(rows[0]["median_ci95_high_ms"], "14.00")

    def test_peak_rss_parses_linux_and_macos_time_formats(self) -> None:
        linux = "Maximum resident set size (kbytes): 2048"
        macos = "1048576  maximum resident set size"
        self.assertEqual(benchmark_scale.parse_peak_rss(linux), 2.0)
        self.assertEqual(benchmark_scale.parse_peak_rss(macos), 1.0)


if __name__ == "__main__":
    unittest.main()
