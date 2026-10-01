#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Download release inputs pinned by exact URL, size bound and SHA-256.

Used by the release/package workflows for bundled helpers (yt-dlp, Deno),
the Windows libmpv development archive and AppImage/rustup build tools.
Nothing is extracted, marked executable or reported before its complete bytes
match the pin. Update a pin only in a reviewed commit, using the upstream
release digest.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

MIB = 1024 * 1024
# name: url, sha256, max bytes, kind ("file"|"zip"|"7z"), output name / zip member.
PINS: dict[str, dict] = {
    "yt-dlp-linux-x86_64": {
        "version": "2026.08.19", "kind": "file", "output": "yt-dlp", "max_bytes": 96 * MIB,
        "url": "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp_linux",
        "sha256": "58162f9bfdc27458ea47bfcb311cf47028f17d8154a8bf7d689861d46399230a",
        "license": "Unlicense (yt-dlp); bundled PyInstaller runtime components retain their licenses",
        "source": "https://github.com/yt-dlp/yt-dlp/tree/2026.08.19"},
    "yt-dlp-windows-x86_64": {
        "version": "2026.08.19", "kind": "file", "output": "yt-dlp.exe", "max_bytes": 64 * MIB,
        "url": "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.19/yt-dlp.exe",
        "sha256": "66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a",
        "license": "Unlicense (yt-dlp); bundled PyInstaller runtime components retain their licenses",
        "source": "https://github.com/yt-dlp/yt-dlp/tree/2026.08.19"},
    "deno-linux-x86_64": {
        "version": "2.9.7", "kind": "zip", "member": "deno", "output": "deno", "max_bytes": 96 * MIB,
        "url": "https://github.com/denoland/deno/releases/download/v2.9.7/deno-x86_64-unknown-linux-gnu.zip",
        "sha256": "c6527f24f4b16031d3ae4fa9f658d5f11534c8d84ce7dc8502420280919c3490",
        "license": "MIT", "source": "https://github.com/denoland/deno/tree/v2.9.7"},
    "deno-windows-x86_64": {
        "version": "2.9.7", "kind": "zip", "member": "deno.exe", "output": "deno.exe", "max_bytes": 96 * MIB,
        "url": "https://github.com/denoland/deno/releases/download/v2.9.7/deno-x86_64-pc-windows-msvc.zip",
        "sha256": "a0c3101b4158d1dfb7d6a78a7bf0f3de80c96bb423c152beec8beb22786f2238",
        "license": "MIT", "source": "https://github.com/denoland/deno/tree/v2.9.7"},
    # shinchiro/mpv-winbuild-cmake: include/mpv/*.h, libmpv.dll.a and libmpv-2.dll.
    "mpv-dev-windows-x86_64": {
        "version": "20260928-git-e470f8986e", "kind": "7z", "output": "mpv-dev", "max_bytes": 64 * MIB,
        "url": "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/20260928/"
               "mpv-dev-x86_64-20260928-git-e470f8986e.7z",
        "sha256": "81795d759e01016f1550fd71651a1a5d59ab5c28ef31c0b6793224e9cff39459",
        "license": "GPL-3.0-or-later build of mpv (GPL-2.0-or-later/LGPL) with FFmpeg and other libraries",
        "source": "https://github.com/mpv-player/mpv/commit/e470f8986e"},
    "appimagetool-x86_64": {
        "version": "1.9.1", "kind": "file", "output": "appimagetool", "max_bytes": 32 * MIB,
        "url": "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage",
        "sha256": "ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0",
        "license": "MIT", "source": "https://github.com/AppImage/appimagetool/tree/1.9.1"},
    "appimage-runtime-x86_64": {
        "version": "20251108", "kind": "file", "output": "runtime-x86_64", "max_bytes": 8 * MIB,
        "url": "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64",
        "sha256": "2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d",
        "license": "MIT", "source": "https://github.com/AppImage/type2-runtime/tree/20251108"},
    "rustup-init-linux-x86_64": {
        "version": "1.29.1", "kind": "file", "output": "rustup-init", "max_bytes": 64 * MIB,
        "url": "https://static.rust-lang.org/rustup/archive/1.29.1/x86_64-unknown-linux-gnu/rustup-init",
        "sha256": "dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71",
        "license": "MIT OR Apache-2.0", "source": "https://github.com/rust-lang/rustup/tree/1.29.1"},
}
EXECUTABLE = {"yt-dlp", "yt-dlp.exe", "deno", "deno.exe", "appimagetool", "runtime-x86_64", "rustup-init"}


class PinError(Exception):
    pass


def download(url: str, destination: Path, limit: int) -> None:
    if not url.startswith("https://"):
        raise PinError("Pinned downloads must use HTTPS")
    request = urllib.request.Request(url, headers={"User-Agent": "oxplay-release-fetch"})
    with urllib.request.urlopen(request, timeout=60) as response, destination.open("xb") as output:
        if not response.geturl().startswith("https://"):
            raise PinError("A pinned download redirected away from HTTPS")
        total = 0
        while block := response.read(MIB):
            total += len(block)
            if total > limit:
                raise PinError("A pinned download exceeded its size bound")
            output.write(block)


def verify(path: Path, pin: dict) -> None:
    if path.stat().st_size > pin["max_bytes"]:
        raise PinError("A pinned download exceeded its size bound")
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while block := stream.read(MIB):
            digest.update(block)
    if digest.hexdigest() != pin["sha256"]:
        raise PinError("A pinned download does not match its SHA-256")


def extract_zip_member(archive: Path, member: str, destination: Path) -> None:
    with zipfile.ZipFile(archive) as bundle:
        names = [info for info in bundle.infolist() if info.filename == member]
        if len(names) != 1 or names[0].is_dir() or names[0].file_size > 512 * MIB:
            raise PinError("The pinned archive lacks exactly one expected member")
        with bundle.open(names[0]) as source, destination.open("xb") as output:
            shutil.copyfileobj(source, output, MIB)


def extract_7z(archive: Path, destination: Path) -> None:
    tool = shutil.which("7z") or shutil.which("7zz")
    if tool is None:
        raise PinError("7-Zip is required to extract the pinned libmpv archive")
    listing = subprocess.run([tool, "l", "-slt", "-ba", str(archive)], capture_output=True, text=True,
                             check=True, timeout=120).stdout
    for line in listing.splitlines():
        if line.startswith("Path = "):
            path = PurePosixPath(line.removeprefix("Path = ").replace("\\", "/"))
            if path.is_absolute() or ".." in path.parts or ":" in line.removeprefix("Path = "):
                raise PinError("The pinned 7z archive contains an unsafe path")
    destination.mkdir()
    subprocess.run([tool, "x", "-y", f"-o{destination}", str(archive)], stdout=subprocess.DEVNULL, check=True,
                   timeout=600)


def fetch(name: str, directory: Path, downloader=download) -> Path:
    pin = PINS.get(name)
    if pin is None:
        raise PinError("Unknown pinned input")
    directory.mkdir(parents=True, exist_ok=True)
    output = directory / pin["output"]
    if output.exists() or output.is_symlink():
        raise PinError(f"Refusing to overwrite {output}")
    with tempfile.TemporaryDirectory(prefix=".fetch-", dir=directory) as temporary:
        archive = Path(temporary) / "download"
        downloader(pin["url"], archive, pin["max_bytes"])
        verify(archive, pin)
        if pin["kind"] == "file":
            archive.replace(output)
        elif pin["kind"] == "zip":
            staged = Path(temporary) / "member"
            extract_zip_member(archive, pin["member"], staged)
            staged.replace(output)
        else:
            staged = Path(temporary) / "extracted"
            extract_7z(archive, staged)
            staged.replace(output)
    if output.is_file() and output.name in EXECUTABLE:
        output.chmod(output.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    return output


def provenance(names: list[str]) -> list[dict]:
    return [{"name": name, **{key: PINS[name][key] for key in ("version", "url", "sha256", "license", "source")}}
            for name in names]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("names", nargs="+", choices=sorted(PINS))
    parser.add_argument("--dest", type=Path, required=True)
    args = parser.parse_args()
    try:
        for name in args.names:
            print(fetch(name, args.dest))
        record = args.dest / "pinned-inputs.json"
        existing = json.loads(record.read_text()) if record.is_file() else []
        known = {item["name"] for item in existing}
        record.write_text(json.dumps(existing + [item for item in provenance(args.names) if item["name"] not in known],
                                     indent=2, sort_keys=True) + "\n")
    except (PinError, OSError, zipfile.BadZipFile, subprocess.SubprocessError) as error:
        print(f"Pinned fetch stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
