#!/usr/bin/env python3
"""Verify exact version lockstep between mem, the bundled skill, and release docs."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any

import tomllib

SEMVER = re.compile(
    r"^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$"
)
VERSIONED_SKILL_DOCS = (
    Path("README.md"),
    Path("docs/getting-started.md"),
    Path("skills/mnemark/references/cli-guide.md"),
)
COMPATIBILITY_PATH = Path("skills/mnemark/compatibility.json")
COMPATIBILITY_FIXTURE = Path("docs/schemas/fixtures/skill-compatibility-v1.json")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root",
    )
    parser.add_argument("--tag", help="release tag to verify, for example v0.9.0")
    return parser.parse_args()


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except OSError as error:
        raise ValueError(f"failed to read {path}: {error}") from error


def repo_file(repo: Path, relative: Path | str) -> Path:
    root = repo.resolve(strict=True)
    candidate = (root / relative).resolve(strict=True)
    if not candidate.is_relative_to(root) or not candidate.is_file():
        raise ValueError(f"repository file is missing or escapes the root: {relative}")
    return candidate


def repo_directory(repo: Path, relative: Path | str) -> Path:
    root = repo.resolve(strict=True)
    candidate = (root / relative).resolve(strict=True)
    if not candidate.is_relative_to(root) or not candidate.is_dir():
        raise ValueError(
            f"repository directory is missing or escapes the root: {relative}"
        )
    return candidate


def load_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(read_text(path))
    except json.JSONDecodeError as error:
        raise ValueError(f"failed to parse {path}: {error}") from error
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def workspace_version(path: Path) -> str:
    try:
        document = tomllib.loads(read_text(path))
        version = document["workspace"]["package"]["version"]
    except (KeyError, TypeError, tomllib.TOMLDecodeError) as error:
        raise ValueError(f"{path} has no workspace.package.version: {error}") from error
    if not isinstance(version, str):
        raise ValueError(f"{path} workspace.package.version must be a string")
    return version


def locked_versions(path: Path, package_name: str) -> set[str]:
    document = tomllib.loads(read_text(path))
    return {
        package["version"]
        for package in document.get("package", [])
        if package.get("name") == package_name
        and isinstance(package.get("version"), str)
    }


def core_dependency_version(path: Path) -> str:
    try:
        document = tomllib.loads(read_text(path))
        dependency = document["dependencies"]["mem-core"]
        version = dependency["version"]
    except (KeyError, TypeError, tomllib.TOMLDecodeError) as error:
        raise ValueError(
            f"{path} has no mem-core dependency version: {error}"
        ) from error
    if not isinstance(version, str):
        raise ValueError(f"{path} mem-core dependency version must be a string")
    return version


def verify(repo: Path, release_tag: str | None) -> tuple[str, list[str]]:
    version = workspace_version(repo_file(repo, "Cargo.toml"))
    manifest = load_json(repo_file(repo, COMPATIBILITY_PATH))
    fixture = load_json(repo_file(repo, COMPATIBILITY_FIXTURE))
    skill = read_text(repo_file(repo, "skills/mnemark/SKILL.md"))
    errors: list[str] = []
    expected_tag = f"v{version}"

    if not SEMVER.fullmatch(version):
        errors.append(f"workspace version is not SemVer: {version!r}")
    for package_name in ("mnemark", "mem-core"):
        versions = locked_versions(repo_file(repo, "Cargo.lock"), package_name)
        if versions != {version}:
            errors.append(
                f"Cargo.lock {package_name} versions {sorted(versions)!r} do not match {version!r}"
            )
    core_requirement = core_dependency_version(
        repo_file(repo, "crates/mem-cli/Cargo.toml")
    )
    if core_requirement != f"={version}":
        errors.append(
            "crates/mem-cli/Cargo.toml mem-core requirement "
            f"{core_requirement!r} does not exactly match '={version}'"
        )

    expected_values: dict[str, Any] = {
        "schemaVersion": 1,
        "skillVersion": version,
        "cliVersion": version,
        "compatibility": "exact",
        "releaseTag": expected_tag,
    }
    for key, expected in expected_values.items():
        if manifest.get(key) != expected:
            errors.append(
                f"{COMPATIBILITY_PATH} {key} {manifest.get(key)!r} does not match {expected!r}"
            )
        if fixture.get(key) != expected:
            errors.append(
                f"{COMPATIBILITY_FIXTURE} {key} {fixture.get(key)!r} does not match {expected!r}"
            )

    compatibility = re.search(
        r"^compatibility:\s*Requires mem CLI ([^ ]+) exactly$", skill, re.MULTILINE
    )
    if compatibility is None:
        errors.append("SKILL.md has no exact mem CLI compatibility declaration")
    elif compatibility.group(1) != version:
        errors.append(
            f"SKILL.md compatibility {compatibility.group(1)!r} does not match {version!r}"
        )

    gate = f"mem --json-errors contract --skill-version {version}"
    if gate not in skill:
        errors.append(f"SKILL.md execution gate is missing `{gate}`")

    source = f"https://github.com/NeoHsu/mnemark/tree/{expected_tag}"
    for relative in VERSIONED_SKILL_DOCS:
        if source not in read_text(repo_file(repo, relative)):
            errors.append(f"{relative} is missing tag-pinned skill source {source}")

    schema_directory = repo_directory(repo, "docs/schemas")
    schema_paths = sorted(schema_directory.glob("*.schema.json"))
    if not schema_paths:
        errors.append("docs/schemas has no published JSON Schemas")
    for schema_path in schema_paths:
        schema = load_json(repo_file(repo, Path("docs/schemas") / schema_path.name))
        expected_id = (
            "https://github.com/NeoHsu/mnemark/blob/"
            f"{expected_tag}/docs/schemas/{schema_path.name}"
        )
        if schema.get("$id") != expected_id:
            errors.append(
                f"docs/schemas/{schema_path.name} $id {schema.get('$id')!r} "
                f"does not match {expected_id!r}"
            )

    if release_tag is not None and release_tag not in (version, expected_tag):
        errors.append(
            f"release tag {release_tag!r} does not match {version!r} or {expected_tag!r}"
        )
    return version, errors


def main() -> int:
    args = parse_args()
    try:
        repo = args.repo.resolve(strict=True)
        version, errors = verify(repo, args.tag)
    except (OSError, ValueError, tomllib.TOMLDecodeError) as error:
        sys.stderr.write(f"skill version check failed: {error}\n")
        return 1

    if errors:
        for error in errors:
            sys.stderr.write(f"skill version check failed: {error}\n")
        return 1
    sys.stdout.write(
        f"verified mem CLI and mnemark skill exact lockstep at {version}\n"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
