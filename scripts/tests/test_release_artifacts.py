from __future__ import annotations

import hashlib
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
HARDENER = ROOT / "scripts" / "harden-installers.py"
VERIFIER = ROOT / "scripts" / "verify-release-artifacts.py"


def load_verifier():
    spec = importlib.util.spec_from_file_location("verify_release_artifacts", VERIFIER)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load verifier")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseArtifactToolTests(unittest.TestCase):
    def test_hardener_makes_both_installers_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="mnemark-installer-hardening-") as temp:
            root = pathlib.Path(temp)
            distrib = root / "target/distrib"
            distrib.mkdir(parents=True)
            powershell = distrib / "mnemark-installer.ps1"
            powershell.write_text(
                "before\n  $wc.downloadFile($url, $dir_path)\nafter\n",
                encoding="utf-8",
            )
            shell = distrib / "mnemark-installer.sh"
            shell.write_text(
                """before
    if [ -z "$_checksum_value" ]; then
        return 0
    fi
    case "$_checksum_style" in
        sha256)
            if ! check_cmd sha256sum; then
                say "skipping sha256 checksum verification (it requires the 'sha256sum' command)"
                return 0
            fi
            _calculated_checksum="$(sha256sum -b "$_file" | awk '{printf $1}')"
            ;;
after
""",
                encoding="utf-8",
            )
            completed = subprocess.run(
                [sys.executable, str(HARDENER)],
                cwd=root,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(completed.returncode, 0, completed.stderr)
            hardened_powershell = powershell.read_text(encoding="utf-8")
            self.assertIn("Get-FileHash -Algorithm SHA256", hardened_powershell)
            self.assertIn('$wc.downloadFile("$url.sha256"', hardened_powershell)
            hardened_shell = shell.read_text(encoding="utf-8")
            self.assertIn("shasum -a 256", hardened_shell)
            self.assertIn("openssl dgst -sha256", hardened_shell)
            self.assertIn("missing checksum for downloaded archive", hardened_shell)
            self.assertNotIn("skipping sha256 checksum verification", hardened_shell)

    def test_verifier_checks_sidecars_manifest_and_sbom(self) -> None:
        verifier = load_verifier()
        with tempfile.TemporaryDirectory(prefix="mnemark-artifact-verifier-") as temp:
            root = pathlib.Path(temp).resolve()
            artifact = root / "artifact.txt"
            artifact.write_text("artifact\n", encoding="utf-8")
            digest = hashlib.sha256(artifact.read_bytes()).hexdigest()
            artifact.with_name("artifact.txt.sha256").write_text(
                f"{digest}  artifact.txt\n", encoding="utf-8"
            )
            verifier.verify_sidecar(artifact)

            (root / "sha256.sum").write_text(
                f"{digest}  artifact.txt\n", encoding="utf-8"
            )
            verifier.verify_checksum_manifest(root)

            sbom = root / "mnemark.cdx.xml"
            sbom.write_text(
                '<?xml version="1.0"?><bom xmlns="http://cyclonedx.org/schema/bom/1.5"><components><component type="application"><hashes><hash alg="SHA-256">abc</hash></hashes></component></components></bom>',
                encoding="utf-8",
            )
            sbom_digest = hashlib.sha256(sbom.read_bytes()).hexdigest()
            sbom.with_name("mnemark.cdx.xml.sha256").write_text(
                f"{sbom_digest}  mnemark.cdx.xml\n", encoding="utf-8"
            )
            verifier.verify_sbom(sbom)

    def test_verifier_rejects_archive_path_escape(self) -> None:
        verifier = load_verifier()
        with self.assertRaisesRegex(ValueError, "unsafe path"):
            verifier.safe_names(["../memory.db"], pathlib.Path("bundle.tar.xz"))

    def test_verifier_rejects_checksum_manifest_path_escape(self) -> None:
        verifier = load_verifier()
        with tempfile.TemporaryDirectory(prefix="mnemark-checksum-escape-") as temp:
            root = pathlib.Path(temp)
            directory = root / "distrib"
            directory.mkdir()
            outside = root / "outside.txt"
            outside.write_text("outside\n", encoding="utf-8")
            checksum = hashlib.sha256(outside.read_bytes()).hexdigest()
            (directory / "sha256.sum").write_text(
                f"{checksum}  ../outside.txt\n", encoding="utf-8"
            )
            with self.assertRaisesRegex(ValueError, "unsafe path"):
                verifier.verify_checksum_manifest(directory)


if __name__ == "__main__":
    unittest.main()
