# Diagnostic libmpv baseline

This standalone OpenGL window isolates libmpv timing from Slint. It is a test
program, not a second application frontend and not part of the shipped runtime.
It accepts an explicit absolute local media path only. The default settings do
not read mpv configuration or load scripts, and they request hardware decoding;
printed `hwdec` is the observed decoder, not the request.

```sh
cc -std=c11 -Wall -Wextra -Werror -O2 tools/media-baseline/main.c \
  $(pkg-config --cflags --libs mpv sdl2) -framework OpenGL -framework CoreVideo \
  -framework IOKit -framework CoreFoundation -o /tmp/serein-media-baseline
/tmp/serein-media-baseline "$PWD/artifacts/local-1080p60.mp4" 30 1 0.05
```

Arguments are absolute clip path, duration in seconds, whether libmpv blocks
until frame target time (0/1), and `video-timing-offset` in seconds. Optional fifth
argument sets advanced control (default 1); optional sixth selects swap mode:
0 uses SDL CVDisplayLink timing, 1 uses native CGL interval 1, 2 disables both
waits, 3 tests waiting before rendering rather than after, and 4 uses a demand-started
CVDisplayLink with callback-only wakeup and stops it before each draw. Mode 5
keeps display-clock phase during active video and stops after two empty ticks.
Mode 6 matches the current application's phase-continuous clock: it keeps the
clock running while the native engine is observed Playing and unpaused, even
through gaps in frame notifications. Display ticks only wake SDL when a frame is
pending; engine frame callbacks do not independently wake SDL while the clock is
running. FILE_LOADED/PLAYBACK_RESTART, SEEK, pause, cache buffering and EOF events
drive state without polling. Paused seeks can render a final frame without a
clock restart. Hidden/minimized windows pause and stop the clock; restoration
resumes only a pause introduced by hiding. SDL visibility events are not proof
of Cocoa's complete occlusion behavior. Mode 6 also mirrors the application's
scoped idle-display-sleep assertion during observed playback; it does not wake
a sleeping display or change system preferences.

Modes 1–6 are
macOS timing diagnostics; the documented build command targets macOS. The window
defaults to 928×280 logical pixels (optional seventh/eighth arguments set width
and height; 692×389 matches the current 1384×778 application target), uses the actual drawable's physical size, requests
OpenGL 4.1 and Retina drawable resolution, and prints observed display mode and swap-setting
result. Only one-shot checkpoints/end timers run. Coalesced media callbacks
wake the SDL event loop. Control uses asynchronous calls and property
observations; default `ADVANCED_CONTROL=1` follows the installed render API contract.

The program does not call `report_swap`: like the reviewed upstream SDL example,
it does not claim a GL buffer swap return is actual display scanout. Blocking
render here is a diagnostic baseline, not permission to block the Slint UI's
ordinary input loop in production. This baseline also omits Slint composition,
so it cannot by itself qualify the application's resource or presentation gate.

Source/API review used mpv's installed 2.5.0 C API (Homebrew mpv 0.41.0_10) and
[the upstream SDL example](https://github.com/mpv-player/mpv-examples/blob/e0d1a84c99e8c469b58fde31a1541401acfb0eb2/libmpv/sdl/main.c),
whose repository HEAD was resolved with `git ls-remote` on 2026-09-29. No upstream
example file is vendored: the checked revision did not include a repository
license, and the small diagnostic implementation is newly authored under
GPL-3.0-or-later. The API/threading rules are in the installed `mpv/render.h`.

Measured-system diagnostic dependencies are Homebrew sdl2-compat 2.32.72 and
SDL3 3.4.16; both installed license files contain the zlib license. These are
build/test dependencies only and are not added to Cargo or application packaging.
Their installed license locations are respectively
`/opt/homebrew/Cellar/sdl2-compat/2.32.72/LICENSE.txt` and
`/opt/homebrew/Cellar/sdl3/3.4.16/LICENSE.txt`. libmpv licensing remains covered by
`docs/licensing.md`; this harness does not change the engine distribution.

## Current phase-continuous comparison

Compile after any source edits and outside a measurement window:

```sh
cc -std=c11 -Wall -Wextra -Werror -O2 tools/media-baseline/main.c \
  $(pkg-config --cflags --libs mpv sdl2) \
  -framework OpenGL -framework CoreVideo -framework IOKit -framework CoreFoundation \
  -o /tmp/serein-media-baseline
shasum -a 256 /tmp/serein-media-baseline tools/media-baseline/main.c
```

The matched application options are nonblocking render, 50 ms timing offset,
advanced control disabled, and mode 6. A short functional run should precede
resource sampling:

```sh
/tmp/serein-media-baseline "$PWD/artifacts/local-1080p60.mp4" 10 0 0.05 0 6 692 389
```

Only after that succeeds and the whole host is quiet, collect the same ten-second
warm-up plus sixty one-second samples used for the application:

```sh
python3 scripts/measure.py --warmup 10 --seconds 60 --include-new-vt-services \
  --output artifacts/standalone-mode6.json -- \
  /tmp/serein-media-baseline "$PWD/artifacts/local-1080p60.mp4" 85 0 0.05 0 6 692 389
```

Record source/binary hashes, GL vendor/renderer/version, observed drawable
1384×778 (do not assume the requested logical size proves it), display refresh,
VideoToolbox hardware decoder, H264 1920×1080@60, audible AAC/48 kHz, and native
clock counters from the log. Checkpoints at 10, 25, 70 seconds and exit provide
dropped-frame counters bracketing the sampled interval. They are discrete SDL
events, so inspect actual timestamps when interpreting boundary differences.
Foreign-host CPU and newly appearing VT service attribution have the same
limitations as `measure.py`. SDL/driver and direct-default-framebuffer overhead
differ from the application's persistent texture plus Slint composition; this
is a baseline attribution comparison, not equivalent application functionality.

Mode 6 compiled successfully on 2026-09-29 with the strict command above; binary
SHA-256 `f19d1d40032a13a7f1c842aafaffd4bf3db66f9063999dccf4c4451efc4fa528`,
source SHA-256 `cf7a2949ba531af191a4c3c8042944b3e03a2f91f2d645bd8f7c12c1e03bfdc4`.
The subsequent native functional and 85-second resource runs completed with
observed VideoToolbox H264/AAC and no additional warm-interval dropped frames;
results and attribution limits are in
[the standalone baseline record](../../docs/standalone-baseline.md).
Earlier mode-5 measurements remain historical and do not qualify this updated
lifecycle. Modes 0–5 retain their scheduling behavior.
