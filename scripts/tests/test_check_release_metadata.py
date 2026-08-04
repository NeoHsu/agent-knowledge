from __future__ import annotations

import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "check-release-metadata.py"
VERSION = "1.2.3"


class ReleaseMetadataCheckerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="mnemark-release-metadata-")
        self.root = pathlib.Path(self.temporary.name)
        (self.root / "docs").mkdir()
        (self.root / "skills/mnemark/references").mkdir(parents=True)
        (self.root / "Cargo.toml").write_text(
            f'[workspace.package]\nversion = "{VERSION}"\n', encoding="utf-8"
        )
        (self.root / "Cargo.lock").write_text(
            "\n".join(
                [
                    "[[package]]",
                    'name = "mem-core"',
                    f'version = "{VERSION}"',
                    "",
                    "[[package]]",
                    'name = "mnemark"',
                    f'version = "{VERSION}"',
                    "",
                ]
            ),
            encoding="utf-8",
        )
        (self.root / "CHANGELOG.md").write_text(
            f"## [{VERSION}] - 2026-01-01\n", encoding="utf-8"
        )
        marker = f"source version `{VERSION}`\n"
        for relative in (
            "README.md",
            "docs/getting-started.md",
            "docs/performance.md",
            "skills/mnemark/references/cli-guide.md",
        ):
            (self.root / relative).write_text(marker, encoding="utf-8")
        self.git("init", "-b", "main")
        disabled_hooks = self.root / ".git/disabled-hooks"
        disabled_hooks.mkdir()
        self.git("config", "core.hooksPath", str(disabled_hooks))
        self.git("config", "user.email", "metadata-check@example.invalid")
        self.git("config", "user.name", "Metadata Check")
        self.git("config", "commit.gpgsign", "false")
        self.git("config", "tag.gpgsign", "false")
        self.git("add", ".")
        self.git("commit", "-m", "fixture")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def git(self, *args: str) -> None:
        completed = subprocess.run(
            ["git", "-C", str(self.root), *args],
            capture_output=True,
            text=True,
            check=False,
            timeout=10,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def run_checker(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(CHECKER), "--root", str(self.root), *args],
            capture_output=True,
            text=True,
            check=False,
            timeout=10,
        )

    def test_accepts_clean_matching_release_metadata(self) -> None:
        completed = self.run_checker("--release-tag", f"v{VERSION}")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn(f"release metadata ok: {VERSION}", completed.stdout)

    def test_accepts_unreleased_target_without_release_tag(self) -> None:
        (self.root / "CHANGELOG.md").write_text(
            f"## [Unreleased — {VERSION}]\n", encoding="utf-8"
        )
        self.git("add", "CHANGELOG.md")
        self.git("commit", "-m", "prepare next version")

        completed = self.run_checker()
        self.assertEqual(completed.returncode, 0, completed.stderr)

    def test_rejects_unreleased_target_for_release_qualification(self) -> None:
        (self.root / "CHANGELOG.md").write_text(
            f"## [Unreleased — {VERSION}]\n", encoding="utf-8"
        )
        self.git("add", "CHANGELOG.md")
        self.git("commit", "-m", "prepare next version")

        completed = self.run_checker("--release-tag", f"v{VERSION}")
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("no dated release heading", completed.stderr)

    def test_rejects_dirty_tree_and_mismatched_tag(self) -> None:
        (self.root / "untracked.txt").write_text("dirty", encoding="utf-8")
        completed = self.run_checker("--release-tag", "v9.9.9")
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("does not match", completed.stderr)
        self.assertIn("requires a clean Git worktree", completed.stderr)

    def test_rejects_reusing_a_version_tag_on_a_new_commit(self) -> None:
        self.git("tag", f"v{VERSION}")
        with (self.root / "README.md").open("a", encoding="utf-8") as handle:
            handle.write("new release work\n")
        self.git("add", "README.md")
        self.git("commit", "-m", "new release work")

        completed = self.run_checker("--release-tag", f"v{VERSION}")

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("bump the version instead of reusing", completed.stderr)

    def test_allows_dirty_development_validation(self) -> None:
        (self.root / "untracked.txt").write_text("dirty", encoding="utf-8")
        completed = self.run_checker("--allow-dirty")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn("dirty development override", completed.stdout)


if __name__ == "__main__":
    unittest.main()
