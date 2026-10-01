# Native media dependencies on Linux and Windows

The `native-rendering` application feature requires the private mpv ABI shipped
by these recipes. Stock mpv 0.41 provides the OpenGL/software render API and
cannot replace these binaries. Audio, demuxing, seek, subtitle rendering,
screenshots and playback controls remain in libmpv.

| Platform | Media renderer / native decoder | UI renderer | GPU transfer |
| --- | --- | --- | --- |
| Linux X11 or Wayland | libplacebo Vulkan / matching-adapter VAAPI | WGPU Vulkan | Same VkDevice image and timeline semaphores; VAAPI imports DMA_BUF directly |
| Windows 10 or later | mpv D3D11 / D3D11VA | WGPU DX12 | Persistent NT-handle shared textures and shared GPU fences on the same adapter LUID |

Neither path reads decoded hardware frames back to CPU memory. Software
decoding can remain available for unsupported codecs/drivers; it uploads decoded
planes through the existing renderer. It should be reported as software decoding
in performance results. D3D11VA may perform a GPU texture copy from decoder
storage to a shader-readable surface; this does not transfer a frame through CPU
memory. Do not force mpv's unsafe `d3d11va-zero-copy` option.

The private Linux and Windows FFmpeg builds enable dav1d so AV1 playback also
works without a hardware AV1 decoder. Each build must decode the authenticated
synthetic AV1 fixture through explicit software `libdav1d`, producing eight
distinct 64×64 frames. The bounded probe and its fixture ship with the build
sources; its result and fixture checksum are recorded in the media provenance.
This check preserves software fallback without changing automatic hardware
decoder selection or the app's H.264 format preference.

## Source inputs and status

Both recipes use [lhc70000/mpv at 97179bce](https://github.com/lhc70000/mpv/tree/97179bce7ed980c53647d6344916f632fe689e9e)
and stock [libplacebo v7.360.1](https://github.com/haasn/libplacebo/tree/v7.360.1).
Their HTTPS archives, plus Jinja, MarkupSafe and fast_float sources, are SHA-256
verified before extraction. `platform_build.py` contains the complete source
pins. Patches apply after `common-mpv-gpu-next.patch`.

The Windows renderer derives from [mpv PR 17764, commit 87316d31](https://github.com/mpv-player/mpv/pull/17764).
This change is still an upstream proposal, not a released render API. Our patch
adds fence synchronization, validates shared textures, makes size queries free of
GPU submissions, and releases the temporary texture wrapper after each frame.
It preserves the existing mpv D3D11 renderer and D3D11VA decoder interop.

The Linux renderer fixes the fork's queue-family/index confusion and replaces
its synchronous `pl_gpu_finish` frame path with the documented libplacebo
[release/hold interoperability API](https://github.com/haasn/libplacebo/blob/v7.360.1/src/include/libplacebo/vulkan.h).
It does not adopt [closed RFC 18258](https://github.com/mpv-player/mpv/pull/18258)
or rely on the unpublished prototype claimed in
[closed issue 18343](https://github.com/mpv-player/mpv/issues/18343).
These recipes therefore produce an application-specific dependency, not an
upstream-supported native libmpv ABI.

## Building

Use Python 3.12 or later, Git, Meson, Ninja and pkg-config. Prefix and work
directories must be new; builds never install into the system prefix. `--plan`
prints the selection without downloads or compilation. `--prepare-only` downloads
and verifies sources and applies patches without compiling. Compiler, SDK and
FFmpeg/dependency packages are external inputs; source pins alone do not make
the resulting binary reproducible. Each build records source/patch checksums,
resolved pkg-config versions and Windows DLL checksums in
`native-media-provenance.json`.

On Linux, prepare FFmpeg development libraries meeting mpv's Meson minimums,
libass, LuaJIT, shaderc, Vulkan 1.3 headers/loader, libva/DRM, ALSA, PulseAudio,
X11 and Wayland development packages. The selected dependencies must support
the same architecture and include the intended hardware decoders. The build
disables OpenGL. Run:

```sh
python3 scripts/native-media/linux.py \
  --prefix "$PWD/.native/linux" --work-dir /tmp/oxplay-linux-native
export OXPLAY_NATIVE_MPV_PREFIX="$PWD/.native/linux"
export PKG_CONFIG_PATH="$OXPLAY_NATIVE_MPV_PREFIX/lib/pkgconfig"
export LD_LIBRARY_PATH="$OXPLAY_NATIVE_MPV_PREFIX/lib"
```

For Windows, use an x64 or ARM64 native toolchain and Windows SDK that provides
`d3d11_4.h`. Prepare a dependency prefix containing matching-architecture FFmpeg
with D3D11VA, libass, Lua, shaderc, SPIRV-Cross C shared library, pkg-config files
and runtime DLLs. Both MSVC/clang-cl and MinGW can be supplied with a Meson native
file. Run from the configured native compiler environment:

```powershell
python scripts/native-media/windows.py --prefix C:\oxplay-native\sdk `
  --work-dir C:\oxplay-native\build --dependencies C:\native-dependencies
$env:OXPLAY_NATIVE_MPV_PREFIX = 'C:\oxplay-native\sdk'
```

The Windows SDK prefix contains `include/mpv`, `mpv.lib` or `libmpv.dll.a`, and
the installed mpv/dependency DLLs at its root. The runtime DLLs must accompany
the final executable. Review the dependency DLL closure and licenses when
packaging; these recipes do not download an arbitrary prebuilt mpv archive.
Linux packages likewise need the private libmpv and matching libplacebo plus
their shared-library dependencies. Preserve the provenance and patched source
inputs with distribution artifacts.

## ABI and synchronization contract

Installed platform headers define `OXPLAY_NATIVE_RENDER_ABI 1`.
Linux installs `mpv/render_vk.h`; Windows installs `mpv/render_d3d11.h`.
API type strings are `vulkan` and `d3d11`. Parameter IDs are Vulkan init/FBO
21/22, Metal init/FBO 23/24, D3D11 init/FBO 25/26, native capacity 27.
The native headers are the authoritative complete C layouts; Rust mirrors use
`repr(C)`.

Vulkan initialization passes the host instance, physical/logical device, exact
enabled extension list and `VkPhysicalDeviceFeatures2` chain. Required
libplacebo features include `timelineSemaphore` and `hostQueryReset`. Only
graphics queue index zero, count one is imported; dedicated compute/transfer
counts must be zero. The callback receives a **queue family**, not a queue index.
Native queue submits and WGPU submits must execute on the same serialized UI
thread, with the supplied identity/lock callbacks. A GPU polling worker may poll
completion but must not submit work to the shared queue.

The Vulkan FBO adds `target_layout`, wait timeline/value, signal timeline/value
and a required `int *result` enqueue status. The signal value is nonzero and
strictly increases per semaphore. Libplacebo waits before writing, holds the
image back to `target_layout`, then signals completion. Rendering returns
without a CPU completion wait. WGPU must track the same before/after layout;
the intended bridge keeps the tracked state sampled, waits for the producer
signal before UI reads, and signals readiness after prior UI reads. Global
Output-owned timelines outlive cached decoder mappings and resize generations.

VAAPI is enabled only with a DRM render node matching
`VkPhysicalDeviceDrmPropertiesEXT.renderMajor/renderMinor`. A mismatched node
fails initialization. An absent node or unavailable DMA_BUF import keeps VAAPI
disabled. Mapping uses libplacebo's
[VAAPI-to-DRM direct mapping](https://github.com/haasn/libplacebo/blob/v7.360.1/src/include/libplacebo/utils/libav_internal.h)
with `AV_HWFRAME_MAP_DIRECT`; no CPU-copy map fallback is requested. Mapped
decoder frames retain their AVFrame/DMA_BUF leases until their last GPU signal
completes. The registry is capped at 64 mappings and exposes capacity through
parameter 27.

D3D11 initialization passes a non-single-threaded, multithread-protected
`ID3D11Device5` on the DX12 adapter's exact LUID. FBOs must be RGBA8, single-plane
2D textures with render-target/shader-resource binding,
`D3D11_USAGE_DEFAULT`, and `D3D11_RESOURCE_MISC_SHARED_NTHANDLE | D3D11_RESOURCE_MISC_SHARED`. Keyed mutexes
and swapchain buffers are rejected. The FBO adds `ID3D11Fence *` wait/signal
pointers, their 64-bit values, and a required `int *result`. A DX12 consumer
submission transitions the resource to COMMON before signaling readiness;
D3D11 enqueues a wait, renders, signals producer completion and flushes. DX12
enqueues a wait before transitioning to sampled use. The WGPU HAL's
[staged fence wait/signal API](https://github.com/gfx-rs/wgpu/blob/v30.0.1/wgpu-hal/src/dx12/mod.rs)
keeps these operations attached to actual UI submissions.

Check both mpv's return value and the FBO enqueue status before adding the
consumer wait. A negative status does not guarantee that its signal will fire.
Retain texture/device/fence leases until producer and consumer work completes.
Windows capacity checking bounds producer backlog to two and starts one finite
event waiter only when full; completion wakes the UI without per-frame CPU
polling. Linux decoder retirement uses nonblocking timeline counter checks.

Shutdown waits are exceptional and bounded to five seconds in the adapter's
explicit fence path. A failed/device-lost submission retains native leases until
process exit instead of freeing storage still referenced by the GPU. Upstream
libmpv/libplacebo destruction has its own internal cleanup waits; this patch does
not claim that every upstream teardown path is cancellable. Drivers that hang
inside an API call still require process-level recovery.

## Qualification

Source archive verification, patch application, portable build-plan execution,
C header ABI checks and a Windows Rust API cross-check passed on the macOS
development host. The Linux C translation unit passed a host syntax check
against real libplacebo 7.360.1/FFmpeg 9/Vulkan headers with a platform-header
shim; that is not a Linux binary build. Native Linux and
Windows compilation, GPU execution and CPU/memory/cadence measurements must be
performed on those platforms. A passing macOS build does not establish their
runtime qualification.

Before shipping each native platform, verify hardware decode status, subtitle
rendering, audio sync, paused first-frame display, seek, resize, video switching,
minimize/restore, close with frames in flight, device loss and unsupported-codec
software fallback. Exercise X11 and Wayland independently, and test hybrid GPU
adapter matching. Record driver/adapter, source provenance, idle/paused CPU,
steady playback CPU, resident/GPU memory under resize and long playback, frame
cadence and ambient sampling costs. Neither platform should show full-frame CPU
readback or a per-frame GPU-idle wait in the hardware path.
