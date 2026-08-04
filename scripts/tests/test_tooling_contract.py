from __future__ import annotations

import tomllib
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MISE_CONFIG = ROOT / "mise.toml"
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
DEPENDABOT_CONFIG = ROOT / ".github" / "dependabot.yml"
DIST_CONFIG = ROOT / "dist-workspace.toml"
WORKSPACE_MANIFEST = ROOT / "Cargo.toml"
CLI_MANIFEST = ROOT / "crates" / "mem-cli" / "Cargo.toml"

EXPECTED_TASKS = {
    "audit",
    "check:fast",
    "check:pr",
    "contract:check",
    "contract:update",
    "coverage",
    "deny",
    "deps:check",
    "deps:duplicates",
    "fmt",
    "lint",
    "msrv",
    "python:complexity",
    "python:format",
    "python:lint",
    "sccache:stats",
    "security",
    "security:secrets",
    "size:bloat",
    "size:check",
    "test",
    "test:nextest",
    "tooling:test",
    "workflow:check",
    "workflow:security",
}


class ToolingContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.config = tomllib.loads(MISE_CONFIG.read_text(encoding="utf-8"))
        cls.ci = CI_WORKFLOW.read_text(encoding="utf-8")
        cls.release = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        cls.dependabot = DEPENDABOT_CONFIG.read_text(encoding="utf-8")
        cls.dist = tomllib.loads(DIST_CONFIG.read_text(encoding="utf-8"))
        cls.workspace_manifest = tomllib.loads(
            WORKSPACE_MANIFEST.read_text(encoding="utf-8")
        )
        cls.cli_manifest = tomllib.loads(CLI_MANIFEST.read_text(encoding="utf-8"))

    def test_expected_development_tasks_are_registered(self) -> None:
        tasks = self.config["tasks"]
        self.assertFalse(EXPECTED_TASKS - tasks.keys())
        for dangerous_name in ("publish", "release", "tag"):
            self.assertNotIn(dangerous_name, tasks)

    def test_ci_versions_match_mise_contract(self) -> None:
        tools = self.config["tools"]
        for fragment in (
            f'GITLEAKS_VERSION: "{tools["gitleaks"]}"',
            f"cargo-audit@{tools['cargo:cargo-audit']}",
            f"cargo-deny@{tools['cargo:cargo-deny']}",
            f"cargo-llvm-cov@{tools['cargo:cargo-llvm-cov']}",
            f"cargo-machete@{tools['cargo:cargo-machete']}",
            f"cargo-nextest@{tools['cargo:cargo-nextest']}",
            f'version: "v{tools["sccache"]}"',
            f'version: "{tools["ruff"]}"',
            f"version: {tools['zizmor']}",
        ):
            self.assertIn(fragment, self.ci)
        self.assertIn('CARGO_INCREMENTAL: "0"', self.ci)
        self.assertIn("RUSTC_WRAPPER: sccache", self.ci)
        self.assertIn("--fail-under-lines 86", self.ci)
        self.assertIn(f'python-version: "{tools["python"]}"', self.release)

        rust_version = tools["rust"]["version"]
        self.assertEqual(
            self.workspace_manifest["workspace"]["package"]["rust-version"],
            rust_version,
        )
        self.assertIn(f'["stable","{rust_version}"]', self.ci)
        self.assertIn(f"toolchain: {rust_version}", self.ci)
        self.assertGreaterEqual(self.release.count(f"toolchain: {rust_version}"), 2)

    def test_ci_cancels_obsolete_runs_and_bounds_jobs(self) -> None:
        self.assertIn("concurrency:", self.ci)
        self.assertIn("cancel-in-progress: true", self.ci)
        self.assertGreaterEqual(self.ci.count("timeout-minutes:"), 3)
        self.assertIn("workflow-security:", self.ci)

    def test_local_gate_keeps_security_and_msrv_checks(self) -> None:
        commands = self.config["tasks"]["check:pr"]["run"]
        for command in (
            "cargo machete",
            "python3 scripts/check-secrets.py",
            "ruff check scripts",
            "ruff format --check scripts",
            "cargo nextest run --workspace --locked --status-level all",
            "cargo test --doc --workspace --locked",
            "cargo +1.97.1 check --workspace --all-targets --locked",
            "cargo deny check",
            "cargo audit --deny warnings",
            "scripts/check-workflows.sh",
            "zizmor --persona pedantic --min-severity medium --min-confidence medium .",
        ):
            self.assertIn(command, commands)
        self.assertTrue((ROOT / ".gitleaks.toml").is_file())
        self.assertTrue((ROOT / ".gitleaksignore").is_file())
        scanner = (ROOT / "scripts/check-secrets.py").read_text(encoding="utf-8")
        self.assertIn('"git"', scanner)
        self.assertIn('"dir"', scanner)
        self.assertTrue((ROOT / "ruff.toml").is_file())

    def test_release_runs_native_archive_smoke(self) -> None:
        self.assertIn("verify-platform-artifacts:", self.release)
        self.assertIn(
            "python scripts/verify-release-artifacts.py --platform-only --execute-native",
            self.release,
        )
        self.assertIn(
            "python3 scripts/verify-release-artifacts.py --execute-native", self.release
        )
        self.assertIn("needs.verify-platform-artifacts.result", self.release)
        self.assertIn("python3 scripts/check-binary-size.py", self.ci)
        self.assertIn("python3 scripts/check-binary-size.py", self.release)
        self.assertEqual(self.dist["dist"]["pr-run-mode"], "upload")
        self.assertGreaterEqual(
            self.release.count("if: needs.plan.outputs.publishing == 'true'"), 2
        )

    def test_integration_tests_share_one_harness(self) -> None:
        self.assertFalse(self.cli_manifest["package"]["autotests"])
        targets = {target["name"] for target in self.cli_manifest["test"]}
        self.assertEqual(targets, {"integration", "doc_drift"})

    def test_dependency_updates_keep_major_changes_separate(self) -> None:
        self.assertIn("rust-patch-updates", self.dependabot)
        self.assertIn("rust-minor-updates", self.dependabot)
        self.assertIn("github-actions-non-major", self.dependabot)
        self.assertNotIn("rust-dependencies:", self.dependabot)


if __name__ == "__main__":
    unittest.main()
