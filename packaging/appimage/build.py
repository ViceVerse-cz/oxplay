#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Package Oxplay as a Type 2 AppImage and a portable tarball; never launch it.

The private libmpv and its non-system shared-library closure (FFmpeg, libass,
libplacebo, ...) are copied into usr/lib with RUNPATH set, so no
LD_LIBRARY_PATH leaks into helpers or browsers the app starts. Libraries every
desktop provides (glibc, GPU drivers, X11/Wayland, audio servers, fontconfig)
come from the host, as in the AppImage project's exclude list.
"""
from __future__ import annotations

import argparse
import filecmp
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "packaging/linux"))
import package  # noqa: E402

REPOSITORY = ("ViceVerse-cz", "oxplay")
# Host-provided libraries (AppImage excludelist, trimmed to what this closure can reach).
EXCLUDED = re.compile(r"^(?:" + "|".join([
    r"ld-linux.*", r"libanl", r"libBrokenLocale", r"libc", r"libdl", r"libm", r"libmvec", r"libnss_.*",
    r"libpthread", r"libresolv", r"librt", r"libthread_db", r"libutil", r"libgcc_s", r"libstdc\+\+",
    r"libGL", r"libGLX.*", r"libGLdispatch", r"libOpenGL", r"libEGL.*", r"libGLESv2", r"libgbm", r"libdrm.*",
    r"libvulkan", r"libOpenCL", r"libva.*", r"libvdpau", r"libX11.*", r"libX[a-z]+", r"libxcb.*",
    r"libxkbcommon.*", r"libwayland-.*", r"libxshmfence", r"libasound", r"libpulse.*", r"libjack",
    r"libpipewire.*", r"libfontconfig", r"libfreetype", r"libharfbuzz", r"libexpat", r"libz",
    r"libdbus-1", r"libsystemd", r"libudev", r"libcap", r"libuuid", r"libblkid", r"libmount", r"libselinux",
    r"libcom_err", r"libgpg-error", r"libkeyutils", r"libkrb5.*", r"libgssapi_krb5", r"libk5crypto",
]) + r")\.so(?:\.[0-9]+)*$")
MAX_APPIMAGE = 1024 * 1024 * 1024


def update_information(version: str, tag: str) -> tuple[str, str]:
    if not package.SEMVER.fullmatch(version):
        raise ValueError("Expected a semantic application version")
    if tag != "development" and tag != f"v{version}":
        raise ValueError("The AppImage release tag must match the application version")
    channel = "latest-pre" if "-" in version else "latest"
    name = f"oxplay-{tag}-Linux-X64.AppImage"
    return name, f"gh-releases-zsync|{REPOSITORY[0]}|{REPOSITORY[1]}|{channel}|oxplay-*-Linux-X64.AppImage.zsync"


def ldd(path: Path, library_path: str | None) -> dict[str, str | None]:
    environment = {key: value for key, value in os.environ.items() if key != "LD_LIBRARY_PATH"}
    if library_path:
        environment["LD_LIBRARY_PATH"] = library_path
    resolved: dict[str, str | None] = {}
    for line in package.output("ldd", path, env=environment).splitlines():
        if match := re.match(r"^\s*(\S+) => (not found|(/\S+)) \(", line):
            resolved[match[1]] = match[3]
    return resolved


def bundle_libraries(appdir: Path, mpv_prefix: Path) -> list[str]:
    executable = appdir / "usr/lib/oxplay/oxplay"
    libraries = ldd(executable, str(mpv_prefix / "lib"))
    missing = [name for name, path in libraries.items() if path is None]
    if missing or "libmpv.so.2" not in libraries:
        raise ValueError(f"Unresolved build-host libraries: {', '.join(missing) or 'libmpv.so.2'}")
    # A library that a host-provided library also needs (libffi for libwayland,
    # for example) must come from the host too: the loader shares one copy per soname.
    host = {name for name in libraries if EXCLUDED.match(name)}
    for name in sorted(host):
        host |= set(ldd(Path(libraries[name]), None))
    bundled = []
    for name, source in sorted(libraries.items()):
        if name in host:
            continue
        destination = appdir / "usr/lib" / name
        shutil.copyfile(Path(source).resolve(strict=True), destination)
        destination.chmod(0o644)
        package.checked("patchelf", "--set-rpath", "$ORIGIN", destination)
        bundled.append(name)
    package.checked("patchelf", "--set-rpath", "$ORIGIN/..", executable)
    # Re-resolve without the build prefix: bundled names must load from usr/lib.
    for name, path in ldd(executable, None).items():
        if path is None:
            raise ValueError(f"AppImage library closure is incomplete: {name}")
        inside = Path(path).resolve().is_relative_to(appdir.resolve())
        if (name in bundled) != inside:
            raise ValueError(f"AppImage library {name} resolved to an unexpected location")
    return bundled


def build(binary: Path, helpers: Path, mpv_prefix: Path, version: str, tag: str, destination: Path) -> list[Path]:
    package.native_elf(binary)
    if platform.machine() != "x86_64":
        raise ValueError("AppImage releases currently support Linux x86_64 only")
    name, information = update_information(version, tag)
    tool, runtime = helpers / "appimagetool", helpers / "runtime-x86_64"
    if not tool.is_file() or not runtime.is_file():
        raise ValueError("Fetch appimagetool-x86_64 and appimage-runtime-x86_64 with scripts/fetch_pinned.py first")
    for command in ("file", "zsyncmake", "patchelf", "desktop-file-validate"):
        if not shutil.which(command):
            raise ValueError(f"The '{command}' command is required")
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="oxplay-appimage-") as directory:
        temporary = Path(directory)
        appdir = temporary / "Oxplay.AppDir"
        package.stage_payload(appdir, binary, helpers, mpv_prefix, portable=True)
        bundled = bundle_libraries(appdir, mpv_prefix)
        shutil.copyfile(ROOT / "packaging/appimage/AppRun", appdir / "AppRun")
        (appdir / "AppRun").chmod(0o755)
        shutil.copyfile(ROOT / "packaging/linux" / f"{package.APP_ID}.desktop", appdir / f"{package.APP_ID}.desktop")
        shutil.copyfile(ROOT / "packaging/linux/oxplay.svg", appdir / "oxplay.svg")
        doc = appdir / "usr/share/doc/oxplay"
        shutil.copyfile(ROOT / "packaging/appimage/RUNTIME-LICENSE", doc / "licenses/AppImage-runtime.txt")
        (doc / "AppImage-bundled-libraries.txt").write_text("\n".join(bundled) + "\n", encoding="utf-8")
        package.checked("desktop-file-validate", appdir / f"{package.APP_ID}.desktop")
        candidate = temporary / name
        subprocess.run([str(tool), "--runtime-file", str(runtime), "--updateinformation", information,
                        "--no-appstream", str(appdir), str(candidate)], check=True, cwd=temporary,
                       env={**os.environ, "ARCH": "x86_64", "VERSION": version, "APPIMAGE_EXTRACT_AND_RUN": "1"})
        with candidate.open("rb") as stream:
            header = stream.read(20)
        if header[:6] != b"\x7fELF\x02\x01" or header[8:11] != b"AI\x02" or header[18:20] != b"\x3e\x00":
            raise ValueError("Expected a Type 2 x86_64 AppImage")
        if candidate.stat().st_size > MAX_APPIMAGE:
            raise ValueError("The AppImage exceeds its size bound")
        candidate.chmod(0o755)
        # The runtime answers --appimage-* itself (no FUSE mount); never set
        # APPIMAGE_EXTRACT_AND_RUN here, or the option reaches the application.
        runtime_env = {key: value for key, value in os.environ.items() if key != "APPIMAGE_EXTRACT_AND_RUN"}
        embedded = subprocess.check_output([str(candidate), "--appimage-updateinformation"], text=True,
                                           env=runtime_env)
        if embedded.strip() != information:
            raise ValueError("AppImage update information did not survive packaging")
        zsync = candidate.with_name(candidate.name + ".zsync")
        if not zsync.is_file() or not 0 < zsync.stat().st_size <= 16 * 1024 * 1024:
            raise ValueError("AppImage zsync metadata is missing, empty or oversized")
        # Run only the pinned runtime's extraction, never AppRun or the application.
        subprocess.run([str(candidate), "--appimage-extract"], cwd=temporary, stdout=subprocess.DEVNULL, check=True,
                       env=runtime_env)
        extracted = temporary / "squashfs-root"
        for source in appdir.rglob("*"):
            if source.is_file():
                target = extracted / source.relative_to(appdir)
                if not target.is_file() or not filecmp.cmp(source, target, shallow=False):
                    raise ValueError(f"AppImage changed payload: {source.relative_to(appdir)}")
        for path in ("AppRun", "usr/bin/oxplay", "usr/lib/oxplay/oxplay", "usr/lib/oxplay/yt-dlp", "usr/lib/oxplay/deno"):
            if not os.access(extracted / path, os.X_OK):
                raise ValueError(f"AppImage lost executable permission: {path}")
        results = [destination / candidate.name, destination / zsync.name]
        shutil.copyfile(candidate, results[0])
        results[0].chmod(0o755)
        shutil.copyfile(zsync, results[1])
        results.append(tarball(appdir, destination / f"oxplay-{tag}-Linux-X64.tar.gz"))
        print(f"Unsigned AppImage and tarball: {', '.join(path.name for path in results)}; "
              f"{len(bundled)} bundled libraries")
        return results


def tarball(appdir: Path, archive: Path) -> Path:
    """Same payload as the AppImage; ./oxplay is the AppRun entry point."""
    prefix = archive.name.removesuffix(".tar.gz")
    epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))

    def normalize(info: tarfile.TarInfo) -> tarfile.TarInfo:
        info.uid = info.gid = 0
        info.uname = info.gname = ""
        info.mtime = epoch
        return info

    members = sorted(path for path in appdir.rglob("*") if path.name not in ("AppRun", ".DirIcon"))
    with tarfile.open(archive, "x:gz", format=tarfile.PAX_FORMAT) as output:
        output.add(appdir / "AppRun", arcname=f"{prefix}/oxplay", filter=normalize)
        for path in members:
            if path.is_symlink():
                raise ValueError(f"Unexpected symlink in the portable payload: {path.name}")
            output.add(path, arcname=f"{prefix}/{path.relative_to(appdir).as_posix()}", recursive=False,
                       filter=normalize)
    return archive


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/oxplay")
    parser.add_argument("--helpers", type=Path, required=True, help="Directory from scripts/fetch_pinned.py")
    parser.add_argument("--mpv-prefix", type=Path, required=True)
    parser.add_argument("--version", default=None, help="Defaults to the workspace version")
    parser.add_argument("--tag", default="development")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    try:
        build(args.binary.resolve(strict=True), args.helpers.resolve(strict=True), args.mpv_prefix.resolve(strict=True),
              args.version or package.workspace_version(), args.tag, args.output.resolve())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"AppImage packaging stopped: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
