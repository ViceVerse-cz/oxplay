# Playback keyboard routing

The shared Slint player handles Space/K, J/L, arrows, M, F and Escape after
focused child widgets have had the first opportunity to consume a key. `/`
focuses search. Modifier combinations remain available to widgets and the host.
Sliders handle their own arrow/Home/End keys; buttons handle Space/Return.

Ignored child keys can still bubble. In the pinned Slint revision
[`cf3b07d4917e6759a63b0c03913a2594ec653414`](https://github.com/slint-ui/slint/tree/cf3b07d4917e6759a63b0c03913a2594ec653414),
`internal/core/items/text.rs::TextInput::key_event` rejects special keys such as
Escape. `internal/core/window.rs` then delivers the event to ancestor items.
The playback shortcut scope therefore checks the documented
`TextInputInterface.text-input-focused` builtin before handling any shortcut.
The builtin is set on TextInput focus acquisition, including readonly inputs,
and cleared when focus is lost. It covers editors rather than only the search
field. Rejected events retain Slint's normal popup Escape behavior; this change
does not install a capture handler ahead of text editing or IME processing.

The explicit local `--smoke-test` now checks both directions: at 11 seconds,
a click in the fullscreen video focuses the player, then Escape invokes
fullscreen exit exactly once; at 13 seconds,
Escape while editing search preserves focus and invokes zero fullscreen
callbacks. A diagnostic-only callback counter detects the latter even when
the window was already outside fullscreen. Existing typing/focus checks remain.
The first post-soak native run (`0f311dd`, retained locally under
`artifacts/raster-native-v1/local.log`) reached the first Escape assertion but
then failed the 12-second scroll-restoration assertion. The new test had called
`focus-player()` immediately before Escape. That function intentionally reveals
the player and resets its saved normal-window scroll, conflicting with the
existing scroll-restoration check. The revised test uses the ordinary video
TouchArea click, which changes focus without resetting the saved offset. It
retains the later restoration and single-file-load assertions.

The corrected sequence passed at source `ca21de92a94abf209b76576c8b584751d3d20b4f`
in both [debug](evidence/2026-09-29-raster-corrected-debug-local.log) (21.723 seconds)
and [release](evidence/2026-09-29-raster-corrected-release-local.log) (21.549 seconds).
Both runs observed one player Escape callback, restored watch scroll, and zero
playback fullscreen callbacks from editor Escape. Existing typed `f`, Tab/focus
intent, local pause/seek, resize/minimize/restore and one-file-load checks passed.
Observed mute/unmute retained volume 100 and one file load. These are event/state
checks, not a listening test. [Source inventories and binary/run hashes](evidence/2026-09-29-raster-corrected-summary.json)
associate both builds with 125 captured inputs; the prior failed run remains retained.
They do not replace manual native IME composition, text selection/copy,
screen-reader, DPI, or other-platform qualification.
