#!/usr/bin/env python3
"""Verify the exact CC-CEDICT source archive used by Lindera's build script."""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import sys

ARCHIVE_NAME = "CC-CEDICT-MeCab-0.1.0-20200409.tar.gz"
EXPECTED_SHA256 = "ed3cf9e3ec8a80647f0ec783dc09dad43b8ccad2e994f5eab6ff13a41d0916c8"


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def dictionary_archives(target_dir: pathlib.Path) -> list[pathlib.Path]:
    return sorted(
        path
        for path in target_dir.rglob(ARCHIVE_NAME)
        if path.is_file() and not path.is_symlink()
    )


def verify_archives(
    archives: list[pathlib.Path], expected_sha256: str = EXPECTED_SHA256
) -> list[str]:
    if not archives:
        raise ValueError(
            f"no {ARCHIVE_NAME} build input found; the release build cannot prove "
            "which CJK dictionary source was embedded"
        )
    hashes: list[str] = []
    for archive in archives:
        actual = sha256(archive)
        if actual != expected_sha256:
            raise ValueError(
                f"CJK dictionary SHA-256 mismatch for {archive}: "
                f"expected {expected_sha256}, got {actual}"
            )
        hashes.append(actual)
    return hashes


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--target-dir",
        type=pathlib.Path,
        default=pathlib.Path("target"),
        help="Cargo target directory containing Lindera build outputs",
    )
    parser.add_argument(
        "--archive",
        action="append",
        type=pathlib.Path,
        default=[],
        help="verify an explicit archive instead of discovering Cargo outputs",
    )
    args = parser.parse_args()

    archives = args.archive or dictionary_archives(args.target_dir)
    try:
        hashes = verify_archives(archives)
    except (OSError, ValueError) as error:
        sys.stderr.write(f"CJK dictionary verification failed: {error}\n")
        return 1
    unique_hashes = sorted(set(hashes))
    print(
        f"CJK dictionary source verified: {len(archives)} archive(s), "
        f"SHA-256 {unique_hashes[0]}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
