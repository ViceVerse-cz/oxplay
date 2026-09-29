# Playback focus intent

## Neutral browsing startup (unit tests passed; native validation pending)

Untouched browsing now starts with the existing shared shell `FocusScope`, rather
than the Search editor. This is normal production behavior, not a measurement
flag. The ancestor is excluded from Tab navigation and click focus; Tab proceeds
to actual controls, and slash, clicking Search, or explicitly focusing it retains
Slint's ordinary editable text and blinking caret. No asynchronously loaded card
automatically takes focus. Local-file startup still uses its guarded player
handoff; explicit guest/account selections retain their own request scopes.

The prior `.slint` chain was `Window -> shortcuts -> search`. At the pinned Slint
revision, `internal/compiler/passes/focus_handling.rs::call_focus_on_init`
resolves that chain into initial focus code. `TextInput::show_cursor` installs the
`TextCursorBlinker` binding, whose timer toggles at half the platform cursor-flash
cycle. Removing only the second forwarding link selects the stable ancestor
instead. `FocusScope::focus_event` accepts programmatic focus even when
`focus-on-tab-navigation` and `focus-on-click` are false; those flags therefore do
not disable its explicit startup focus or its descendants' interaction.

`BrowseStartup` shares the existing bounded one-shot handoff and interaction
epoch. Initial inactive-window notifications may precede first activation;
established focus loss, occlusion, mouse/key/IME input, an explicit transition to
Search, or a newer playback selection cancels this intent. The handoff only
consumes it on a visible, focused Home/local-library page. No additional polling
or recurring animation is introduced. The existing AccessKit limitation below
still applies; this does not establish screen-reader acceptance.

The previous `ca21de9` fixture run recorded 162 UI draws, zero video draws and
one application redraw request over its approximately 70-second session. Its
settled CPU mean was 1.2482% of one logical CPU, above the 1% release ceiling,
with 130.656 MiB RSS. The active Search caret and source binding support a
recurring-draw explanation; they do **not** attribute all CPU cost to the caret.
Evidence remains at `artifacts/raster-native-v2/resources/settled.json` and
`settled.json.log`. Those measurements predate this change.

Three new deterministic tests cover startup activation, single use, explicit
search/input cancellation, scope supersession, and real focus-loss/occlusion.
The existing finite local-library diagnostic now checks initial neutral focus,
Tab departure, and slash-to-Search before its usual callback tests. The deterministic tests pass in the locked workspace suite; the revised finite
native checks have not yet run.
Native focus validation and fresh controlled idle measurements remain required;
no resource improvement is claimed.

An explicit initial guest or account video selection may transfer keyboard focus
to the persistent player when its accepted media request opens the watch page.
The transfer is optional: a later user interaction cancels it. Quality changes,
stream refresh, progress events and frame presentation do not request focus.

`crates/app/src/focus_intent.rs` retains at most one nonsecret intention in Rust
UI state. Guest scope includes the catalog worker generation; account scope
includes the playback selection and account session generations. The intention
also records an interaction epoch and whether search was focused when it was
armed. A matching successful initial publication can consume it once, only while
the window is focused, visible, not occluded, and showing loaded playback. A
stale result cannot consume a newer intention. Guest-to-account and
account-to-guest cancellation preserve the other scope's intention. Guest media
waiting for the existing native stop barrier keeps its original request scope.
Load errors, cancellation and request supersession discard the applicable intent.

The existing Winit event filter invalidates intentions on pressed keys, pressed
mouse buttons, wheel input, touch start, IME composition/commit, window focus loss
and occlusion. Initial inactive-window notifications for an explicit local startup
are the narrow exception described below. It propagates all events to Slint. Cursor motion, key/button
releases, rendering and media clocks do not invalidate intent or update Slint
models. In the pinned Slint source, `internal/backends/winit/winitwindowadapter.rs`
calls the filter before dispatching the input to Slint. Thus the initiating input
cancels an older intention before the selection callback arms its replacement.

Shared search edit and focus callbacks also invalidate intent. The focus callback
compares against search focus captured at submission: a deferred notification
from the initiating card click must not cancel the new intention merely because
that click moved focus out of search. A subsequent Tab or return to search does
cancel it. No cursor coordinates, frame buffers, credentials or provider
responses are stored in this mechanism.

## Validation status

At checkpoint 9c70e8e, the implementation included seven deterministic tests for exact scope matching,
single use, supersession, cancellation isolation, IME/focus/occlusion, pointer
input versus cursor motion, hidden/failed requests, and deferred search-focus
notifications. All seven passed in the coordinated workspace run: 240 Rust tests
passed, three explicitly ignored tests remained ignored, and strict workspace
Clippy passed.

The opt-in local `--smoke-test` now includes finite delayed-focus checks alongside
its existing playback, seek, resize, scrolling, fullscreen and minimize checks.
At 13 seconds it arms a diagnostic intention and sends actual Slint text and Tab
input. At 13.2–13.8 seconds it checks that later typing/Tab retains focus, an
unchanged intention focuses the existing player, and a consumed intention cannot
be reused. No provider response, network request or account identity is fabricated.
The process must reach the final focus checkpoint before reporting success.

The macOS native run on 2026-09-29 passed in 21.122 seconds with exit code 0 and no
timeout. Source checkpoint: `9c70e8e`. Release SHA-256:
`541f4cef58a9efb3e7f366a1b1b44938df9ea1cbcb05d4b7ee2ac0449575f84b`.
Evidence: `artifacts/clock-corrected/functional/local.json` and `local.log`.
All four delayed checkpoints completed. Assertions verified that Tab left search
without being redirected to the player, unchanged intent focused the player,
and attempting to reuse it left search focused. The log reports focus after each
checkpoint's deliberate restoration, not the intermediate assertion state.
The same run kept one native file load through the focus changes and reported
zero catalog row changes, with the single initial fixture-model reset unchanged.

This was a local-fixture functional test with diagnostic progress staging enabled
and stable-video-target disabled. It is not a performance measurement, a live
guest-extraction focus test, an authenticated-account test, or validation on any
other operating system. No real account was used. Existing lifecycle assertions
also passed; fullscreen/resize frame drops in this run are not represented as a
steady-playback performance result.

## Accessibility limitation

This is not screen-reader qualification. The pinned Winit backend handles
AccessKit actions through `CustomEvent::Accesskit` in `event_loop.rs`, outside the
public Winit window-event filter. Shared search focus/edit notifications cover
search changes, but an AccessKit action moving focus directly between other
controls is not comprehensively observed by this implementation. Such a focus
change can therefore evade intent cancellation. No public Slint focused-item
observer was found in the adopted API; the application does not depend on
private `WindowInner` state. A source-verified general focus notification or
conservative policy adjustment plus native screen-reader testing remains required
before claiming complete accessibility behavior.


## Local startup correction and frozen ee59eaa result

The first local-startup check at `f6dea13` failed after 3.210 seconds (exit 101):
the player did not have focus before diagnostic keyboard input. Its finite
motion check had passed, which did not make the lifecycle successful. That
[source-associated failure remains retained](evidence/2026-09-29-preferences-native-audit.json).
An intermediate debug build also failed after 4.598 seconds. The instrumented
debug follow-up passed in 21.715 seconds; those two debug binaries have retained
hashes/logs but no frozen source-input capture, so neither is attributed to the
final release commit.

The observed startup sequence delivered an initial inactive `Focused(false)`
notification before first activation. Treating it as a loss of established focus
discarded the explicit `LocalStartup` intention. The correction preserves only
that pending scope when the window was already inactive. Actual focused-to-
unfocused transitions, pressed input, edits and occlusion still cancel intent.
The application filter matches pinned Slint's macOS workaround by using the
native Winit window's `has_focus()` value rather than blindly trusting the raw
Focused payload (`winitwindowadapter.rs` at Slint
`cf3b07d4917e6759a63b0c03913a2594ec653414`). No private Slint focus API is used.

A request-scoped, zero-delay single-shot Slint Timer defers the handoff until
Slint has processed activation and forward-focus. Its callback holds weak window
and state references, rechecks actual native focus and local loaded/visible
watch-page state, and consumes the matching intent once. It neither introduces
polling nor reapplies focus on playback-clock updates. Rendering and activation
may arrive in either order; incomplete startup leaves the intention available,
while later user interaction still invalidates it.

Frozen source `ee59eaa47289382b6784547e8bece1b38e5e60a5` produced release
`464fdcf1432f1425698cf2892f2fd7f3e942c4e132dfb176c5c45bcb5582b0a8`.
All 119 captured inputs independently match that commit. Central checks passed
261 Rust tests, three ignored, formatting, strict workspace all-target Clippy
and the locked release build. The focus module now contains eight deterministic
tests, including startup activation versus later-input/focus-loss cancellation.

The final release local lifecycle passed in 21.071 seconds with exit 0 and no
timeout. It asserted initial player focus before synthetic input, then completed
paused seek, resize/scroll/fullscreen, text-input shortcut exclusion, all four
delayed-focus stages, hidden controls and minimize/restore. It retained one file
load, zero catalog changes and the one fixture reset. Its finite composed-video
check changed 838/9,216 grid pixels. Final VO/decoder drops were 65/0 during the
intentional lifecycle transitions; this is not steady-playback performance.
Clock staging and stable targets were both off.

[Sanitized release/debug logs, metadata and source provenance](evidence/2026-09-29-startup-focus-summary.json)
exclude private profile paths. This proves the exercised local startup and
finite interaction sequence on this macOS host, not live account focus,
screen-reader behavior, compositor pacing, perceptual A/V or another platform.
A separate hour-long soak was started after the release check; no completed soak
or resource qualification is claimed by this evidence.
