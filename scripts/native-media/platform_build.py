#!/usr/bin/env python3
"""Pinned source preparation shared by the native Linux/Windows media builds."""
from __future__ import annotations

import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request
import shlex
import ctypes
import sys

MPV_REVISION = "97179bce7ed980c53647d6344916f632fe689e9e"
MPV_URL = f"https://codeload.github.com/lhc70000/mpv/tar.gz/{MPV_REVISION}"
MPV_SHA256 = "0aa760258c3ad07b5339bb8998ea0f93b53357234eb49836f0caaa9da15e96e7"
PLACEBO_VERSION = "7.360.1"
PLACEBO_URL = f"https://codeload.github.com/haasn/libplacebo/tar.gz/refs/tags/v{PLACEBO_VERSION}"
PLACEBO_SHA256 = "d05fdf90bea2f629eaa2d115e909fd356388ac639e54f77b87a018a6d76224bd"
FFMPEG_VERSION = "9.0.2"
FFMPEG_URL = f"https://ffmpeg.org/releases/ffmpeg-{FFMPEG_VERSION}.tar.xz"
FFMPEG_SHA256 = "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e"
PATCHES = Path(__file__).resolve().parent / "patches"
AUXILIARY = {
    # Exact v7.360.1 gitlink. The Ubuntu 24.04 loader is sufficient at runtime,
    # but libplacebo's compilation requires Vulkan 1.4 header declarations.
    "Vulkan-Headers": ("KhronosGroup/Vulkan-Headers", "450bd2232225d6c7728a4108055ac2e37cef6475", "26df9841c30806a994e2fdf42f7c87bcb1ced9db9a06033469123939fb3fa075"),
    "jinja": ("pallets/jinja", "15206881c006c79667fe5154fe80c01c65410679", "b88a20dcc2e34072fcf4159325bc6c34cd4b29a81a8b83d15d2f28ba561da296"),
    "markupsafe": ("pallets/markupsafe", "297fc8e356e6836a62087949245d09a28e9f1b13", "da7c010c9c81a66ac73036558c1fcb6212b50482f43211cd1254035b94f82414"),
    "fast_float": ("fastfloat/fast_float", "97b54ca9e75f5303507699d27c6b4f4efe4641a1", "2b132274539286e41f37857cac22aa8441d21bd86d55de825a3342b149f66801"),
}


def run(args: list[str], *, cwd: Path | None = None, env=None) -> None:
    print("+ " + " ".join(args), flush=True)
    subprocess.run(args, cwd=cwd, env=env, check=True, timeout=1800)


def fetch_source(url: str, expected: str, destination: Path, archive_path: Path | None = None) -> None:
    # Authenticate the complete archive before parsing or extracting any member.
    request = urllib.request.Request(url, headers={"User-Agent": "Oxplay-native-media/1"})
    with urllib.request.urlopen(request, timeout=120) as response:
        if not response.url.startswith("https://"):
            raise RuntimeError("Source redirected outside HTTPS")
        archive = response.read(64 * 1024 * 1024 + 1)
    if len(archive) > 64 * 1024 * 1024:
        raise RuntimeError("Source archive exceeds 64 MiB")
    if hashlib.sha256(archive).hexdigest() != expected:
        raise RuntimeError(f"Source SHA-256 mismatch: {url}")
    if archive_path:
        archive_path.parent.mkdir(parents=True, exist_ok=True)
        archive_path.write_bytes(archive)
    if destination.exists() and any(destination.iterdir()):
        raise RuntimeError(f"Source destination is not empty: {destination}")
    destination.mkdir(exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:*") as tar:
        for member in tar.getmembers():
            pieces = Path(member.name).parts
            if len(pieces) < 2:
                continue
            member.name = str(Path(*pieces[1:]))
            if member.issym() or member.islnk():
                # Source archives should not need escaping symlink ownership.
                target = Path(member.linkname)
                if target.is_absolute() or ".." in target.parts:
                    raise RuntimeError(f"Unsafe source link: {member.name}")
            tar.extract(member, destination, filter="data")


def prepare(work: Path, platform_patch: str) -> tuple[Path, Path, list[dict]]:
    if work.exists():
        raise RuntimeError(f"Build workspace must be new: {work}")
    work.mkdir(parents=True)
    mpv = work / "mpv"
    placebo = work / "libplacebo"
    archives = work / "source-archives"
    fetch_source(MPV_URL, MPV_SHA256, mpv, archives / "mpv.tar.gz")
    fetch_source(PLACEBO_URL, PLACEBO_SHA256, placebo, archives / "libplacebo.tar.gz")
    for name, (repository, revision, checksum) in AUXILIARY.items():
        fetch_source(f"https://codeload.github.com/{repository}/tar.gz/{revision}",
                     checksum, placebo / "3rdparty" / name, archives / f"{name}.tar.gz")
    applied = []
    for name in ("common-mpv-gpu-next.patch", platform_patch):
        patch = PATCHES / name
        if not patch.is_file():
            raise RuntimeError(f"Required native source patch is missing: {patch}")
        run(["git", "apply", "--check", str(patch)], cwd=mpv)
        run(["git", "apply", str(patch)], cwd=mpv)
        applied.append({"file": name, "sha256": hashlib.sha256(patch.read_bytes()).hexdigest()})
    return mpv, placebo, applied


def build_environment(prefix: Path, dependencies: Path | None) -> dict[str, str]:
    env = os.environ.copy()
    roots = [prefix] + ([dependencies] if dependencies else [])
    directories = [str(root / folder) for root in roots for folder in ("lib/pkgconfig", "lib64/pkgconfig", "share/pkgconfig")]
    env["PKG_CONFIG_PATH"] = os.pathsep.join(directories + [env.get("PKG_CONFIG_PATH", "")])
    return env


def provenance(prefix: Path, platform: str, applied: list[dict], env: dict,
               *, work: Path, runtime_dependencies: list[dict] | None = None,
               private_ffmpeg: bool = False) -> None:
    # Record every resolved native dependency instead of claiming a reproducible
    # binary from source pins alone. The SDK/compiler/system packages are external.
    dependencies = {}
    for package in ("libavcodec", "libavformat", "libavutil", "libass", "libplacebo", "vulkan", "shaderc", "spirv-cross-c-shared", "libva", "lua", "luajit"):
        result = subprocess.run(["pkg-config", "--modversion", package], env=env, text=True, capture_output=True)
        if result.returncode == 0:
            dependencies[package] = result.stdout.strip()
    value = {"schema": 1, "native_render_abi": 1, "platform": platform, "mpv": {"revision": MPV_REVISION, "url": MPV_URL, "sha256": MPV_SHA256},
             "libplacebo": {"version": PLACEBO_VERSION, "url": PLACEBO_URL, "sha256": PLACEBO_SHA256},
             "patches": applied, "resolved_dependencies": dependencies,
             "auxiliary_sources": {name: {"repository": repository, "revision": revision, "sha256": checksum}
                                   for name, (repository, revision, checksum) in AUXILIARY.items()},
             "runtime_dlls_sha256": {dll.name: hashlib.sha256(dll.read_bytes()).hexdigest() for dll in sorted(prefix.glob("*.dll"))},
             "runtime_dependencies": runtime_dependencies or [],
             "compiler": tool_version(os.environ.get("CC", "cc")),
             "meson": tool_version("meson"), "pkg_config": tool_version("pkg-config"),
             "source_bundle": "share/oxplay-native/sources.tar.gz",
             "source_bundle_sha256": hashlib.sha256((prefix / "share/oxplay-native/sources.tar.gz").read_bytes()).hexdigest()}
    if private_ffmpeg:
        value["ffmpeg"] = {"version": FFMPEG_VERSION, "url": FFMPEG_URL, "sha256": FFMPEG_SHA256}
    value["meson_build_options"] = {}
    for name in ("mpv", "libplacebo"):
        options = work / name / "build/meson-info/intro-buildoptions.json"
        value["meson_build_options"][name] = json.loads(options.read_text())
    (prefix / "ci-source.txt").write_text(f"Oxplay native ABI 1: mpv {MPV_REVISION}; libplacebo {PLACEBO_VERSION}\n")
    (prefix / "ci-build-options.json").write_text(json.dumps(value["meson_build_options"], indent=2) + "\n")
    (prefix / "native-media-provenance.json").write_text(json.dumps(value, indent=2) + "\n")
    commands = (["pacman", "-Q"], ["dpkg-query", "-W", "-f=${Package}=${Version}\\n"],
                ["rpm", "-qa", "--qf", "%{NAME}=%{VERSION}-%{RELEASE}.%{ARCH}\\n"])
    for command in commands:
        if shutil.which(command[0]):
            result = subprocess.run(command, capture_output=True, text=True, check=True)
            (prefix / "share/oxplay-native/host-packages.txt").write_text(
                "\n".join(sorted(result.stdout.splitlines())) + "\n")
            break
    inventory = []
    for path in sorted(prefix.rglob("*")):
        if path.is_file():
            inventory.append({"path": path.relative_to(prefix).as_posix(),
                              "bytes": path.stat().st_size,
                              "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                              "symlink": os.readlink(path) if path.is_symlink() else None})
    (prefix / "native-media-inventory.json").write_text(json.dumps(inventory, indent=2) + "\n")


def tool_version(command: str) -> str:
    result = subprocess.run([*shlex.split(command), "--version"], text=True, capture_output=True, check=True)
    return result.stdout.splitlines()[0]


def source_bundle(prefix: Path, work: Path, applied: list[dict]) -> None:
    """Ship original authenticated sources, all modifications and build recipes."""
    doc = prefix / "share/oxplay-native"
    doc.mkdir(parents=True, exist_ok=True)
    with tarfile.open(doc / "sources.tar.gz", "w:gz") as archive:
        for path in sorted((work / "source-archives").iterdir()):
            archive.add(path, arcname="upstream/" + path.name)
        for patch in applied:
            archive.add(PATCHES / patch["file"], arcname="patches/" + patch["file"])
        for name in ("platform_build.py", "linux.py", "windows.py", "check_abi.c"):
            archive.add(PATCHES.parent / name, arcname="build/" + name)
        archive.add(PATCHES.parents[2] / "packaging/linux/install-build-deps.sh",
                    arcname="build/install-build-deps-linux.sh")
    for name in ("mpv", "libplacebo", "ffmpeg"):
        for pattern in ("COPYING*", "LICENSE*", "Copyright*", "DOCS/copyright.md"):
            for path in sorted((work / name).glob(pattern)):
                if path.is_file():
                    target = doc / "licenses" / name / path.name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(path, target)
    sources = {"schema": 1, "native_render_abi": 1,
               "platform": "windows-d3d11" if any("windows-" in item["file"] for item in applied) else "linux-vulkan",
               "bundle": "sources.tar.gz",
               "bundle_sha256": hashlib.sha256((doc / "sources.tar.gz").read_bytes()).hexdigest(),
               "upstream_archives": [{"file": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
                                     for path in sorted((work / "source-archives").iterdir())],
               "patches": applied, "runtime_sources": []}
    (doc / "sources.json").write_text(json.dumps(sources, indent=2) + "\n")


def check_installed_abi(prefix: Path, work: Path, env: dict, *, vulkan: bool) -> None:
    """Actually compile the installed C ABI assertions and load its client API."""
    args = [*shlex.split(env.get("CC", "cc")), "-std=c11", "-Werror", "-c",
            str(PATCHES.parent / "check_abi.c"), "-I" + str(prefix / "include"),
            "-o", str(work / "native-render-abi.o")]
    if vulkan:
        args += ["-DCHECK_VULKAN", "-I" + str(work / "libplacebo/3rdparty/Vulkan-Headers/include"),
                 *shlex.split(subprocess.check_output(
            ["pkg-config", "--cflags", "vulkan"], text=True, env=env))]
    run(args, env=env)
    library = prefix / ("lib/libmpv.so.2" if vulkan else "libmpv-2.dll")
    if vulkan:
        run([sys.executable, "-c", "import ctypes,sys; m=ctypes.CDLL(sys.argv[1]); "
             "m.mpv_client_api_version.restype=ctypes.c_ulong; "
             "assert m.mpv_client_api_version() >= ((2 << 16) | 5)", str(library)], env=env)
        return
    # Python's DLL directory lease must remain alive through client API loading.
    lease = os.add_dll_directory(str(prefix)) if os.name == "nt" else None
    try:
        media = ctypes.CDLL(str(library))
        media.mpv_client_api_version.restype = ctypes.c_ulong
        api = media.mpv_client_api_version()
        if (api >> 16, api & 0xffff) < (2, 5):
            raise RuntimeError("Built native client API is below 2.5")
    finally:
        if lease:
            lease.close()


def require_tools() -> None:
    for tool in ("git", "meson", "ninja", "pkg-config"):
        if not shutil.which(tool):
            raise RuntimeError(f"Required build tool is missing: {tool}")
