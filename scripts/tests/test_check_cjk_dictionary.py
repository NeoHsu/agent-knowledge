from __future__ import annotations

import hashlib
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "check-cjk-dictionary.py"
SPEC = importlib.util.spec_from_file_location("check_cjk_dictionary", CHECKER)
assert SPEC is not None and SPEC.loader is not None
check_cjk_dictionary = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check_cjk_dictionary
SPEC.loader.exec_module(check_cjk_dictionary)


class CjkDictionaryVerificationTests(unittest.TestCase):
    def test_verifies_matching_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = pathlib.Path(temporary) / check_cjk_dictionary.ARCHIVE_NAME
            archive.write_bytes(b"pinned dictionary fixture")
            expected = hashlib.sha256(archive.read_bytes()).hexdigest()

            self.assertEqual(
                check_cjk_dictionary.verify_archives([archive], expected), [expected]
            )

    def test_rejects_mismatch_and_missing_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            archive = pathlib.Path(temporary) / check_cjk_dictionary.ARCHIVE_NAME
            archive.write_bytes(b"unexpected dictionary fixture")
            with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                check_cjk_dictionary.verify_archives([archive], "0" * 64)
        with self.assertRaisesRegex(ValueError, "no .* build input found"):
            check_cjk_dictionary.verify_archives([])

    def test_discovery_ignores_symlinked_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            real = root / "outside.tar.gz"
            real.write_bytes(b"fixture")
            nested = root / "build" / "crate" / "out"
            nested.mkdir(parents=True)
            link = nested / check_cjk_dictionary.ARCHIVE_NAME
            try:
                link.symlink_to(real)
            except OSError:
                self.skipTest("symlinks unavailable")
            self.assertEqual(check_cjk_dictionary.dictionary_archives(root), [])

    def test_cli_rejects_target_without_build_archive(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            completed = subprocess.run(
                [sys.executable, str(CHECKER), "--target-dir", temporary],
                capture_output=True,
                text=True,
                check=False,
            )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("cannot prove", completed.stderr)


if __name__ == "__main__":
    unittest.main()
