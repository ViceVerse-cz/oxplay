# Native Metal media dependencies

The macOS video presenter uses a private build of mpv and libplacebo, both
pinned to the IINA author's experimental native Metal branches. The application
retains mpv's demuxer, audio clock, stream callbacks, command/property API and
libass subtitle rendering. VideoToolbox frames are imported as Metal textures;
decoded video does not make a round trip through CPU pixel buffers.

Sources, exact revisions, archive SHA-256 checksums and licenses are recorded in
[`macos-sources.json`](../scripts/native-media/macos-sources.json). Upstream pins:

- [mpv `97179bce`](https://github.com/lhc70000/mpv/commit/97179bce7ed980c53647d6344916f632fe689e9e).
- [libplacebo `a9629687`](https://github.com/lhc70000/libplacebo/commit/a96296875a067887fc75d7b74a0a4292422c204d).

The stock mpv 0.41 render API provides OpenGL and software rendering only.
These custom dependencies extend it; installing a stock libmpv is insufficient.
The IINA fork's original Metal API presents directly to a `CAMetalLayer`.
Our patches instead permit rendering to a caller-owned texture, which Slint
composes with controls, popups, mini-player and the existing same-window PiP.

## Building

The build requires Xcode tools/SDK, Python 3.11 or later, Meson, Ninja, CMake,
pkg-config and the application's existing FFmpeg/libass/system dependencies.
No build stage modifies Homebrew or installs into a system prefix.

```sh
python3 scripts/native-media/macos.py --jobs 2
```

If Meson/Ninja live in a private tool environment, provide its executable
directory with `--tools`. The existing workspace tool environment can be used:

```sh
python3 scripts/native-media/macos.py \
  --tools artifacts/mpv-timers-tools-20260929/venv/bin --jobs 2
```

The script verifies downloaded archives before extracting them and reconstructs
the patched source from those archives on every invocation. Build output remains
separate and can be reused incrementally. After downloading the inputs once,
`--offline` performs the same build without fetching source. Missing or changed
cached archives cause an error. `--prepare-only` stops after source verification
and patch application.

Output goes under `artifacts/native-media/macos`:

- `downloads`: verified source archives, including pinned shader compiler and
  libplacebo template-parser submodules.
- `source`: reconstructed source and reviewed patches.
- `build-*`: private CMake/Meson output.
- `prefix`: installed headers/libraries and copied licenses.
- `build-result.json`: successful build commands, source/patch checksums,
  installed-file checksums and SDK/deployment information.

The native sources compile with deployment target macOS 12.0. This does not
establish the minimum supported OS of the resulting package: the current
Homebrew dependency inputs target macOS 27 and must be rebuilt for any lower
release target. The script builds private
SPIRV-Cross static archives and links them into libplacebo; Metal shaders use the
existing shaderc dependency. Vulkan/OpenGL libplacebo backends are disabled for
this macOS dependency build. Existing Homebrew libraries remain dependency
inputs and must be covered by the application's release source/notice workflow.

Build the application against this explicit private prefix. Its media build
script validates the installed ABI header and adds the private library rpath:

```sh
OXPLAY_NATIVE_MPV_PREFIX="$PWD/artifacts/native-media/macos/prefix" \
  cargo build --locked --release -p oxplay --features native-rendering
```

Do not substitute ambient Homebrew `libmpv` or set `DYLD_LIBRARY_PATH` to select
the renderer. For the qualifying binary, verify `otool -L` and loaded library
paths resolve to this prefix before starting measurements.

For a native macOS bundle, build this private stack on the release runner and
then invoke the packager with its explicit prefix:

```sh
python3 scripts/native-media/macos.py --jobs 2
python3 scripts/package_macos.py bundle --build --bundle-helpers \
  --native-mpv-prefix "$PWD/artifacts/native-media/macos/prefix" \
  --maximum-macos 26.0 --output dist/Oxplay.app
```

The macOS 26 ceiling is a release-runner check, not an assertion about a local
macOS 27 build. Homebrew dependencies from the local macOS 27 host require
macOS 27; rebuild against compatible runner dependencies for the older target.
The packager relocates the full private/native/helper closure, verifies the
installed ABI and hashes, and retains these pinned archives, applied patches,
licenses and build manifest as native-media evidence. See
[packaging details](packaging.md) for corresponding-source and signing limits.

## ABI

The installed `mpv/render_mtl.h` defines `OXPLAY_NATIVE_RENDER_ABI 1`.
These extensions belong to our pinned dependencies, not the stock mpv ABI.
All render operations remain serialized on the owning renderer thread.

| Parameter | ID | Data |
|---|---:|---|
| API type | 1 | NUL-terminated `metal` |
| Metal initialization | 23 | `mpv_metal_init_params *` |
| Metal texture target | 24 | `mpv_metal_texture *` |
| Native capacity query | 27 | `int *` output via `get_info` or `render` |

```c
typedef struct mpv_metal_init_params {
    void *layer;                 /* NULL selects offscreen mode */
    void *metal_device;          /* id<MTLDevice>, required offscreen */
    void *command_queue;         /* id<MTLCommandQueue>, required offscreen */
    void (*wakeup)(void *);       /* optional; Metal completion thread */
    void *wakeup_context;
} mpv_metal_init_params;

typedef struct mpv_metal_texture {
    void *texture;               /* caller-owned id<MTLTexture> */
    int width;
    int height;
} mpv_metal_texture;
```

The presenter supplies WGPU's own Metal device/queue. Targets use RGBA8Unorm,
with ShaderRead and RenderTarget usage, and matching dimensions in the range
1–4096. libplacebo validates that imported textures belong to the same device.
The initial offscreen output colorspace is sRGB SDR; the application does not
claim HDR output merely because the underlying fork contains HDR algorithms.

`mpv_render_context_get_info` with parameter 27 writes zero when capacity is
available and one when the three native submissions are occupied. The render
call checks capacity again before consuming mpv's queued frame. If busy, it
returns success, writes one to the optional parameter-27 output and leaves the
frame pending. A native completion wakes the host only if a capacity check
actually deferred work. The host retries through its event loop; no polling
timer or per-frame completion redraw is necessary.

The native render call submits work before returning. Queue ordering ensures
that the subsequent WGPU/Slint sample sees the completed write. The host owns
its texture references and retires them according to host GPU completion.
mpv's renderer owns imported VideoToolbox/CoreVideo references and retains
them until the last native command buffer using them completes.

## Patches and lifetime rules

[`common-mpv-gpu-next.patch`](../scripts/native-media/patches/common-mpv-gpu-next.patch)
adds platform hardware-device registration/mapping hooks and backend information
queries. It checks capacity before mpv consumes the pending frame and destroys
the platform device before freeing the common hardware-device registry. It
preserves libplacebo's monotonic frame-content signatures; allocation addresses
can be reused and cannot identify decoded image content for renderer caching.

[`macos-mpv-metal-texture.patch`](../scripts/native-media/patches/macos-mpv-metal-texture.patch)
adds caller-owned targets and maps VideoToolbox pixel-buffer planes through
CoreVideo's Metal texture cache. NV12 and 16-bit planar formats preserve
mpv/libplacebo's component ordering, color depth, range and chroma metadata.
The implementation follows the existing upstream `hwdec_vt_pl.m` importer.
libass and bitmap subtitles continue through the fork's existing GPU overlays.

[`macos-libplacebo-offscreen.patch`](../scripts/native-media/patches/macos-libplacebo-offscreen.patch)
bounds asynchronous native submissions and releases imported CoreVideo objects
at completion. Push constants use encoder-owned snapshots. Large inline vertex
and index data use immutable encoder-retained buffers; repeated draws within a
frame cannot overwrite data still read by the GPU. Host buffer/texture writes
use ordered staging transfers, including writes to shared storage.

Steady-state capacity checks and native rendering never wait for GPU completion.
Teardown drains outstanding native work and completion handlers before releasing
callback storage. Context destruction remains an explicit synchronization point,
as required to prevent callbacks after the host has freed its wake context.

## Qualification

The standalone headless check compiles against the installed header and asserts
ABI version 1, five initialization pointers, a 16-byte target and parameter IDs
23/24/27. It deliberately fills the three submissions behind a shared-event
gate, verifies exactly one deferred-work wake, renders 15 actual VideoToolbox
frames, requires at least ten distinct rendered RGB hashes from the moving
fixture, and checks no callback survives teardown. This catches stale content
even when render callbacks and playback time continue advancing.
The readback and teardown wait only in this diagnostic.

```sh
clang -fobjc-arc -I artifacts/native-media/macos/prefix/include \
  scripts/native-media/macos_smoke.m -L artifacts/native-media/macos/prefix/lib \
  -lmpv -Wl,-rpath,"$PWD/artifacts/native-media/macos/prefix/lib" \
  -framework Foundation -framework Metal \
  -o artifacts/native-media/macos/native-metal-smoke
ffmpeg -hide_banner -loglevel error -f lavfi -i testsrc2=size=320x180:rate=30 \
  -t 2 -c:v libx264 -pix_fmt yuv420p -y artifacts/native-media/macos/fixture.mp4
artifacts/native-media/macos/native-metal-smoke \
  "$PWD/artifacts/native-media/macos/fixture.mp4"
meson test -C artifacts/native-media/macos/build-libplacebo \
  --no-rebuild --print-errorlogs
```

The paused handoff diagnostic keeps the same Metal context through playing
video, paused audio-only, paused video, stop and another paused video load. Its
default API mode matches the app: no advanced-control parameter and no swap
reports. It requires fresh playback-restart events, decoded-video dimensions,
VideoToolbox and nonblack rendered content. Audio-only must have no decoded
video dimensions. Diagnostic polling/readback are confined to this executable.

```sh
clang -fobjc-arc -I artifacts/native-media/macos/prefix/include \
  scripts/native-media/macos_handoff_smoke.m -L artifacts/native-media/macos/prefix/lib \
  -lmpv -Wl,-rpath,"$PWD/artifacts/native-media/macos/prefix/lib" \
  -framework Foundation -framework Metal \
  -o artifacts/native-media/macos/native-metal-handoff-smoke
artifacts/native-media/macos/native-metal-handoff-smoke \
  "$PWD/artifacts/local-1080p60.mp4" \
  "$PWD/artifacts/account-integration/audio-only.wav"
```

Compilation, dependency tests and application playback checks are separate
evidence. A successful `build-result.json` proves private compilation/installation,
not UI feature parity or playback performance. Runtime qualification must verify
the loaded private libmpv/libplacebo paths, direct `videotoolbox` hardware decode,
color/orientation, subtitle placement and Off behavior, pause/seek/load barriers,
overlays, PiP, mini-player and ambient sampling before performance comparisons.

These upstream Metal branches are experimental and remain unmerged. The native
renderer should be promoted only with that runtime evidence and the exact source
and patch inventory used for the qualifying build.
