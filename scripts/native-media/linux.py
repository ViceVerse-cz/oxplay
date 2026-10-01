#!/usr/bin/env python3
"""Build the pinned Vulkan libmpv into a private Linux prefix (no root required)."""
import argparse
import json
from pathlib import Path
import sys

from platform_build import (MPV_REVISION, PLACEBO_VERSION, build_environment,
                            prepare, provenance, require_tools, run, fetch_source,
                            FFMPEG_URL, FFMPEG_SHA256, FFMPEG_VERSION,
                            source_bundle, check_installed_abi, check_software_av1)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, required=True)
    parser.add_argument("--work-dir", type=Path, required=True)
    parser.add_argument("--dependencies", type=Path, help="Existing FFmpeg/libass/shaderc/SDK pkg-config prefix")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--build-ffmpeg", action="store_true", help="Build pinned full FFmpeg into the private prefix (required for release packages)")
    parser.add_argument("--prepare-only", action="store_true")
    parser.add_argument("--plan", action="store_true", help="Print source/backend selection without network or compilation")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    if args.plan:
        print(json.dumps({"mpv": MPV_REVISION, "libplacebo": PLACEBO_VERSION, "api": "vulkan", "patches": ["common-mpv-gpu-next.patch", "linux-vulkan-interop.patch"]}, indent=2))
        return 0
    if sys.platform != "linux":
        parser.error("Native Vulkan media build requires Linux; --plan is portable")
    prefix = args.prefix.resolve()
    if prefix.exists():
        parser.error("--prefix must name a new directory")
    require_tools()
    mpv, placebo, applied = prepare(args.work_dir.resolve(), "linux-vulkan-interop.patch")
    if args.prepare_only:
        return 0
    env = build_environment(prefix, args.dependencies.resolve() if args.dependencies else None)
    env["LD_LIBRARY_PATH"] = str(prefix / "lib") + (":" + env["LD_LIBRARY_PATH"] if env.get("LD_LIBRARY_PATH") else "")
    if args.build_ffmpeg:
        ffmpeg = args.work_dir.resolve() / "ffmpeg"
        fetch_source(FFMPEG_URL, FFMPEG_SHA256, ffmpeg,
                     args.work_dir.resolve() / "source-archives" / f"ffmpeg-{FFMPEG_VERSION}.tar.xz")
        # Built-in H.264/HEVC/AAC/Opus decoders are retained; Fedora's restricted
        # system ffmpeg-free cannot supply the release's codec coverage.
        run([str(ffmpeg / "configure"), f"--prefix={prefix}", "--libdir=" + str(prefix / "lib"),
             "--enable-shared", "--disable-static", "--disable-programs", "--disable-doc",
             "--enable-gpl", "--enable-gnutls", "--enable-libdav1d", "--enable-vaapi", "--enable-libdrm"], cwd=ffmpeg, env=env)
        run(["make", "-j", str(args.jobs)], cwd=ffmpeg, env=env)
        run(["make", "install"], cwd=ffmpeg, env=env)
    run(["meson", "setup", str(placebo / "build"), str(placebo), f"--prefix={prefix}", "--libdir=lib", "--buildtype=release", "--wrap-mode=nofallback",
         "-Dvulkan=enabled", "-Dopengl=disabled", "-Dd3d11=disabled", "-Dshaderc=enabled", "-Dglslang=disabled", "-Dlcms=disabled", "-Ddovi=disabled", "-Dtests=false", "-Ddemos=false"], env=env)
    run(["meson", "compile", "-C", str(placebo / "build"), "-j", str(args.jobs)], env=env)
    run(["meson", "install", "-C", str(placebo / "build"), "--no-rebuild"], env=env)
    run(["meson", "setup", str(mpv / "build"), str(mpv), f"--prefix={prefix}", "--libdir=lib", "--buildtype=release", "--wrap-mode=nofallback",
         # The upstream encoding test uses av://lavfi:testsrc; libavfilter is
         # mandatory, but registering lavfi inputs requires libavdevice too.
         "-Dauto_features=disabled", "-Dlibavdevice=enabled", "-Dlibmpv=true", "-Dcplayer=true", "-Dtests=true", "-Dbuild-date=false", "-Dgl=disabled", "-Dvulkan=enabled", "-Dvaapi=enabled", "-Dvaapi-drm=enabled", "-Ddrm=enabled",
         # Slint owns X11/Wayland windows; standalone mpv surface backends are
         # unused by the offscreen API and demand Wayland newer than Ubuntu24.
         "-Dx11=disabled", "-Dwayland=disabled", "-Dalsa=enabled", "-Dpulse=enabled", "-Dlua=luajit", "-Dmanpage-build=disabled"], env=env)
    run(["meson", "compile", "-C", str(mpv / "build"), "-j", str(args.jobs)], env=env)
    run(["meson", "test", "-C", str(mpv / "build"), "--print-errorlogs", "--timeout-multiplier=2"], env=env)
    run(["meson", "install", "-C", str(mpv / "build"), "--no-rebuild"], env=env)
    check_installed_abi(prefix, args.work_dir.resolve(), env, vulkan=True)
    check_software_av1(prefix, args.work_dir.resolve(), env, windows=False)
    source_bundle(prefix, args.work_dir.resolve(), applied)
    provenance(prefix, "linux-vulkan", applied, env, work=args.work_dir.resolve(), private_ffmpeg=args.build_ffmpeg)
    print(f"Set OXPLAY_NATIVE_MPV_PREFIX={prefix}, PKG_CONFIG_PATH={prefix}/lib/pkgconfig and LD_LIBRARY_PATH={prefix}/lib")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
