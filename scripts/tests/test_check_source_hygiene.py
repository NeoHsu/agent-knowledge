from __future__ import annotations

import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "check-source-hygiene.py"


class SourceHygieneCheckerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="mnemark-source-hygiene-")
        self.root = pathlib.Path(self.temporary.name)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_checker(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(CHECKER), "--root", str(self.root)],
            capture_output=True,
            text=True,
            check=False,
            timeout=10,
        )

    def test_accepts_source_checkout_without_runtime_artifacts(self) -> None:
        completed = self.run_checker()

        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("source hygiene ok", completed.stdout)

    def test_rejects_each_runtime_artifact_without_reading_it(self) -> None:
        (self.root / "memory.db").write_bytes(b"not a sqlite database")
        (self.root / "memory.db-wal").write_bytes(b"wal fixture")
        (self.root / "memory.db-shm").write_bytes(b"shm fixture")
        (self.root / ".mem.lock").write_text("", encoding="utf-8")
        (self.root / "index").mkdir()
        (self.root / ".bundle-replace-backup-stale").mkdir()

        completed = self.run_checker()

        self.assertNotEqual(completed.returncode, 0)
        for expected in (
            "memory.db",
            "memory.db-wal",
            "memory.db-shm",
            ".mem.lock",
            "index",
            ".bundle-replace-backup-stale",
        ):
            self.assertIn(expected, completed.stderr)


if __name__ == "__main__":
    unittest.main()
