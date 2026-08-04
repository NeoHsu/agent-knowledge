#!/usr/bin/env python3
"""Enforce a reviewable release-binary size budget."""

from __future__ import annotations

import argparse
import math
import sys
from pathlib import Path

DEFAULT_MAX_MIB = 50.0


def default_binary() -> Path:
    name = "mem.exe" if sys.platform == "win32" else "mem"
    return Path("target/release") / name


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=default_binary())
    parser.add_argument("--max-mib", type=float, default=DEFAULT_MAX_MIB)
    return parser.parse_args()


def verify(binary: Path, max_mib: float) -> tuple[int, int]:
    if not math.isfinite(max_mib) or max_mib <= 0:
        raise ValueError("--max-mib must be finite and greater than zero")
    if not binary.is_file():
        raise ValueError(f"release binary not found: {binary}")
    size = binary.stat().st_size
    try:
        limit = int(max_mib * 1024 * 1024)
    except (OverflowError, ValueError) as error:
        raise ValueError("--max-mib cannot be represented as bytes") from error
    if size > limit:
        raise ValueError(
            f"{binary} is {size / 1024 / 1024:.2f} MiB, exceeding the {max_mib:.2f} MiB budget"
        )
    return size, limit


def main() -> int:
    args = parse_args()
    try:
        size, limit = verify(args.binary, args.max_mib)
    except (OSError, ValueError) as error:
        sys.stderr.write(f"binary size check failed: {error}\n")
        return 1
    sys.stdout.write(
        f"verified {args.binary}: {size / 1024 / 1024:.2f} MiB <= {limit / 1024 / 1024:.2f} MiB\n"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
