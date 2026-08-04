#!/usr/bin/env python3
"""Reject runtime memory artifacts from the mnemark source checkout."""

from __future__ import annotations

import argparse
import pathlib
import sys

FORBIDDEN_PATHS = (
    "memory.db",
    "memory.db-wal",
    "memory.db-shm",
    ".mem.lock",
    "index",
)
FORBIDDEN_GLOBS = (".bundle-replace-backup-*",)


def runtime_artifacts(root: pathlib.Path) -> list[pathlib.Path]:
    found = [
        root / relative for relative in FORBIDDEN_PATHS if (root / relative).exists()
    ]
    for pattern in FORBIDDEN_GLOBS:
        found.extend(root.glob(pattern))
    return sorted(set(found), key=lambda path: path.name)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=pathlib.Path,
        default=pathlib.Path(__file__).resolve().parents[1],
    )
    args = parser.parse_args()

    root = args.root.resolve()
    artifacts = runtime_artifacts(root)
    if artifacts:
        sys.stderr.write(
            "source hygiene check failed: runtime memory artifacts found\n"
        )
        for path in artifacts:
            sys.stderr.write(f"- {path.relative_to(root)}\n")
        sys.stderr.write(
            "move required data to the intended private runtime store or remove stale fixtures; "
            "the source checkout must remain runtime-data free\n"
        )
        return 1

    print("source hygiene ok: no runtime memory artifacts in the source checkout")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
