#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Build the reviewed native Metal media stack into a private workspace prefix.

Archives are immutable SHA-256 inputs. --offline uses the already downloaded
archives and installed build dependencies without accessing the network.
Nothing is installed into Homebrew or a system directory.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import time
import urllib.request

SCRIPT = Path(__file__).resolve().parent
REPO = SCRIPT.parents[1]
PATCHES = {
    "libplacebo": ["macos-libplacebo-offscreen.patch"],
    "mpv": ["common-mpv-gpu-next.patch", "macos-mpv-metal-texture.patch"],
}


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source_inputs(root: Path, offline: bool) -> dict:
    pins = json.loads((SCRIPT / "macos-sources.json").read_text())["sources"]
    cache = root / "downloads"
    cache.mkdir(parents=True, exist_ok=True)
    for name, pin in pins.items():
        archive = cache / f"{name}-{pin['revision']}.tar.gz"
        if not archive.exists():
            if offline:
                raise RuntimeError(f"Offline source archive is missing: {archive}")
            with tempfile.NamedTemporaryFile(dir=cache, delete=False) as output:
                staging = Path(output.name)
                try:
                    request = urllib.request.Request(pin["url"], headers={"User-Agent": "oxplay-native-media"})
                    with urllib.request.urlopen(request, timeout=60) as response:
                        if not response.geturl().startswith("https://"):
                            raise RuntimeError("Source download redirected away from HTTPS")
                        count = 0
                        while block := response.read(1024 * 1024):
                            count += len(block)
                            if count > 64 * 1024 * 1024:
                                raise RuntimeError("Source archive exceeded its size bound")
                            output.write(block)
                    output.flush()
                    if digest(staging) != pin["sha256"]:
                        raise RuntimeError(f"Source checksum mismatch: {name}")
                    staging.rename(archive)
                finally:
                    staging.unlink(missing_ok=True)
        if archive.stat().st_size != pin["bytes"] or digest(archive) != pin["sha256"]:
            raise RuntimeError(f"Source checksum/size mismatch: {name}")
    return pins


def prepare(root: Path, pins: dict) -> None:
    # Reconstruct from the verified archives each time; incremental build output
    # is separate, so local edits never silently become dependency inputs.
    staging = Path(tempfile.mkdtemp(prefix="source-", dir=root))
    try:
        for name, pin in pins.items():
            destination = staging / pin["destination"]
            destination.mkdir(parents=True, exist_ok=True)
            archive = root / "downloads" / f"{name}-{pin['revision']}.tar.gz"
            with tarfile.open(archive, "r:gz") as bundle:
                for member in bundle.getmembers():
                    parts = Path(member.name).parts
                    if len(parts) > 1:
                        member.name = str(Path(*parts[1:]))
                        bundle.extract(member, destination, filter="data")
        for name, patches in PATCHES.items():
            for patch in patches:
                subprocess.run(["patch", "--batch", "--forward", "-p1", "-i", str(SCRIPT / "patches" / patch)],
                               cwd=staging / name, check=True)
        source = root / "source"
        if source.exists():
            shutil.rmtree(source)
        staging.rename(source)
    finally:
        if staging.exists():
            shutil.rmtree(staging)


def build_environment(root: Path, tools: Path | None) -> dict:
    env = os.environ.copy()
    if tools:
        env["PATH"] = str(tools) + os.pathsep + env.get("PATH", "")
    prefix = root / "prefix"
    env["PKG_CONFIG_PATH"] = str(prefix / "lib/pkgconfig") + os.pathsep + env.get("PKG_CONFIG_PATH", "")
    # Homebrew SDK system packages provide GL/appleframework pkg-config stubs.
    brew = shutil.which("brew", path=env["PATH"])
    include_flags = ""
    if brew:
        homebrew = Path(subprocess.check_output([brew, "--prefix"], text=True).strip())
        include_flags = "-I" + str(homebrew / "include") + " "
        sdk_pc = homebrew / "Library/Homebrew/os/mac/pkgconfig" / platform.mac_ver()[0].split(".")[0]
        pc_dirs = sorted({p.resolve() for kind in ("lib", "share")
                          for p in (homebrew / "opt").glob("*/" + kind + "/pkgconfig")
                          if p.is_dir()})
        env["PKG_CONFIG_PATH"] += os.pathsep + os.pathsep.join(map(str, pc_dirs))
        env["PKG_CONFIG_PATH"] += os.pathsep + str(homebrew / "lib/pkgconfig") + os.pathsep + str(sdk_pc)
    sdk = subprocess.check_output(["xcrun", "--sdk", "macosx", "--show-sdk-path"], text=True).strip()
    env["SDKROOT"] = sdk
    env["PKG_CONFIG"] = (shutil.which("pkgconf", path=env["PATH"]) or "pkg-config") + " --define-variable=homebrew_sdkroot=" + sdk
    env["MACOSX_DEPLOYMENT_TARGET"] = "12.0"
    for key in ("CFLAGS", "CXXFLAGS", "OBJCFLAGS"):
        env[key] = f"-I{prefix / 'include'} " + include_flags + env.get(key, "")
    env["LDFLAGS"] = f"-L{prefix / 'lib'} " + env.get("LDFLAGS", "")
    env["PYTHONPATH"] = os.pathsep.join(str(root / "source/libplacebo/3rdparty" / name / "src")
                                      for name in ("jinja", "markupsafe"))
    env["PYTHONDONTWRITEBYTECODE"] = "1"
    return env


def run_stage(root: Path, name: str, command: list[str], env: dict) -> dict:
    log = root / f"{name}.log"
    started = time.monotonic()
    print(name, flush=True)
    with log.open("w") as output:
        result = subprocess.run(command, cwd=REPO, env=env, stdout=output, stderr=subprocess.STDOUT)
    record = {"name": name, "command": command, "exit_code": result.returncode,
              "seconds": time.monotonic() - started, "log_sha256": digest(log)}
    if result.returncode:
        raise RuntimeError(f"{name} failed; inspect {log}")
    return record


def build(root: Path, pins: dict, env: dict, jobs: int) -> dict:
    prefix = root / "prefix"
    meson = shutil.which("meson", path=env["PATH"])
    cmake = shutil.which("cmake", path=env["PATH"])
    if not meson or not cmake or not shutil.which("ninja", path=env["PATH"]):
        raise RuntimeError("Meson, CMake and Ninja must be installed or supplied with --tools")
    stages = []
    cross = root / "build-spirv-cross"
    stages.append(run_stage(root, "spirv-cross-configure", [cmake, "-S", str(root / "source/spirv-cross"),
        "-B", str(cross), "-G", "Ninja", "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_INSTALL_PREFIX=" + str(prefix),
        "-DCMAKE_INSTALL_LIBDIR=lib", "-DSPIRV_CROSS_STATIC=ON", "-DSPIRV_CROSS_SHARED=OFF",
        "-DSPIRV_CROSS_CLI=OFF", "-DSPIRV_CROSS_ENABLE_TESTS=OFF", "-DSPIRV_CROSS_ENABLE_CPP=OFF",
        "-DSPIRV_CROSS_ENABLE_REFLECT=OFF", "-DCMAKE_OSX_DEPLOYMENT_TARGET=12.0"], env))
    stages.append(run_stage(root, "spirv-cross-compile", [cmake, "--build", str(cross), "--parallel", str(jobs)], env))
    stages.append(run_stage(root, "spirv-cross-install", [cmake, "--install", str(cross)], env))
    options = {
        "libplacebo": ["-Dmetal=enabled", "-Dvulkan=disabled", "-Dopengl=disabled", "-Dd3d11=disabled",
                        "-Dglslang=disabled", "-Dshaderc=enabled", "-Ddemos=false", "-Dtests=true", "-Dlibdovi=disabled"],
        "mpv": ["-Dlibmpv=true", "-Dcplayer=false", "-Dbuild-date=false", "-Dhtml-build=disabled",
                "-Dmanpage-build=disabled", "-Djavascript=disabled", "-Dlua=disabled", "-Dvulkan=disabled",
                "-Dlibarchive=enabled", "-Duchardet=enabled"],
    }
    for name in ("libplacebo", "mpv"):
        directory = root / f"build-{name}"
        command = [meson, "setup", str(directory), str(root / "source" / name),
                   "--prefix=" + str(prefix), "--libdir=lib", "--buildtype=release", "--wrap-mode=nofallback",
                   "-Dc_args=" + env["CFLAGS"], "-Dcpp_args=" + env["CXXFLAGS"],
                   "-Dobjc_args=" + env["OBJCFLAGS"]]
        if (directory / "meson-private/coredata.dat").exists():
            command.append("--reconfigure")
        stages.append(run_stage(root, name + "-configure", command + options[name], env))
        stages.append(run_stage(root, name + "-compile", [meson, "compile", "-C", str(directory), "--jobs=" + str(jobs)], env))
        # All install destinations are configured under the private prefix.
        installed = json.loads((directory / "meson-info/intro-installed.json").read_text())
        for destination in installed.values():
            for value in destination if isinstance(destination, list) else [destination]:
                if not Path(value).is_relative_to(prefix):
                    raise RuntimeError(f"Installation escapes private prefix: {value}")
        stages.append(run_stage(root, name + "-install", [meson, "install", "-C", str(directory), "--no-rebuild"], env))
    licenses = prefix / "share/licenses/native-media"
    licenses.mkdir(parents=True, exist_ok=True)
    for name, pin in pins.items():
        source = root / "source" / pin["destination"]
        destination = licenses / name
        destination.mkdir(exist_ok=True)
        for file in source.iterdir():
            if file.is_file() and (file.name.startswith(("LICENSE", "COPYING")) or file.name == "Copyright"):
                shutil.copyfile(file, destination / file.name)
    return {"schema": 1, "status": "compiled_and_installed", "native_render_abi": 1, "sources": pins,
            "patches": {patch: digest(SCRIPT / "patches" / patch) for patches in PATCHES.values() for patch in patches},
            "stages": stages, "sdk": env["SDKROOT"], "deployment_target": "12.0",
            "installed_files": {str(p.relative_to(prefix)): digest(p) for p in sorted(prefix.rglob("*"))
                                if p.is_file() and not p.is_symlink()},
            "limitations": ["Application/runtime playback qualification is separate.",
                            "Installed system dependency libraries are build inputs; packaging must include their notices and sources."]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=REPO / "artifacts/native-media/macos")
    parser.add_argument("--tools", type=Path, help="Directory containing meson/ninja Python build tools")
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--jobs", type=int, default=2)
    args = parser.parse_args()
    if platform.system() != "Darwin":
        parser.error("This dependency build targets macOS")
    if not 1 <= args.jobs <= 64:
        parser.error("--jobs must be between 1 and 64")
    root = args.root.resolve()
    if not root.is_relative_to(REPO / "artifacts"):
        parser.error("--root must be under this repository's artifacts directory")
    root.mkdir(parents=True, exist_ok=True)
    pins = source_inputs(root, args.offline)
    prepare(root, pins)
    if not args.prepare_only:
        result = build(root, pins, build_environment(root, args.tools.resolve() if args.tools else None), args.jobs)
        (root / "build-result.json").write_text(json.dumps(result, indent=2) + "\n")
        print("Native dependencies installed privately; playback qualification remains separate.")


if __name__ == "__main__":
    main()
