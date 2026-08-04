#!/usr/bin/env python3
"""Make cargo-dist's generated shell and PowerShell installers fail closed on SHA-256."""

from __future__ import annotations

import sys
from pathlib import Path

POWERSHELL_MARKER = "  $wc.downloadFile($url, $dir_path)\n"
POWERSHELL_REPLACEMENT = """  $wc.downloadFile($url, $dir_path)

  # Fetch the release sidecar from the same authenticated origin and fail closed.
  $checksum_path = "$dir_path.sha256"
  $wc.downloadFile("$url.sha256", $checksum_path)
  $checksum_text = (Get-Content -Raw -Path $checksum_path).Trim()
  $expected_checksum = ($checksum_text -split '\\s+')[0].ToLowerInvariant()
  $actual_checksum = (Get-FileHash -Algorithm SHA256 -Path $dir_path).Hash.ToLowerInvariant()
  if ($actual_checksum -ne $expected_checksum) {
    Remove-Item -Force -ErrorAction SilentlyContinue $dir_path
    throw "ERROR: SHA-256 checksum verification failed for $artifact_name"
  }
"""

SHELL_MARKER = """    if [ -z "$_checksum_value" ]; then
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
"""
SHELL_REPLACEMENT = """    if [ -z "$_checksum_value" ]; then
        err "missing checksum for downloaded archive"
    fi
    case "$_checksum_style" in
        sha256)
            if check_cmd sha256sum; then
                _calculated_checksum="$(sha256sum -b "$_file" | awk '{printf $1}')"
            elif check_cmd shasum; then
                _calculated_checksum="$(shasum -a 256 "$_file" | awk '{printf $1}')"
            elif check_cmd openssl; then
                _calculated_checksum="$(openssl dgst -sha256 "$_file" | awk '{printf $NF}')"
            else
                err "sha256 checksum verification requires sha256sum, shasum, or openssl"
            fi
            ;;
"""


def distribution_file(distribution: Path, name: str) -> Path:
    path = (distribution / name).resolve(strict=True)
    if not path.is_relative_to(distribution) or not path.is_file():
        raise ValueError(f"installer is missing or escapes target/distrib: {name}")
    return path


def replace_once(path: Path, marker: str, replacement: str) -> None:
    source = path.read_text(encoding="utf-8")
    if source.count(marker) != 1:
        raise ValueError(f"expected exactly one cargo-dist marker in {path.name}")
    path.write_text(source.replace(marker, replacement), encoding="utf-8", newline="\n")


def main() -> int:
    root = Path.cwd().resolve(strict=True)
    distribution = (root / "target/distrib").resolve(strict=True)
    powershell = distribution_file(distribution, "mnemark-installer.ps1")
    shell = distribution_file(distribution, "mnemark-installer.sh")
    replace_once(powershell, POWERSHELL_MARKER, POWERSHELL_REPLACEMENT)
    replace_once(shell, SHELL_MARKER, SHELL_REPLACEMENT)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        sys.stderr.write(f"failed to harden installers: {error}\n")
        raise SystemExit(1) from error
