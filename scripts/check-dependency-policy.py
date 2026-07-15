#!/usr/bin/env python3
"""Fail when locked dependencies lack provenance or license metadata."""

from __future__ import annotations

import json
import os
import subprocess
import sys

CRATES_IO_INDEX = "registry+https://github.com/rust-lang/crates.io-index"


def main() -> int:
    cargo = os.environ.get("CARGO", "cargo")
    completed = subprocess.run(
        [cargo, "metadata", "--locked", "--format-version", "1"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        sys.stderr.write(completed.stderr)
        return completed.returncode

    metadata = json.loads(completed.stdout)
    workspace = set(metadata["workspace_members"])
    violations: list[str] = []
    dependencies = 0
    for package in metadata["packages"]:
        if package["id"] in workspace:
            continue
        dependencies += 1
        label = f"{package['name']} {package['version']}"
        source = package.get("source")
        if source != CRATES_IO_INDEX:
            violations.append(f"{label}: unapproved source {source!r}")
        if not package.get("license") and not package.get("license_file"):
            violations.append(f"{label}: missing license metadata")

    if violations:
        sys.stderr.write("dependency policy violations:\n")
        for violation in violations:
            sys.stderr.write(f"- {violation}\n")
        return 1

    sys.stdout.write(
        f"dependency policy ok: {dependencies} locked crates.io packages "
        "with license metadata\n"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
