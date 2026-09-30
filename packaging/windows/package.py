#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage the portable Windows x86_64 payload and its zip; never run Oxplay.

Flat layout beside oxplay.exe: libmpv-2.dll (pinned shinchiro build), the
pinned yt-dlp.exe and deno.exe, license texts and BUILD-INFO.json. The NSIS
installer (installer.nsi) installs exactly this directory.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import sys
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "packaging"))
import notices  # noqa: E402

PAYLOAD = ("oxplay.exe", "libmpv-2.dll", "yt-dlp.exe", "deno.exe")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def portable_executable(path: Path) -> None:
    """Accept only x86_64 PE images (machine 0x8664)."""
    with path.open("rb") as stream:
        header = stream.read(4096)
    if header[:2] != b"MZ" or len(header) < 64:
        raise ValueError(f"{path.name} is not a Windows executable")
    offset = int.from_bytes(header[60:64], "little")
    if header[offset:offset + 4] != b"PE\0\0" or int.from_bytes(header[offset + 4:offset + 6], "little") != 0x8664:
        raise ValueError(f"{path.name} is not an x86_64 Windows image")


def stage(binary: Path, mpv_dir: Path, helpers: Path, version: str, output: Path) -> Path:
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("Expected a semantic application version")
    if output.exists():
        raise ValueError(f"Refusing to overwrite {output}")
    sources = {"oxplay.exe": binary, "libmpv-2.dll": mpv_dir / "libmpv-2.dll",
               "yt-dlp.exe": helpers / "yt-dlp.exe", "deno.exe": helpers / "deno.exe"}
    for name, source in sources.items():
        if source.is_symlink() or not source.is_file():
            raise ValueError(f"Missing Windows payload input: {name}")
        portable_executable(source)
    output.mkdir(parents=True)
    for name, source in sources.items():
        shutil.copyfile(source, output / name)
    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE.txt")
    notices.copy_helper_licenses(output / "licenses")
    inputs = notices.pinned_inputs(helpers)
    mpv = next((item for item in inputs if item["name"] == "mpv-dev-windows-x86_64"), None)
    if mpv is None:
        raise ValueError("The pinned libmpv provenance is missing")
    (output / "THIRD-PARTY-NOTICES.txt").write_text(notices.render(
        inputs, f"libmpv-2.dll from shinchiro/mpv-winbuild-cmake {mpv['version']} (mpv, FFmpeg and their "
                "dependencies; GPL). Build recipes: https://github.com/shinchiro/mpv-winbuild-cmake"), encoding="utf-8")
    (output / "README.txt").write_text(
        f"Oxplay {version} for Windows x86_64 (experimental, unsigned).\r\n\r\n"
        "Run oxplay.exe. libmpv-2.dll, yt-dlp.exe and deno.exe must stay in this folder.\r\n"
        "Windows SmartScreen may warn because the executables are not Authenticode signed.\r\n"
        "Project: https://github.com/ViceVerse-cz/oxplay\r\n", encoding="utf-8")
    (output / "BUILD-INFO.json").write_text(json.dumps({
        "schema": 1, "version": version, "files": {name: sha256(output / name) for name in PAYLOAD},
        "pinned_inputs": inputs}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return output


def archive(directory: Path, destination: Path) -> Path:
    """Deterministic zip of the flat payload (no top-level folder, like the model project)."""
    if destination.exists():
        raise ValueError(f"Refusing to overwrite {destination}")
    with zipfile.ZipFile(destination, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for path in sorted(directory.rglob("*")):
            if path.is_file():
                info = zipfile.ZipInfo(path.relative_to(directory).as_posix(), date_time=(1980, 1, 1, 0, 0, 0))
                info.compress_type = zipfile.ZIP_DEFLATED
                info.external_attr = 0o644 << 16
                with path.open("rb") as source, bundle.open(info, "w") as target:
                    shutil.copyfileobj(source, target, 1024 * 1024)
    return destination


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/oxplay.exe")
    parser.add_argument("--mpv-dir", type=Path, required=True)
    parser.add_argument("--helpers", type=Path, required=True, help="Directory from scripts/fetch_pinned.py")
    parser.add_argument("--version", default=None)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/windows")
    parser.add_argument("--zip", type=Path, help="Also write this portable zip")
    args = parser.parse_args()
    try:
        version = args.version or tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        directory = stage(args.binary, args.mpv_dir, args.helpers, version, args.output)
        print(directory)
        if args.zip:
            print(archive(directory, args.zip))
    except (ValueError, OSError, KeyError) as error:
        print(f"Windows packaging stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
