from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
CHECKER = ROOT / "scripts" / "check-skill-version.py"
VERSION = "1.2.3"


class SkillVersionCheckerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="mnemark-skill-version-")
        self.root = pathlib.Path(self.temporary.name)
        (self.root / "docs/schemas/fixtures").mkdir(parents=True)
        (self.root / "skills/mnemark/references").mkdir(parents=True)
        (self.root / "crates/mem-cli").mkdir(parents=True)
        (self.root / "Cargo.toml").write_text(
            f'[workspace]\nmembers = []\n\n[workspace.package]\nversion = "{VERSION}"\n',
            encoding="utf-8",
        )
        (self.root / "crates/mem-cli/Cargo.toml").write_text(
            f'[dependencies]\nmem-core = {{ path = "../mem-core", version = "={VERSION}" }}\n',
            encoding="utf-8",
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
        compatibility = {
            "schemaVersion": 1,
            "skillVersion": VERSION,
            "cliVersion": VERSION,
            "compatibility": "exact",
            "releaseTag": f"v{VERSION}",
        }
        encoded = json.dumps(compatibility, indent=2) + "\n"
        (self.root / "skills/mnemark/compatibility.json").write_text(
            encoded, encoding="utf-8"
        )
        (self.root / "docs/schemas/fixtures/skill-compatibility-v1.json").write_text(
            encoded, encoding="utf-8"
        )
        schema_id = (
            "https://github.com/NeoHsu/mnemark/blob/"
            f"v{VERSION}/docs/schemas/skill-compatibility-v1.schema.json"
        )
        (self.root / "docs/schemas/skill-compatibility-v1.schema.json").write_text(
            json.dumps({"$id": schema_id, "type": "object"}, indent=2) + "\n",
            encoding="utf-8",
        )
        source = f"https://github.com/NeoHsu/mnemark/tree/v{VERSION}"
        (self.root / "skills/mnemark/SKILL.md").write_text(
            "\n".join(
                [
                    "---",
                    "name: mnemark",
                    "description: fixture",
                    f"compatibility: Requires mem CLI {VERSION} exactly",
                    "---",
                    f"`mem --json-errors contract --skill-version {VERSION}`",
                    "",
                ]
            ),
            encoding="utf-8",
        )
        for relative in (
            "README.md",
            "docs/getting-started.md",
            "skills/mnemark/references/cli-guide.md",
        ):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source + "\n", encoding="utf-8")

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def run_checker(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(CHECKER), "--repo", str(self.root), *args],
            capture_output=True,
            text=True,
            check=False,
            timeout=10,
        )

    def test_accepts_exact_version_lockstep(self) -> None:
        completed = self.run_checker("--tag", f"v{VERSION}")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertIn(f"exact lockstep at {VERSION}", completed.stdout)

    def test_rejects_stale_skill_and_release_tag(self) -> None:
        compatibility_path = self.root / "skills/mnemark/compatibility.json"
        compatibility = json.loads(compatibility_path.read_text(encoding="utf-8"))
        compatibility["skillVersion"] = "1.2.2"
        compatibility_path.write_text(
            json.dumps(compatibility, indent=2) + "\n", encoding="utf-8"
        )
        completed = self.run_checker("--tag", "v9.9.9")
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("skillVersion", completed.stderr)
        self.assertIn("release tag", completed.stderr)

    def test_rejects_stale_schema_identifier(self) -> None:
        schema = self.root / "docs/schemas/skill-compatibility-v1.schema.json"
        schema.write_text(
            json.dumps({"$id": "https://example.invalid/stale"}) + "\n",
            encoding="utf-8",
        )
        completed = self.run_checker()
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("$id", completed.stderr)

    def test_rejects_non_exact_core_dependency(self) -> None:
        (self.root / "crates/mem-cli/Cargo.toml").write_text(
            f'[dependencies]\nmem-core = {{ path = "../mem-core", version = "{VERSION}" }}\n',
            encoding="utf-8",
        )
        completed = self.run_checker()
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("does not exactly match", completed.stderr)

    def test_rejects_unpinned_install_docs_and_missing_gate(self) -> None:
        (self.root / "README.md").write_text(
            "https://github.com/NeoHsu/mnemark/tree/main\n", encoding="utf-8"
        )
        (self.root / "skills/mnemark/SKILL.md").write_text(
            f"---\nname: mnemark\ndescription: fixture\ncompatibility: Requires mem CLI {VERSION} exactly\n---\n",
            encoding="utf-8",
        )
        completed = self.run_checker()
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("execution gate", completed.stderr)
        self.assertIn("tag-pinned skill source", completed.stderr)


if __name__ == "__main__":
    unittest.main()
