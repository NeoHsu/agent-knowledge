from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check-binary-size.py"


def load_checker():
    spec = importlib.util.spec_from_file_location("check_binary_size", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load binary size checker")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BinarySizeTests(unittest.TestCase):
    def test_accepts_binary_below_budget(self) -> None:
        checker = load_checker()
        with tempfile.TemporaryDirectory(prefix="mnemark-size-") as temp:
            binary = Path(temp) / "mem"
            binary.write_bytes(b"x" * 1024)

            size, limit = checker.verify(binary, 1.0)

            self.assertEqual(size, 1024)
            self.assertEqual(limit, 1024 * 1024)

    def test_rejects_binary_above_budget(self) -> None:
        checker = load_checker()
        with tempfile.TemporaryDirectory(prefix="mnemark-size-") as temp:
            binary = Path(temp) / "mem"
            binary.write_bytes(b"x" * 2048)

            with self.assertRaisesRegex(ValueError, "exceeding"):
                checker.verify(binary, 0.001)


if __name__ == "__main__":
    unittest.main()
