# Picture in picture

The player’s picture-in-picture button or **P** turns the existing Serein window
into a compact floating player. Play/pause, seek, elapsed time and mute remain
available. **P**, **Escape**, the return button or closing the compact window
restores the main window. Closing the restored main window exits normally.
Fullscreen and PiP are mutually exclusive; F in PiP returns to the main player.
Shortcuts remain inactive while editing text.

This is a floating mode of the current window, not a second browsing window.
The main browsing layout returns when PiP closes. It preserves the same player,
media load, compiled Slint component and presentation context; no borrowed
texture crosses a window boundary and no extra decoder is created. It does not
change guest/account authority, quality, captions or accepted pause intent.
Account expiry and disconnection still invalidate authenticated playback.

The initial client size is 480×328 logical pixels, with a 360×240 minimum.
The original logical client size, physical desktop position, maximized state
and window-button configuration are retained. Restoration clamps an off-screen
window to the current monitor while respecting the normal 760×600 UI minimum.
Winit exposes monitor bounds, not a portable desktop work-area API. DPI changes,
different monitor arrangements and compositor-specific behavior need further
native qualification.

All controls are in `crates/app/ui/app.slint`. The controller in
`crates/app/src/picture_in_picture.rs` owns geometry only; media and graphics
ownership stay in the existing presenter. Slint’s `always-on-top` property owns
the window level. The unchanged Lucide `picture-in-picture-2.svg` uses the
[existing upstream revision and notices](../crates/app/ui/icons/SOURCE.md).

## Backend boundaries

The controller recognizes AppKit, Win32 and X11 window handles. Native Wayland
is explicitly unavailable because this pinned Winit backend does not implement
the always-on-top request. The experimental native-child presenter also keeps
PiP disabled until it has its own compact overlay/lifecycle validation.
Recognizing an API is not a Windows/X11 runtime support claim.

PiP capability is checked from actual render/window initialization. In the
pinned Slint source, `show()` can return before Winit creates its native window;
an earlier one-time check after `show()` incorrectly left PiP disabled. The
event-driven check fixes that without polling or a recurring redraw timer.
Maximize/fullscreen window buttons are disabled during PiP where Winit supports
that hint; native fullscreen requests are returned to windowed mode before
restoration. Multi-display fullscreen, Spaces and physical OS-button behavior
remain separate manual cases.

## Executed functional checks

On the development Apple M1/macOS 27.0 host, the debug build passed nine finite
local-video stages: compact entry, observed pause, paused seek, resume with
advancing media time, 360×260 resize, Escape, restored geometry, re-entry and
Slint close-request restoration. Every stage checked the same native window,
media load and presenter generation. Local H.264 video and SRT subtitles were
visible in the inspected 480×328 capture. The player observed VideoToolbox;
this is not a resource-budget, perceptual audio-sync or general codec claim.

A separate one-shot CoreGraphics query, filtered to the owned application PID
before reading output fields, observed window 5311 at layer 3 with a 480×360
decorated frame during PiP and layer 0 with its restored 900×682 frame afterward.
These include native decorations; the corresponding client sizes are 480×328
and 900×650. The query emitted no window titles or other applications’ metadata.
The application exited successfully and its supervised test process was reaped.
Working-tree evidence is retained under ignored `artifacts/pip-v3/`.

The release build also passed all nine stages, entering through the actual **P**
key handler and returning through Escape and the Slint close request. Its capture
was inspected and retains the subtitle and fixture label. The owned native
window 5321 moved from layer 3 (480×360 decorated) to layer 0 (900×682), with its
original position restored. Evidence is in `artifacts/pip-release-v1/`; executable
SHA256 is `5b0d5603488a50f64319b47a5df10b25a316b8f1ca2fc947c8c2aedec1c939f0`.
Both builds exited successfully with unchanged window/load/presenter identities.

Two prior attempts remain recorded: `pip-v1` could not create a CoreVideo clock
while the display was asleep; `pip-v2` played video but exposed the early
capability-check bug described above. Neither is counted as a pass. The display
was explicitly awakened for the successful functional test. No performance or
usage benchmark was run.

To repeat with an existing local clip and a fresh isolated profile:

```sh
cargo build --locked
./target/debug/serein --pip-smoke-test \
  --local /absolute/path/clip.mp4 --subtitle /absolute/path/captions.srt \
  --data-root /absolute/path/new-profile --ui-size 900x650 \
  --snapshot /absolute/path/new-pip.png
```

The diagnostic uses a 25-second watchdog, forbids online inputs and other
diagnostic modes, and labels its fixture state. It is not part of headless CI.
The system must have an awake graphical session. The screenshot is a single
explicit test capture, not the normal video presentation path.
