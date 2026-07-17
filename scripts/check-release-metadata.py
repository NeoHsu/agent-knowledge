#!/usr/bin/env python3
"""Validate release version metadata and clean-worktree requirements."""

from __future__ import annotations

import argparse
import pathlib
import re
import subprocess
import sys

VERSIONED_DOCS = (
    "README.md",
    "docs/getting-started.md",
    "docs/performance.md",
    "skills/mnemark/references/cli-guide.md",
)
WORKSPACE_PACKAGES = ("mnemark", "mem-core")


def read_text(path: pathlib.Path, errors: list[str]) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"cannot read {path}: {error}")
        return ""


def command_output(command: list[str], errors: list[str]) -> str:
    try:
        completed = subprocess.run(command, capture_output=True, text=True, check=False)
    except OSError as error:
        errors.append(f"cannot run {' '.join(command)}: {error}")
        return ""
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip()
        errors.append(f"command failed ({' '.join(command)}): {detail}")
        return ""
    return completed.stdout


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parents[1],
    )
    parser.add_argument("--release-tag", default="")
    parser.add_argument("--allow-dirty", action="store_true")
    args = parser.parse_args()

    root = args.root.resolve()
    errors: list[str] = []
    cargo_toml = read_text(root / "Cargo.toml", errors)
    version_match = re.search(
        r'(?ms)^\[workspace\.package\].*?^version = "([^"]+)"', cargo_toml
    )
    if version_match is None:
        errors.append("Cargo.toml has no workspace.package version")
        version = ""
    else:
        version = version_match.group(1)

    lock = read_text(root / "Cargo.lock", errors)
    for package in WORKSPACE_PACKAGES:
        pattern = rf'(?ms)^\[\[package\]\]\nname = "{re.escape(package)}"\nversion = "([^"]+)"'
        match = re.search(pattern, lock)
        if match is None:
            errors.append(f"Cargo.lock is missing workspace package {package}")
        elif match.group(1) != version:
            errors.append(f"Cargo.lock {package} version {match.group(1)} != {version}")

    changelog = read_text(root / "CHANGELOG.md", errors)
    if version and f"## [{version}]" not in changelog:
        errors.append(f"CHANGELOG.md has no release heading for {version}")

    expected = f"source version `{version}`"
    for relative in VERSIONED_DOCS:
        if expected not in read_text(root / relative, errors):
            errors.append(f"{relative} does not identify {expected}")

    tag = args.release_tag.strip()
    if tag and tag not in (version, f"v{version}"):
        errors.append(f"release tag {tag!r} does not match {version} or v{version}")

    head = command_output(["git", "-C", str(root), "rev-parse", "HEAD"], errors).strip()
    candidate_tags = {tag} if tag else set()
    for candidate in sorted(candidate_tags):
        listed = command_output(
            ["git", "-C", str(root), "tag", "--list", candidate], errors
        ).splitlines()
        if candidate not in listed:
            continue
        tagged_commit = command_output(
            ["git", "-C", str(root), "rev-list", "-n", "1", candidate], errors
        ).strip()
        if head and tagged_commit and tagged_commit != head:
            errors.append(
                f"existing tag {candidate} points to {tagged_commit}, not HEAD {head}; "
                "bump the version instead of reusing a published tag"
            )

    status = command_output(
        ["git", "-C", str(root), "status", "--porcelain", "--untracked-files=all"],
        errors,
    )
    if status.strip() and not args.allow_dirty:
        errors.append(
            "release qualification requires a clean Git worktree; "
            "use --allow-dirty only for development validation"
        )

    if errors:
        sys.stderr.write("release metadata check failed:\n")
        for error in errors:
            sys.stderr.write(f"- {error}\n")
        return 1

    qualifier = " (dirty development override)" if status.strip() else ""
    print(f"release metadata ok: {version}{qualifier}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
