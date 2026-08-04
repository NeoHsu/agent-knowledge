#!/usr/bin/env python3
"""Scan Git history and the current non-ignored source tree with gitleaks."""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONFIG = ROOT / ".gitleaks.toml"
IGNORE = ROOT / ".gitleaksignore"


def run(command: list[str]) -> None:
    subprocess.run(command, cwd=ROOT, check=True)


def current_source_paths() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    paths: list[Path] = []
    for raw_path in result.stdout.split(b"\0"):
        if not raw_path:
            continue
        relative = Path(os.fsdecode(raw_path))
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"unsafe path returned by git: {relative}")
        paths.append(relative)
    return paths


def scan_current_source() -> None:
    with tempfile.TemporaryDirectory(prefix="mnemark-gitleaks-") as temp:
        snapshot = Path(temp)
        for relative in current_source_paths():
            source = ROOT / relative
            if source.is_symlink() or not source.is_file():
                continue
            destination = snapshot / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
        run(
            [
                "gitleaks",
                "dir",
                "--redact",
                "--no-banner",
                "--config",
                str(CONFIG),
                str(snapshot),
            ]
        )


def main() -> int:
    try:
        run(
            [
                "gitleaks",
                "git",
                "--redact",
                "--no-banner",
                "--config",
                str(CONFIG),
                "--gitleaks-ignore-path",
                str(IGNORE),
                str(ROOT),
            ]
        )
        scan_current_source()
    except FileNotFoundError as error:
        sys.stderr.write(f"secret scan dependency is unavailable: {error.filename}\n")
        return 127
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        sys.stderr.write(f"secret scan failed: {error}\n")
        return 1
    sys.stdout.write(
        "verified Git history and current non-ignored sources with gitleaks\n"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
