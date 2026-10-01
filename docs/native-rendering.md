# Native rendering

The application has one `VideoPresenter` contract for native GPU output and the
explicit OpenGL comparison. Platform adapters produce GPU textures sampled by
the shared Slint UI; application code never transports full video frames through
CPU buffers. The native build uses the `native-rendering` Cargo feature.

| Platform | Slint UI | Native media output | Hardware validation |
| --- | --- | --- | --- |
| macOS | Metal through FemtoVG-WGPU | Metal/libplacebo and VideoToolbox texture mapping | Requires measured evidence from the macOS runtime checks |
| Windows | Direct3D 12 through FemtoVG-WGPU | D3D11/libplacebo and D3D11VA on the same adapter, shared with DX12 | Unavailable in the current macOS workspace |
| Linux | Vulkan through FemtoVG-WGPU | Vulkan/libplacebo on the same device/queue, matched-node VAAPI DMA-BUF mapping | Unavailable in the current macOS workspace |

Implemented adapters and cross-target API checks do not qualify Linux/Windows
drivers, hardware decoding, presentation correctness, performance, or power use.
Real platform hosts are required before support claims can be made.

## Build and selection

Native output requires the pinned custom libmpv/libplacebo prefix built by
`scripts/native-media/macos.py`, `linux.py`, or `windows.py`. Stock libmpv is
incompatible with the private native render ABI. `crates/media/build.rs` requires
the platform header to advertise `OXPLAY_NATIVE_RENDER_ABI 1` before linking.
Use each builder's `--help` for host toolchain and prefix arguments. Linux and
Windows builds require their respective hosts; `--plan` only inspects the plan.

The macOS build requires Homebrew `vulkan-headers` alongside its Metal/shader
dependencies. The pinned libplacebo compiles Vulkan stubs that include the
public Vulkan header even when its Vulkan backend is disabled; this prerequisite
does not enable Vulkan rendering or add a Vulkan runtime dependency.

`OXPLAY_NATIVE_MPV_PREFIX` overrides the dependency prefix; the default is
`artifacts/native-media/<platform>/prefix`. Windows also accepts `MPV_DIR` for
the native development prefix. Build with `cargo build -p oxplay --release
--features native-rendering`. Native builds select the platform backend by
default; `--graphics-backend native` selects it explicitly. Selection prefers a
low-power adapter, verifies the observed API/adapter, rejects CPU adapters, and
does not silently fall back to OpenGL.

For OpenGL playback comparison, build with `--no-default-features` against stock
libmpv and run with `--graphics-backend opengl`. This is a separate linked build:
the Linux and Windows native dependency scripts disable mpv's OpenGL renderer.
The experimental macOS native-child diagnostic also uses OpenGL.

## Shared UI and media lifecycle

Slint is pinned to `cf3b07d4917e6759a63b0c03913a2594ec653414`. Native UI integration
uses `renderer-femtovg-wgpu`, `unstable-wgpu-30`, and the existing winit features.
The selector uses `renderer_name("femtovg-wgpu")` and `require_wgpu_30`; the pinned
renderer rejects `require_metal`, `require_d3d`, and `require_vulkan` as substitutes
when the public WGPU30 integration feature is enabled.

`GraphicsAPI::WGPU30` supplies the renderer's instance, device, and queue during
render notifications. `device.adapter_info()` records the observed backend, GPU,
device type, vendor/device identifiers, and driver. `NativePresenter` attaches
media to those resources and preserves mpv control, audio, captions, seeks, and
correlated first-frame/load behavior. Private source patches implement the
native render API; an external mpv window is not involved.

The common presenter initializes each new RGBA8 target through WGPU before its
first native write, preventing WGPU's lazy initialization from erasing that
frame. Slint retains the imported texture through `Image::try_from`. The pinned
API accepts `Rgba8Unorm`/`Rgba8UnormSrgb` with `TEXTURE_BINDING` and
`RENDER_ATTACHMENT` usage; native video uses `Rgba8Unorm`. YUV conversion,
scaling, and caption composition happen in the native GPU renderer before import.

Startup frames remain private until the correlated load is ready. Prior images
survive replacement and resize. Native allocation leases and submitted GPU
commands keep textures alive through completion; releasing a UI image alone
does not make its allocation reusable. Context teardown drains retained producer
work before releasing callback userdata and platform resources.

## Platform synchronization

On macOS, media uses Slint's Metal device and command queue. One queue orders
prior UI reads, the native write, and subsequent UI reads. Tracked texture
transitions agree with the native write/read boundary, and VideoToolbox textures
remain retained through GPU command completion.

On Windows, the media device is selected using the UI DX12 device's adapter LUID.
Persistent D3D11 textures are opened as DX12 resources on the same adapter.
Shared producer/consumer GPU fences order writes and reads; native resources
return to `COMMON` for transfer. Required shared-fence interfaces must exist;
another adapter or a CPU frame copy is not substituted if they are unavailable.

Linux uses `WGPUConfiguration::Manual` from
`oxplay_media::native_gpu_configuration`. The helper creates a Vulkan device
with WGPU HAL's requested features plus `hostQueryReset`, imports it into WGPU,
and passes the same handles to Slint/libplacebo. Automatic WGPU creation at this
revision never enables `hostQueryReset`; requesting `TIMESTAMP_QUERY` does not
fix that omission. The native import receives the exact enabled feature chain,
including Vulkan 1.2 timeline semaphore and host query reset features.

One graphics/compute queue is shared. Native callbacks enforce the owning UI
thread, family, and index; WGPU submissions also belong to that thread. The
pinned libplacebo Vulkan backend has no background queue-submit thread, and VAAPI
decoder work does not access this Vulkan queue.

Two Output-owned timeline semaphores transfer Vulkan targets. WGPU establishes
tracked `RESOURCE`/`SHADER_READ_ONLY_OPTIMAL` and signals ready after prior UI
reads. Libplacebo waits, renders, returns the image to the same sampled layout,
and signals done. The next WGPU submission waits for done before sampling.
Agreeing on the boundary layout avoids a WGPU barrier with an incorrect old
layout after native rendering. Failed native submissions never enqueue a wait
on an unsubmitted signal.

VAAPI requires supported DMA-BUF import features and a render node derived from
the selected physical GPU. The C backend checks that node's device major/minor
against `VkPhysicalDeviceDrmPropertiesEXT` before direct mapping. If no matching
node is available, native VAAPI is disabled; another adapter's output is not
silently copied into this GPU.

Normal Vulkan frames use GPU waits only. Completed leases are collected with
nonblocking polls. If capacity is exhausted, one persistent worker waits for a
specific outstanding submission with a finite timeout and enqueues one retry
wake. Otherwise it blocks on its channel; it does not wait on every frame or run
a polling timer.

## Native ambient summary

`wgpu_ambient.rs` reduces published RGBA8 targets entirely on the GPU. Its compute
shader samples a bilinear 128×72 grid inside `content_rect`, excluding bars, and
averages each 8×8 group into one of 16×9 packed RGBA8 cells. Only the final
576-byte buffer is copied and mapped. Unchanged enabled loads sample at most
four times per second; a new load can sample immediately.

The device requests `Limits::downlevel_defaults()` because Slint's default
WebGL2 limits disable compute/storage buffers. One readback may be pending.
A persistent worker advances its map with a bounded wait for that submission
and enqueues a completion wake. There is no UI-thread GPU wait, full-frame
readback, per-frame worker creation, or polling timer. Paused first-frame colors
can arrive without another decoder frame.

Disable, re-enable, and replacement loads invalidate a generation. Obsolete
results are discarded, and their buffer ownership retires before reuse. An
obsolete completion may wake an enabled replacement once so its paused frame
can be sampled. Ambient failure disables sampling without aborting playback.

## Qualification and performance evidence

FemtoVG-WGPU submits its clear before `BeforeRendering`, submits UI commands
before `AfterRendering`, and presents afterward. `AfterRendering` is neither GPU
nor display completion. WGPU callbacks need a later submission or device/instance
poll; the adapters arrange progress when paused frames or exhausted capacity
would otherwise produce no event.

Compare release builds with identical fixtures, dimensions, backing scale, UI
settings, and workloads. Record observed UI/media APIs, GPU adapter, hardware
decoder, CPU/GPU time, memory, dropped frames, wakeups, idle/paused behavior, and
power. Target-byte counters estimate tracked allocations; memory measurements
must also include in-flight leases, decoder/driver resources, and the compositor.

Verify paused/first frames, rapid seeks and replacements, captions, color and
orientation fixtures, resize, fullscreen, picture-in-picture, overlays,
hide/show, device-loss errors, and teardown on every platform. A native API label
or cross-target type check alone establishes neither better performance nor
hardware-decoder transport.
