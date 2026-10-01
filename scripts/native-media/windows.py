#!/usr/bin/env python3
"""Build the pinned D3D11 libmpv using a prepared Windows native toolchain."""
import argparse
import json
from pathlib import Path
import shutil
import sys
import os
import re
import hashlib
import subprocess
import tarfile
import urllib.request
from functools import lru_cache

from platform_build import (MPV_REVISION, PLACEBO_VERSION, build_environment,
                            prepare, provenance, require_tools, run,
                            source_bundle, check_installed_abi, fetch_source,
                            FFMPEG_URL, FFMPEG_SHA256, FFMPEG_VERSION, check_software_av1)


# auto_features=disabled must not disable the Windows OS backend. POSIX
# pthread IDs are incompatible with the unconditionally selected Win32 timer.
MPV_MESON_OPTIONS = [
    "-Dauto_features=disabled", "-Dwin32-threads=enabled",
    # Upstream's encoding test registers av://lavfi:testsrc through libavdevice.
    "-Dlibavdevice=enabled", "-Dlibmpv=true", "-Dcplayer=true", "-Dtests=true",
    "-Dbuild-date=false", "-Dgl=disabled", "-Dshaderc=enabled", "-Dspirv-cross=enabled",
    "-Dd3d11=enabled", "-Dd3d-hwaccel=enabled", "-Dwasapi=enabled",
    "-Dlua=luajit", "-Dmanpage-build=disabled",
]


def windows_build_environment(prefix: Path, dependencies: Path) -> dict[str, str]:
    env = build_environment(prefix, dependencies)
    # Meson only infers dependent DLL directories for directly linked tests.
    # libmpv-lifetime uses LoadLibraryW, so its private FFmpeg/libplacebo DLLs
    # must also be reachable explicitly, ahead of any stock SDK DLLs.
    env["PATH"] = os.pathsep.join([str(prefix / "bin"), str(dependencies / "bin"), env.get("PATH", "")])
    return env


@lru_cache(maxsize=None)
def msys_executable(name: str) -> str:
    """Bypass CreateProcess's system-directory lookup (notably WSL bash.exe)."""
    cygpath = shutil.which("cygpath")
    if not cygpath:
        raise RuntimeError("MSYS2 cygpath is required for the Windows native source build")
    path = subprocess.check_output([cygpath, "-w", f"/usr/bin/{name}.exe"], text=True).strip()
    if not Path(path).is_file():
        raise RuntimeError(f"Required MSYS2 executable is missing: {path}")
    return path


def imported_dlls(image: Path) -> list[str]:
    dump = subprocess.check_output(["objdump", "-p", str(image)], text=True)
    return sorted(set(re.findall(r"DLL Name:\s*(\S+)", dump, flags=re.IGNORECASE)))


def dll_closure(prefix: Path, dependencies: Path) -> list[dict]:
    """Copy only transitive PE imports; Windows platform DLLs stay on the OS."""
    native = {path.name.casefold(): path for path in (prefix / "bin").glob("*.dll")}
    external = {path.name.casefold(): path for path in (dependencies / "bin").glob("*.dll")}
    roots = [path for path in native.values() if re.fullmatch(r"(?:lib)?mpv-2\.dll", path.name, re.I)]
    if len(roots) != 1:
        raise RuntimeError("Expected exactly one installed native mpv DLL")
    # Keep the original DLL basename: the generated MSVC import library binds it.
    pending = roots.copy()
    copied: dict[str, Path] = {}
    records = []
    system = Path(os.environ["SystemRoot"]) / "System32"
    while pending:
        source = pending.pop()
        key = source.name.casefold()
        if key in copied:
            continue
        copied[key] = source
        target_name = "libmpv-2.dll" if source in roots else source.name
        shutil.copy2(source, prefix / target_name)
        records.append({"dll": target_name, "origin": "private" if key in native else "msys2-ucrt64",
                        "sha256": hashlib.sha256(source.read_bytes()).hexdigest()})
        for name in imported_dlls(source):
            key = name.casefold()
            target = native.get(key) or external.get(key)
            if target:
                pending.append(target)
            elif key.startswith(("api-ms-win-", "ext-ms-win-")) or (system / name).is_file():
                continue
            else:
                raise RuntimeError(f"Unresolved native Windows DLL import: {source.name} -> {name}")
    # FFmpeg may itself depend on the distribution's stock libplacebo. Reject a
    # different SONAME instead of silently shipping stock output alongside ours.
    placebos = [name for name in copied if "placebo" in name]
    if any(name not in native for name in placebos):
        raise RuntimeError("FFmpeg pulls a different system libplacebo ABI; build FFmpeg without libplacebo")
    return records


def source_recipe_url(base: str, version: str) -> str:
    # MSYS2 mirror filenames preserve pacman epochs using ~ instead of :.
    return f"https://mirror.msys2.org/mingw/sources/{base}-{version.replace(':', '~')}.src.tar.zst"


def download_source_recipe(url: str, target: Path, max_bytes: int = 512 * 1024 * 1024) -> str:
    # GCC's source package is over 100 MiB. Stream authenticated HTTPS inputs
    # with a finite bound instead of holding the entire source archive in RAM.
    digest = hashlib.sha256()
    try:
        with urllib.request.urlopen(url, timeout=120) as response:
            if not response.url.startswith("https://"):
                raise RuntimeError("Runtime source redirected outside HTTPS")
            size = 0
            with target.open("wb") as output:
                while chunk := response.read(1024 * 1024):
                    size += len(chunk)
                    if size > max_bytes:
                        raise RuntimeError(f"Runtime source archive exceeds {max_bytes} bytes")
                    output.write(chunk)
                    digest.update(chunk)
    except Exception:
        target.unlink(missing_ok=True)
        raise
    return digest.hexdigest()


def msys2_package_record(dll: Path) -> dict:
    unix = subprocess.check_output(["cygpath", "-u", str(dll)], text=True).strip()
    name = subprocess.check_output(["pacman", "-Qqo", unix], text=True).strip()
    info = subprocess.check_output(["pacman", "-Qi", name], text=True)
    fields = dict(re.findall(r"^([^:\n]+?)\s*:\s*(.*)$", info, flags=re.MULTILINE))
    version = fields["Version"]
    local = Path(subprocess.check_output(["cygpath", "-m", "/var/lib/pacman/local"], text=True).strip())
    desc = (local / f"{name}-{version}" / "desc").read_text()
    match = re.search(r"%BASE%\n([^\n]+)", desc)
    base = match[1] if match else re.sub(r"^mingw-w64-ucrt-x86_64-", "mingw-w64-", name)
    return {"package": name, "version": version, "base": base,
            "licenses": fields.get("Licenses", "unknown"),
            "upstream": fields.get("URL", ""),
            "source_recipe_url": source_recipe_url(base, version)}


def collect_runtime_sources(prefix: Path, dependencies: Path, records: list[dict], work: Path) -> None:
    """Retrieve upstream-inclusive corresponding sources through exact recipes.

    MSYS2's *.src.tar.zst files alone contain PKGBUILDs/patches, often omitting
    upstream archives. makepkg --allsource downloads and verifies those inputs
    without compiling them, producing an actual source closure per package base.
    """
    packages = {}
    for record in records:
        if record["origin"] == "msys2-ucrt64":
            package = msys2_package_record(dependencies / "bin" / record["dll"])
            record["package"] = package
            packages.setdefault((package["base"], package["version"]), package)
    destination = prefix / "share/oxplay-native/runtime-sources"
    destination.mkdir(parents=True)
    for (base, version), package in sorted(packages.items()):
        directory = work / "runtime-sources" / base
        directory.mkdir(parents=True)
        recipe = directory / "recipe.src.tar.zst"
        package["source_recipe_sha256"] = download_source_recipe(package["source_recipe_url"], recipe)
        run([msys_executable("tar"), "--force-local", "-xf", str(recipe), "-C", str(directory)])
        buildfiles = list(directory.rglob("PKGBUILD"))
        if len(buildfiles) != 1:
            raise RuntimeError(f"Expected one PKGBUILD for {base}")
        unix = subprocess.check_output(["cygpath", "-u", str(buildfiles[0].parent)], text=True).strip()
        run([msys_executable("bash"), "-lc", 'cd "$1" && exec /usr/bin/makepkg --allsource --nodeps --noconfirm --skippgpcheck', "source", unix])
        sources = list(buildfiles[0].parent.glob("*.src.tar.*"))
        if len(sources) != 1:
            raise RuntimeError(f"Expected one upstream-inclusive source package for {base}")
        target = destination / sources[0].name
        shutil.copy2(sources[0], target)
        package["source_archive"] = target.relative_to(prefix).as_posix()
        package["source_archive_sha256"] = hashlib.sha256(target.read_bytes()).hexdigest()
        # Copy the distribution's actual runtime license texts, not just IDs.
        unix_files = subprocess.check_output(["pacman", "-Qlq", package["package"]], text=True).splitlines()
        for unix_file in unix_files:
            if "/share/licenses/" in unix_file and not unix_file.endswith("/"):
                source = Path(subprocess.check_output(["cygpath", "-m", unix_file], text=True).strip())
                if source.is_file():
                    target = prefix / "share/oxplay-native/licenses/msys2" / package["package"] / source.name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, target)
    manifest = prefix / "share/oxplay-native/sources.json"
    data = json.loads(manifest.read_text())
    data["runtime_sources"] = list(packages.values())
    data["runtime_source_verification"] = "Exact installed MSYS2 recipes; makepkg upstream checksums verified; OpenPGP checks disabled"
    manifest.write_text(json.dumps(data, indent=2) + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--dependencies", type=Path, required=True, help="Prepared FFmpeg/libass/shaderc/SDK pkg-config prefix")
    parser.add_argument("--native-file", type=Path, help="Meson native toolchain file (MSVC/clang-cl or MinGW)")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--build-ffmpeg", action="store_true", help="Build pinned full FFmpeg with D3D11VA, avoiding a system libplacebo dependency")
    parser.add_argument("--collect-runtime-sources", action="store_true", help="Retrieve upstream-inclusive sources and licenses for every bundled MSYS2 DLL")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--plan", action="store_true")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    if args.plan:
        print(json.dumps({"mpv": MPV_REVISION, "libplacebo": PLACEBO_VERSION, "api": "d3d11", "ui_api": "dx12", "mpv_options": MPV_MESON_OPTIONS, "patches": ["common-mpv-gpu-next.patch", "windows-d3d11-interop.patch"]}, indent=2))
        return 0
    if sys.platform != "win32":
        parser.error("Native D3D11 media build requires Windows; --plan is portable")
    prefix = args.prefix.resolve()
    if prefix.exists():
        parser.error("--prefix must name a new directory")
    require_tools()
    mpv, placebo, applied = prepare(args.work_dir.resolve(), "windows-d3d11-interop.patch")
    if args.prepare_only:
        return 0
    env = windows_build_environment(prefix, args.dependencies.resolve())
    if args.build_ffmpeg:
        ffmpeg = args.work_dir.resolve() / "ffmpeg"
        fetch_source(FFMPEG_URL, FFMPEG_SHA256, ffmpeg,
                     args.work_dir.resolve() / "source-archives" / f"ffmpeg-{FFMPEG_VERSION}.tar.xz")
        unix_prefix = subprocess.check_output(["cygpath", "-u", str(prefix)], text=True).strip()
        run([msys_executable("bash"), str(ffmpeg / "configure"), "--prefix=" + unix_prefix,
             "--enable-shared", "--disable-static", "--disable-programs", "--disable-doc",
             "--enable-gpl", "--enable-gnutls", "--enable-libdav1d", "--enable-w32threads", "--enable-d3d11va"], cwd=ffmpeg, env=env)
        run([msys_executable("make"), "-j", str(args.jobs)], cwd=ffmpeg, env=env)
        run([msys_executable("make"), "install"], cwd=ffmpeg, env=env)
    native = ["--native-file", str(args.native_file.resolve())] if args.native_file else []
    run(["meson", "setup", str(placebo / "build"), str(placebo), f"--prefix={prefix}", "--libdir=lib", "--buildtype=release", "--wrap-mode=nofallback",
         "-Dvulkan=disabled", "-Dopengl=disabled", "-Dd3d11=enabled", "-Dshaderc=enabled", "-Dglslang=disabled", "-Dlcms=disabled", "-Ddovi=disabled", "-Dtests=false", "-Ddemos=false", *native], env=env)
    run(["meson", "compile", "-C", str(placebo / "build"), "-j", str(args.jobs)], env=env)
    run(["meson", "install", "-C", str(placebo / "build"), "--no-rebuild"], env=env)
    run(["meson", "setup", str(mpv / "build"), str(mpv), f"--prefix={prefix}", "--libdir=lib", "--buildtype=release", "--wrap-mode=nofallback",
         *MPV_MESON_OPTIONS, *native], env=env)
    run(["meson", "compile", "-C", str(mpv / "build"), "-j", str(args.jobs)], env=env)
    run(["meson", "test", "-C", str(mpv / "build"), "--print-errorlogs", "--timeout-multiplier=2"], env=env)
    run(["meson", "install", "-C", str(mpv / "build"), "--no-rebuild"], env=env)
    # MPV_DIR / OXPLAY_NATIVE_MPV_PREFIX use the existing application SDK layout.
    imports = list((prefix / "lib").glob("*mpv*.lib"))
    if imports:
        shutil.copy2(imports[0], prefix / "mpv.lib")
    else:
        imports = list((prefix / "lib").glob("*mpv*.dll.a"))
        if len(imports) != 1:
            raise RuntimeError("Expected one installed mpv import library")
        shutil.copy2(imports[0], prefix / "libmpv.dll.a")
    records = dll_closure(prefix, args.dependencies.resolve())
    if not any((prefix / name).is_file() for name in ("libmpv-2.dll", "mpv-2.dll")):
        raise RuntimeError("Installed mpv runtime DLL is missing")
    check_installed_abi(prefix, args.work_dir.resolve(), env, vulkan=False)
    check_software_av1(prefix, args.work_dir.resolve(), env, windows=True)
    source_bundle(prefix, args.work_dir.resolve(), applied)
    if args.collect_runtime_sources:
        collect_runtime_sources(prefix, args.dependencies.resolve(), records, args.work_dir.resolve())
    provenance(prefix, "windows-d3d11", applied, env, work=args.work_dir.resolve(), runtime_dependencies=records,
               private_ffmpeg=args.build_ffmpeg)
    print(f"Set OXPLAY_NATIVE_MPV_PREFIX={prefix}; installed header: include/mpv/render_d3d11.h")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
