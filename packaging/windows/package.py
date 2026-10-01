#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Stage the portable Windows x86_64 payload and its zip; never run Oxplay.

Native releases stage the verified private media DLL closure beside oxplay.exe,
with its source/license evidence and pinned helpers. The explicit legacy mode
retains the stock-libmpv comparison payload.
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


def native_inputs(prefix: Path) -> tuple[dict, dict[str, Path], Path]:
    manifest = prefix / "native-media-provenance.json"
    if manifest.is_symlink() or not manifest.is_file() or manifest.stat().st_size > 4 * 1024 * 1024:
        raise ValueError("Native media provenance is missing or invalid")
    evidence = json.loads(manifest.read_text(encoding="utf-8"))
    if evidence.get("native_render_abi") != 1 or evidence.get("platform") != "windows-d3d11":
        raise ValueError("Native media provenance does not identify the required Windows ABI")
    header = prefix / "include/mpv/render_d3d11.h"
    if not header.is_file() or "#define OXPLAY_NATIVE_RENDER_ABI 1" not in header.read_text(encoding="utf-8"):
        raise ValueError("Native D3D11 render header is missing or incompatible")
    inventory = evidence.get("runtime_dlls_sha256")
    if not isinstance(inventory, dict) or not inventory or len(inventory) > 256:
        raise ValueError("Native media DLL inventory is missing or exceeds its bound")
    if "libmpv-2.dll" not in inventory or not any("placebo" in name.lower() for name in inventory):
        raise ValueError("Native media inventory lacks libmpv or libplacebo")
    files = {}
    for name, checksum in inventory.items():
        if (not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9_.+-]+\.dll", name)
                or not isinstance(checksum, str) or not re.fullmatch(r"[a-f0-9]{64}", checksum)):
            raise ValueError("Native DLL inventory contains an invalid entry")
        source = prefix / name
        if source.is_symlink() or not source.is_file() or sha256(source) != checksum:
            raise ValueError(f"Native DLL checksum mismatch: {name}")
        portable_executable(source)
        files[name] = source
    if len({name.casefold() for name in files}) != len(files):
        raise ValueError("Native DLL inventory has colliding Windows filenames")
    source_evidence = prefix / "share/oxplay-native"
    if source_evidence.is_symlink() or not source_evidence.is_dir():
        raise ValueError("Native media corresponding-source evidence is missing")
    for name in ("sources.tar.gz", "sources.json"):
        if not (source_evidence / name).is_file():
            raise ValueError("Native media corresponding-source evidence is incomplete")
    if any(path.is_symlink() for path in source_evidence.rglob("*")):
        raise ValueError("Native media source evidence contains a symlink")
    if (evidence.get("source_bundle") != "share/oxplay-native/sources.tar.gz"
            or evidence.get("source_bundle_sha256") != sha256(source_evidence / "sources.tar.gz")):
        raise ValueError("Native media source archive checksum mismatch")
    source_manifest = source_evidence / "sources.json"
    if source_manifest.stat().st_size > 4 * 1024 * 1024:
        raise ValueError("Native media source manifest exceeds its bound")
    source_records = json.loads(source_manifest.read_text(encoding="utf-8"))
    if (source_records.get("schema") != 1 or source_records.get("native_render_abi") != 1
            or source_records.get("platform") != "windows-d3d11"
            or source_records.get("bundle") != "sources.tar.gz"
            or source_records.get("bundle_sha256") != evidence["source_bundle_sha256"]):
        raise ValueError("Native media source manifest does not match the build")
    for record in source_records.get("runtime_sources", []):
        relative = record.get("source_archive", "")
        if (not isinstance(relative, str) or not relative.startswith("share/oxplay-native/runtime-sources/")
                or ".." in Path(relative).parts or "\\" in relative):
            raise ValueError("Native runtime source archive path is invalid")
        archive = prefix / relative
        if not archive.is_file() or sha256(archive) != record.get("source_archive_sha256"):
            raise ValueError("Native runtime source archive checksum mismatch")
    return evidence, files, source_evidence


def installer_include(directory: Path, destination: Path) -> None:
    """Delete only staged files, without recursively removing a user folder."""
    files = sorted(path.relative_to(directory).as_posix() for path in directory.rglob("*") if path.is_file())
    directories = sorted((path.relative_to(directory).as_posix() for path in directory.rglob("*") if path.is_dir()),
                         key=lambda name: (name.count("/"), name), reverse=True)
    if any(not re.fullmatch(r"[A-Za-z0-9_./+ -]+", name) or ".." in Path(name).parts for name in files + directories):
        raise ValueError("Installer inventory contains an unsupported path")
    lines = ['; Generated from the exact staged Windows payload.']
    lines += ['  Delete "$INSTDIR\\' + name.replace('/', '\\') + '"' for name in files]
    lines += ['  RMDir "$INSTDIR\\' + name.replace('/', '\\') + '"' for name in directories]
    destination.write_text("\n".join(lines) + "\n", encoding="utf-8")


def stage(binary: Path, mpv_dir: Path, helpers: Path, version: str, output: Path, *, native: bool = False) -> Path:
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("Expected a semantic application version")
    if output.exists():
        raise ValueError(f"Refusing to overwrite {output}")
    inputs = notices.pinned_inputs(helpers)
    media, source_evidence = None, None
    sources = {"oxplay.exe": binary, "yt-dlp.exe": helpers / "yt-dlp.exe", "deno.exe": helpers / "deno.exe"}
    if native:
        media, media_files, source_evidence = native_inputs(mpv_dir)
        sources.update(media_files)
    else:
        sources["libmpv-2.dll"] = mpv_dir / "libmpv-2.dll"
    for name, source in sources.items():
        if source.is_symlink() or not source.is_file():
            raise ValueError(f"Missing Windows payload input: {name}")
        portable_executable(source)
    output.mkdir(parents=True)
    for name, source in sources.items():
        shutil.copyfile(source, output / name)
    shutil.copyfile(ROOT / "LICENSE", output / "LICENSE.txt")
    notices.copy_helper_licenses(output / "licenses")
    mpv = next((item for item in inputs if item["name"] == "mpv-dev-windows-x86_64"), None)
    if not native and mpv is None:
        raise ValueError("The pinned libmpv provenance is missing")
    if native:
        shutil.copytree(source_evidence, output / "native-media")
        shutil.copyfile(mpv_dir / "native-media-provenance.json", output / "native-media/provenance.json")
        shutil.copyfile(mpv_dir / "include/mpv/render_d3d11.h", output / "native-media/render_d3d11.h")
        media_notice = ("private ABI-1 mpv/libplacebo D3D11 build, shared with the DX12 UI. "
                        "Exact native sources, patches, licenses and dependency inventory are in native-media/.")
        inputs = [item for item in inputs if not item["name"].startswith("mpv-dev")]
    else:
        media_notice = (f"libmpv-2.dll from shinchiro/mpv-winbuild-cmake {mpv['version']} (mpv, FFmpeg and their "
                        "dependencies; GPL). Build recipes: https://github.com/shinchiro/mpv-winbuild-cmake")
    (output / "THIRD-PARTY-NOTICES.txt").write_text(notices.render(
        inputs, media_notice), encoding="utf-8")
    (output / "README.txt").write_text(
        f"Oxplay {version} for Windows x86_64 (experimental, unsigned).\r\n\r\n"
        "Run oxplay.exe. All bundled DLLs, yt-dlp.exe and deno.exe must stay in this folder.\r\n"
        + ("Rendering: DX12 UI and same-adapter D3D11 hardware video (native ABI 1).\r\n" if native else "") +
        "Windows SmartScreen may warn because the executables are not Authenticode signed.\r\n"
        "Project: https://github.com/ViceVerse-cz/oxplay\r\n", encoding="utf-8")
    (output / "BUILD-INFO.json").write_text(json.dumps({
        "schema": 1, "version": version,
        "files": {path.relative_to(output).as_posix(): sha256(path) for path in sorted(output.rglob("*")) if path.is_file()},
        "pinned_inputs": inputs, "rendering": "dx12-d3d11" if native else "opengl",
        "native_media": media}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
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
    parser.add_argument("--native-media", action="store_true", help="Require the private D3D11 media ABI, DLL inventory and source evidence")
    parser.add_argument("--uninstall-include", type=Path, help="Write an NSIS include that deletes only the staged payload")
    args = parser.parse_args()
    try:
        version = args.version or tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        directory = stage(args.binary, args.mpv_dir, args.helpers, version, args.output, native=args.native_media)
        print(directory)
        if args.uninstall_include:
            installer_include(directory, args.uninstall_include)
        if args.zip:
            print(archive(directory, args.zip))
    except (ValueError, OSError, KeyError) as error:
        print(f"Windows packaging stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
