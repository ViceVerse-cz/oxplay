# Watch layout corrections

The media slider explicitly anchors its filled rectangle at the track origin.
Slint otherwise centers a child whose width is smaller than its parent, which
made the red fill disagree with the seek thumb. Volume uses the same correction.
Keyboard focus is visible on the thumb, without a full-width rectangular border.

During scrubbing, the local drag position follows the pointer. After release,
the timeline uses the latest accepted absolute seek target while libmpv's
existing exact-load seek confirmation remains pending. Coalesced seeks retain
the latest target; confirmation, failure, stop and load transitions retire it.
Elapsed/remaining text and persisted resume positions remain observed values.
No new progress timer, smoothing loop or media object is introduced.

The title is followed by one row with circular channel artwork, creator name,
and actions. Explicit cross-axis centering aligns the 40px avatar and 36px
buttons. Names elide within the remaining space; Share/Save retain fixed sizes.
The three-dot control-visibility button is removed. Genuine public guest
artwork and its fallback are described in [channel avatars](channel-avatars.md).

Theatre mode uses T or the upstream Lucide `rectangle-horizontal` control.
It hides the sidebar, widens the existing video host, caps its height against
the viewport, and places the related list below inline watch details. Search
and metadata remain available. Escape returns to the regular layout; native
fullscreen and PiP retain their own Escape behavior. Changing theatre mode
parks focus on the persistent video and reveals the top of the watch page.
Fullscreen/PiP temporarily suspend the theatre layout; returning restores it.
The header menu also exits theatre mode to reveal navigation. It is a session
layout preference, not another window or presenter.

Normal windows use native operating-system decorations, documented in
[window appearance](window-appearance.md). All application content is still
compiled from the shared Slint component library.

Validation on the implementation host:

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo build --workspace --locked`: passed.
- `git diff --check` and vendored icon/license checksums: passed.
- Three focused provider regressions were authored and compiled, not executed:
  explicit avatar identity parsing and foreground/background admission ordering.
  Existing library keyboard diagnostics now open the real editing disclosure.

Two short isolated offline capture attempts used the generated local fixture,
paused playback, demo-labeled related rows, fresh data roots, dark theme,
1100x760/760x600 sizes, `--snapshot`, and `--quit-after 18`. Both exited normally,
but presenter startup failed with `macOS create display clock failed (-6661)`.
The snapshots consequently show the shell/error state rather than the requested
watch/library layout and are **not** visual acceptance evidence. Local attempt
artifacts are under `artifacts/watch-layout-vl54l23s/` (ignored). Slint content
snapshots also exclude native window decorations, so they cannot verify rounded
AppKit corners/traffic lights. Repeat the targeted visual check with an active
native display; no decoder, avatar-provider or theatre/PiP interaction result is
claimed for this source.

No full test suite, performance/usage benchmark, live account operation or new
platform-release qualification is part of this UI correction.
