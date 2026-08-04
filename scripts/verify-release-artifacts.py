#!/usr/bin/env python3
"""Verify cargo-dist archives, checksums, installers, SBOM, and native binaries."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import stat
import subprocess
import sys
import tarfile
import tempfile
import zipfile
from pathlib import Path, PurePosixPath

MAX_BINARY_BYTES = 256 * 1024 * 1024


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def constrained(directory: Path, path: Path) -> Path:
    resolved = path.resolve(strict=True)
    if not resolved.is_relative_to(directory):
        raise ValueError(f"artifact escapes distribution directory: {path}")
    return resolved


def verify_sidecar(path: Path) -> None:
    directory = path.parent.resolve(strict=True)
    path = constrained(directory, path)
    sidecar_path = path.with_name(path.name + ".sha256")
    if not sidecar_path.is_file():
        raise ValueError(f"missing checksum sidecar for {path.name}")
    sidecar = constrained(directory, sidecar_path)
    fields = sidecar.read_text(encoding="utf-8").split()
    if not fields:
        raise ValueError(f"empty checksum sidecar for {path.name}")
    expected = fields[0].lower()
    if len(expected) != 64 or any(
        character not in "0123456789abcdef" for character in expected
    ):
        raise ValueError(f"invalid checksum sidecar for {path.name}")
    if digest(path) != expected:
        raise ValueError(f"checksum mismatch for {path.name}")


def safe_names(names: list[str], archive: Path) -> set[str]:
    basenames: set[str] = set()
    for name in names:
        normalized = PurePosixPath(name.replace("\\", "/"))
        if normalized.is_absolute() or ".." in normalized.parts:
            raise ValueError(f"unsafe path {name!r} in {archive.name}")
        if normalized.name:
            basenames.add(normalized.name)
    return basenames


def checked_zip_names(archive: zipfile.ZipFile, path: Path) -> set[str]:
    members = archive.infolist()
    for member in members:
        safe_names([member.filename], path)
        mode = member.external_attr >> 16
        if stat.S_IFMT(mode) == stat.S_IFLNK:
            raise ValueError(
                f"archive contains an unexpected symlink: {member.filename}"
            )
    return safe_names([member.filename for member in members], path)


def checked_tar_names(archive: tarfile.TarFile, path: Path) -> set[str]:
    members = archive.getmembers()
    for member in members:
        safe_names([member.name], path)
        if member.issym() or member.islnk():
            raise ValueError(f"archive contains an unexpected link: {member.name}")
    return safe_names([member.name for member in members], path)


def verify_binary_archive(directory: Path, path: Path) -> None:
    path = constrained(directory, path)
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            names = checked_zip_names(archive, path)
        executable = "mem.exe"
    else:
        with tarfile.open(path, mode="r:xz") as archive:
            names = checked_tar_names(archive, path)
        executable = "mem"
    required = {executable, "README.md", "CHANGELOG.md", "LICENSE"}
    missing = required - names
    if missing:
        raise ValueError(f"{path.name} is missing: {', '.join(sorted(missing))}")
    verify_sidecar(path)


def verify_source_archive(directory: Path, path: Path) -> None:
    path = constrained(directory, path)
    with tarfile.open(path, mode="r:gz") as archive:
        names = checked_tar_names(archive, path)
    missing = {"Cargo.toml", "Cargo.lock", "README.md", "LICENSE"} - names
    if missing:
        raise ValueError(f"{path.name} is missing: {', '.join(sorted(missing))}")
    verify_sidecar(path)


def binary_from_archive(directory: Path, path: Path) -> tuple[bytes, int]:
    path = constrained(directory, path)
    executable = "mem.exe" if path.suffix == ".zip" else "mem"
    found: list[tuple[bytes, int]] = []
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            checked_zip_names(archive, path)
            for member in archive.infolist():
                if member.is_dir() or PurePosixPath(member.filename).name != executable:
                    continue
                if member.file_size > MAX_BINARY_BYTES:
                    raise ValueError(f"{executable} exceeds the extraction limit")
                found.append((archive.read(member), member.external_attr >> 16))
    else:
        with tarfile.open(path, mode="r:xz") as archive:
            checked_tar_names(archive, path)
            for member in archive.getmembers():
                if not member.isfile() or PurePosixPath(member.name).name != executable:
                    continue
                if member.size > MAX_BINARY_BYTES:
                    raise ValueError(f"{executable} exceeds the extraction limit")
                source = archive.extractfile(member)
                if source is None:
                    raise ValueError(f"could not read {executable} from {path.name}")
                found.append((source.read(), member.mode))
    if len(found) != 1:
        raise ValueError(f"{path.name} should contain exactly one {executable}")
    return found[0]


def native_target() -> str | None:
    architecture = {
        "amd64": "x86_64",
        "x86_64": "x86_64",
        "arm64": "aarch64",
        "aarch64": "aarch64",
    }.get(platform.machine().lower())
    system = {
        "Darwin": "apple-darwin",
        "Linux": "unknown-linux-gnu",
        "Windows": "pc-windows-msvc",
    }.get(platform.system())
    return f"{architecture}-{system}" if architecture and system else None


def execute_native_archive(directory: Path, archives: list[Path]) -> None:
    target = native_target()
    if target is None:
        raise ValueError("cannot determine the native release target")
    archive = next((path for path in archives if target in path.name), None)
    if archive is None:
        raise ValueError(f"no release archive found for native target {target}")
    data, mode = binary_from_archive(directory, archive)
    with tempfile.TemporaryDirectory(prefix="mnemark-release-smoke-") as temp:
        temp_dir = Path(temp)
        binary = temp_dir / ("mem.exe" if archive.suffix == ".zip" else "mem")
        binary.write_bytes(data)
        if archive.suffix != ".zip":
            binary.chmod(mode | stat.S_IXUSR)

        version = subprocess.run(
            [str(binary), "--version"],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
        if "mem " not in f"{version.stdout}{version.stderr}".lower():
            raise ValueError("native archive returned unexpected --version output")

        contract = subprocess.run(
            [
                str(binary),
                "--home",
                str(temp_dir / "isolated-store"),
                "--read-only",
                "contract",
            ],
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )
        try:
            document = json.loads(contract.stdout)
        except json.JSONDecodeError as error:
            raise ValueError("native archive returned invalid contract JSON") from error
        if document.get("status") != "ok" or not document.get("cli_version"):
            raise ValueError("native archive returned an invalid contract")


def verify_sbom(path: Path) -> None:
    verify_sidecar(path)
    if path.stat().st_size > 16 * 1024 * 1024:
        raise ValueError("mnemark.cdx.xml exceeds the 16 MiB validation limit")
    document = path.read_text(encoding="utf-8")
    if 'xmlns="http://cyclonedx.org/schema/bom/1.5"' not in document:
        raise ValueError("mnemark.cdx.xml is not CycloneDX 1.5")
    if "<component " not in document or '<hash alg="SHA-256">' not in document:
        raise ValueError("SBOM has no components or SHA-256 component hashes")


def verify_installers(directory: Path) -> None:
    shell = constrained(directory, directory / "mnemark-installer.sh")
    powershell = constrained(directory, directory / "mnemark-installer.ps1")
    shell_text = shell.read_text(encoding="utf-8")
    powershell_text = powershell.read_text(encoding="utf-8")
    if (
        "verify_checksum" not in shell_text
        or "sha256sum" not in shell_text
        or "shasum -a 256" not in shell_text
        or "openssl dgst -sha256" not in shell_text
        or "missing checksum for downloaded archive" not in shell_text
        or "skipping sha256 checksum verification" in shell_text
    ):
        raise ValueError("shell installer does not fail closed on archive checksums")
    if (
        "Get-FileHash -Algorithm SHA256" not in powershell_text
        or "$url.sha256" not in powershell_text
    ):
        raise ValueError("PowerShell installer does not verify archive checksums")
    verify_sidecar(shell)
    verify_sidecar(powershell)


def verify_checksum_manifest(directory: Path) -> None:
    directory = directory.resolve(strict=True)
    manifest_path = directory / "sha256.sum"
    if not manifest_path.is_file():
        raise ValueError("missing sha256.sum")
    manifest = constrained(directory, manifest_path)
    for line_number, line in enumerate(
        manifest.read_text(encoding="utf-8").splitlines(), start=1
    ):
        if not line.strip():
            continue
        fields = line.split(maxsplit=1)
        if len(fields) != 2:
            raise ValueError(f"invalid sha256.sum entry on line {line_number}")
        expected, name = fields
        expected = expected.lower()
        if len(expected) != 64 or any(
            character not in "0123456789abcdef" for character in expected
        ):
            raise ValueError(f"invalid SHA-256 digest on line {line_number}")
        normalized = PurePosixPath(name.lstrip(" *").replace("\\", "/"))
        if normalized.is_absolute() or ".." in normalized.parts:
            raise ValueError(f"unsafe path {name!r} in sha256.sum")
        artifact_path = directory.joinpath(*normalized.parts)
        if not artifact_path.is_file():
            raise ValueError(f"sha256.sum references missing artifact: {normalized}")
        artifact = constrained(directory, artifact_path)
        if digest(artifact) != expected:
            raise ValueError(f"sha256.sum mismatch for {artifact.name}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--execute-native",
        action="store_true",
        help="execute the archived binary matching the current runner",
    )
    parser.add_argument(
        "--platform-only",
        action="store_true",
        help="verify only platform archives and their checksum sidecars",
    )
    arguments = parser.parse_args()

    directory = Path("target/distrib").resolve(strict=True)
    archives = [
        constrained(directory, path)
        for path in sorted(directory.glob("mnemark-*.tar.xz"))
        + sorted(directory.glob("mnemark-*.zip"))
    ]
    if not archives:
        raise ValueError("no mnemark platform archives found")
    for archive in archives:
        verify_binary_archive(directory, archive)
    if arguments.execute_native:
        execute_native_archive(directory, archives)
    if arguments.platform_only:
        sys.stdout.write(f"verified {len(archives)} platform archive(s)\n")
        return 0

    verify_source_archive(directory, directory / "source.tar.gz")
    verify_sbom(constrained(directory, directory / "mnemark.cdx.xml"))
    verify_installers(directory)
    verify_checksum_manifest(directory)
    sys.stdout.write(
        f"verified {len(archives)} platform archive(s), source archive, checksums, installers, and SBOM\n"
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile) as error:
        sys.stderr.write(f"artifact verification failed: {error}\n")
        raise SystemExit(1) from error
