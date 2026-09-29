# Shared UI validation

The UI remains compiled Slint with a single Winit/FemtoVG window. Unmodified upstream Lucide SVG
icons and restrained red/neutral colors define the shared visual language. Icon provenance,
licenses, and hashes are retained in `crates/app/ui/icons/SOURCE.md`; earlier
project-created icons and text-glyph placeholders have been removed. Browsing uses
virtualized ListView rows of responsive cards; the watch page retains a persistent
video host, a related-result list, keyboard controls and fullscreen controls.

## Executed on macOS / Apple M1

`cargo build --locked -p serein` passed. Two native guest searches used:

```sh
target/debug/serein --search 'Blender open movie' \
  --snapshot artifacts/ui-browse-v3.png --quit-after 18
```

The first diagnostic capture exposed default-position centering mistakes in the
header, heading and thumbnail rectangles. Explicit positions corrected these.
The second capture was inspected: the search header, sidebar, real thumbnail grid,
titles, channel names, durations and next-page action are laid out coherently.
Images and metadata came from the actual guest provider; no fixture feed was used.
The capture is an ignored local development artifact containing the explicit
public search query. It is not a performance sample. It predates the Lucide icon
replacement; the later native captures described below exercise the current icons.

Image workers allow four concurrent requests, a 40-request viewport batch, eight
ready results and a 120-row catalog. Requests are anonymous HTTPS to exact ytimg
hosts, with inherited proxies/cookies and redirects disabled. Compressed inputs
are limited to 2 MiB; image dimensions/allocation are bounded, and decoded output
is at most 320×180 RGBA. Only the viewport plus one row of overscan is requested.
Leaving the range cancels outstanding work and clears stale Slint image references.
New generations reject late results. No disk image cache is currently implemented.
The worst-case retained display buffers (40 rows + eight results + four waiting
workers) are approximately 11.43 MiB, separately from decoder scratch space and
renderer texture caches. Actual GPU-cache eviction and peak resident memory still
need measurement; this arithmetic is not a budget pass.

Thumbnail completion notifies one flat row and its existing grouped child row.
Appending results retains existing child models. Grouping is rebuilt only at a
column-count boundary or explicit catalog replacement. Deterministic tests cover
stable child identity, targeted updates, append behavior and 1,000 unchanged-width
updates without regrouping. Playback/pointer invalidation tests remain separate
from whole-window GPU redraw accounting.

The native file picker is a small system adapter. All account disclosures,
consent, connection status, capabilities, reads/writes and player controls remain
in the shared component. Real account identity verification is required before
account features are enabled; no real credential import has been run by the agent.

## Still required

The source-associated checks below cover selected watch/fullscreen and small-window
paths; complete both-theme layout and interaction checks after further bindings
land. Test actual keyboard traversal and a screen reader; enabling accessibility
is not sufficient evidence. Qualify local library pagination/history/import/export,
account flows with explicitly authorized human sessions, and all non-macOS UIs.
Measure a settled library, rapid scrolling, repeated navigation, hover, retained
thumbnail textures, memory bounds and application-owned process-tree costs.

## Upstream icon replacement

The initial 18 UI SVGs were copied byte-for-byte from official Lucide revision
`66d8f9fc394b8530377e5f6112f0b8908ba01280`. Asset/license hash verification passed
for every entry in `crates/app/ui/icons/SHA256SUMS`. Shared Slint components
use semantic icon names and SVG images for navigation, player controls, the
application badge, thumbnail placeholders and empty states; no font glyph is
used as an icon. Full ISC and retained Feather MIT notices accompany the files.
`cargo check --locked -p serein` passed with all updated SVG assets compiled
through Slint. Subsequent native captures verified Lucide rendering in the local
library (dark) and account page (light, 760×600 logical pixels). Account/settings
content now uses a ScrollView so narrow layouts retain access to lower controls.
The account capture showed the complete initial risk text, consent, protected
storage choice and disabled import action without clipped text. It used an isolated
empty local profile; no consent was selected and no account request was performed.

The library capture used an explicitly generated 10,000-row synthetic database
under `artifacts/library-fixture/Serein`. All collection/row names visibly identify
the fixture. A 100-record page appeared in the virtualized list with a next-page
action; this visual check is not a memory/scrolling budget pass. The generator is
a development-only Cargo example and refuses an existing database destination.

Reproduction (absolute data root; it owns only the `Serein` child):

```sh
cargo run --locked -p serein-storage --example fixture_library -- \
  /absolute/new-fixture/Serein/library.sqlite3
target/debug/serein --data-root /absolute/new-fixture --ui-page library \
  --ui-theme dark --snapshot /absolute/new-library.png --quit-after 18
target/debug/serein --data-root /absolute/empty-profile --ui-page account \
  --ui-theme light --ui-size 760x600 --snapshot /absolute/new-account.png --quit-after 18
```

Diagnostic page/theme/size overrides do not persist preferences or authorize an
account import. Snapshot readback occurs once only when explicitly requested;
never use it in media performance samples. The normal profile is untouched by
these isolated runs. Artifacts remain local and are not shipped as product data.

The 760×600 dark settings capture was also inspected: theme and privacy/about
sections fit with readable wrapping. A new real public search capture verified
mixed video/playlist/channel cards, type badges, genuine thumbnails, pagination
and Lucide navigation. Avatar absence is rendered as an explicit icon placeholder,
not an invented image. These runs had no playback active; the machine's display
subsequently slept, and presentation reported that it was unavailable. Browsing
continued. A separate controlled wake/recovery test is recorded in video integration.

## Native keyboard lifecycle check

The shared visual tree now has an ancestor FocusScope for unhandled keyboard
shortcuts. Text inputs and their IME processing get first refusal. `/` focuses
search; K/Space toggles playback, J/L seeks ten seconds, arrows seek five seconds
or adjust volume, M mutes, F toggles fullscreen and Escape exits it. Modified
keyboard shortcuts are left to the platform/active widget. The video host stays
persistent inside that scope; it is not rebuilt by focus or control changes.

The native `--smoke-test` now dispatches a K press/release after focusing the
player and verifies the actual paused seek at 20 seconds. It dispatches `/`,
asserts search focus, types F into that field and verifies that fullscreen did
not activate. The run exited 0 with active VideoToolbox/AAC output, local subtitle
attached through per-file load options, zero unrelated catalog changes and only
the explicitly labeled fixture initialization reset. Fullscreen now invokes the
same shared UI callbacks as the actual button, exercising the fullscreen layout.

This debug lifecycle test includes resize, fullscreen transitions and minimize;
93 VO drops in that run do not constitute a steady-playback performance result.
It does not replace keyboard traversal of every dialog, IME composition, or an
actual screen-reader test. Local log: `artifacts/keyboard-lifecycle.log`.


## Accessible controls and caption layout audit

Explicit accessible names now cover every shared slider, combo box and text
entry. Directional collection paging names distinguish previous/next; navigation
selection and the playback-details toggle expose their state without treating
the visually emphasized import button as a selection. The caption popup Close
button has a fixed width so it stays at the right edge of its header. These
changes compile through the locked Slint build; full native screen-reader and
IME qualification remains open. The ApplicationServices permission probe
returned false, so source inspection is not reported as a VoiceOver pass.
See [accessibility qualification](accessibility.md).


The `ac87f2c3…` release was compiled and run at760×600 in the light theme.
[Actual local-fixture capture](evidence/2026-09-29-controls-light.png) shows the
seek bar taking the available space, fixed40px icon buttons, readable time labels
and grouped volume/actions. The screenshot was inspected directly. Fullscreen
controls now use the theme surface instead of a fixed dark backdrop; native
fullscreen lifecycle passed, while a dedicated fullscreen visual/contrast audit
remains pending. All icons are the same licensed official Lucide assets.
This18-second run includes one explicit screenshot readback, so it is excluded
from performance evidence. The local color pattern and related rows are labeled
test fixtures. [Log](evidence/2026-09-29-controls-light.log).


## Navigation and responsive watch checkpoint e97ad3f

Release SHA256 `f61a860fad7d31d5200dd36a00cf4d37912fb946407a27a16d05bb38a0b4d634`
is associated with source `e97ad3f916db5b42c7f1a0a3291b8c0fe149ca69`:
all 112 captured input hashes were independently compared with the commit's file
bytes. This identifies the source inputs; it is not an independent reproducible
rebuild of native dependencies. The central check reported 226 Rust tests passing,
three ignored, 50 Python tests, strict workspace Clippy and a locked release build.

The shared shell now uses a 56px header, 232px expanded sidebar and 72px compact
sidebar. Home, local Subscriptions, Playlists and conditionally visible History
have explicit navigation, with account collections still separate. Local tabs
are selected atomically before loading their page: a navigation callback cannot
silently select the previous tab because another callback already made the worker
busy. History follows acknowledged storage preferences and is unavailable while
disabled. The local name and explanatory text distinguish local follows from
YouTube-account subscriptions; Home is guest browsing, not a fabricated or
background-refreshed recommendation feed.

The watch page has one persistent video host inside a scrollable content area.
Compact layouts place a bounded virtualized related list below the player and
metadata, instead of assigning it zero available height. Title/channel text is
bounded; fullscreen resets scrolling and restores the previous nonfullscreen
position afterward. Thumbnail requests account for clipping by both viewports.
The visible transport's progress timer stops when those controls scroll out of
view; this does not claim that all media rendering stops offscreen. No second player or duplicate video Image is instantiated for another layout;
resizing can still replace the persistent presenter's GPU targets.

The 21.046-second local native lifecycle run exited 0 on macOS/Apple M1. It used
a generated local clip and labeled related rows in an isolated profile. The
harness dispatched an actual Slint wheel event at 760×600, observed related rows
become reachable, verified no offscreen related thumbnail requests before
scrolling, and checked that the offscreen transport progress timer stopped.
Fullscreen used offset zero, exiting restored the scrolled position, and the
player retained exactly one file load. The existing paused-seek, text-input
shortcut exclusion, hide-controls and minimize/restore checks also completed.
Catalog changes remained zero and resets stayed at the one fixture initialization.
The test intentionally changes size and visibility; its 64 final VO drops and
8 target allocations are not a steady-playback resource result. It does not
qualify real trackpad gesture behavior or perceived A/V synchronization.

The previous `ab495e0147a39db17f22ba9217b75dac40f82a94ed1c268b2fc5fec1342c78dc`
binary failed after 12.882 seconds with `watch scroll was not restored`. That
failure and its input-hash capture are retained separately. The correction saves
the offset on user scrolling and reapplies it after viewport/content geometry
changes, rather than allowing fullscreen clamping to overwrite it. The successful
repeat is not substituted for the original failed observation.

The 28.260-second offline library callback check also exited 0. It exercised
Subscriptions/Playlists/History navigation, blocked tab changes while a query was
pending, same-tab no-reload behavior, empty-playlist handling, confirmation
cancellation on navigation, and history opt-in/out only after SQL acknowledgement.
Disabled History could not be reopened. All eight scheduled assertion stages
completed; no media file loaded, no video target was allocated and catalog
changes/resets were both zero. Read-only post-run SQLite verification recorded
history disabled, all default privacy flags off, and zero playlist, playlist-item,
subscription and history rows. This is not an import/export, large-library or
human account test.

[Hashed evidence summary](evidence/2026-09-29-ui-navigation-summary.json) links
both successful logs/metadata, the earlier failure, source captures and the
boolean/count-only storage check. Private profile locations were not exported.
These runs used a bounded external display-awake fixture and do not validate
application sleep prevention, energy or performance. Earlier frozen-binary CPU
results do not qualify this changed layout and video-target geometry.

The palette now uses neutral surfaces with restrained red accents: dark
`#0f0f0f/#181818`, light `#ffffff/#f5f5f5`, and accent `#ff0033/#d9002b`.
Selected button labels remain normal text on a neutral selection surface; the
red accent marks icons/focus rather than coloring all text. The recorded sRGB
contrast calculations for normal/secondary text on canvas and normal text on
selection are 16.97/8.25/13.05:1 in dark mode and 19.17/6.29/16.52:1 in light
mode. Selected red icons on selection measure 3.72:1 and 4.54:1 respectively.
These are static token-pair calculations, not a full rendered-state contrast,
accessibility, screen-reader or IME acceptance pass.

There are now 21 unchanged official Lucide SVGs and 22 verified asset/license
hash entries, including the navigation additions from the same pinned upstream
revision. No new icon package, runtime or generated bitmap asset was introduced.


### Inspected captures from the same release

Three separate 20-second native captures used the same `f61a860f…` release and
isolated profiles; all exited 0. The root agent directly inspected every image:

- [Dark guest browsing, 1320×860](../artifacts/ui-current/browse-dark.png): a real
  public Blender search shows genuine thumbnail cards, the new navigation icons,
  and the red/neutral palette. The standard search input retains its platform-blue
  focus treatment; it was not replaced with a custom text input.
- [Light watch page, 760×600](../artifacts/ui-current/watch-light-small.png): the
  generated local clip and explicitly labeled related fixture show a coherent
  player, transport and title. Related content is below the initial viewport;
  the separate wheel/lifecycle check above proves its reachability.
- [Dark settings, 760×600](../artifacts/ui-current/settings-dark-small.png): the
  empty-profile history option is unchecked, and the scrollable settings content
  keeps the About section available below the initial viewport.

No account was connected. The images remain ignored local artifacts, including
public-query content; those links require the original development workspace.
[Exported logs, metadata and image hashes](evidence/2026-09-29-ui-current-summary.json)
identify all three captures without publishing the images or private profile
locations. One explicit screenshot readback and the external display-awake
fixture exclude these captures from performance or power qualification. This
visual inspection does not establish keyboard traversal, native screen-reader,
IME, every interaction state or other-platform support. Later clock experiments
are outside this frozen UI checkpoint and remain unqualified.


## Compact library and finite video-motion checkpoint 83a07b4

Release `c2ff7d1c0f93145713e2de039e9aa6b574051afcba5d07ed331650c025568398`
is associated with source `83a07b4b493a7d411fc21fdbc55daf529596be7e` through
115 captured input hashes. Central validation reported 243 Rust tests passing,
three ignored, 63 Python tests, strict workspace Clippy and a locked release
build. These counts describe this checkpoint, not subsequent uncommitted changes.
[Source captures, log/metadata hashes and inspection notes](evidence/2026-09-29-motion-library-summary.json)
are retained without private profile paths or screenshot pixels.

The lead agent directly inspected both native 760×600 library captures in light
and dark themes. Each shows a coherent local empty-playlist state: the name field
uses the available width, Create/Rename/Delete sit below it, no blank result list
or empty selector consumes space, and all transfer actions fit. Licensed Lucide
icons and the local/account distinction remain visible. The upstream native blue
search-focus treatment remains. Both capture processes exited 0 in isolated
profiles with no account connection. PNGs remain ignored local artifacts; their
relative locations and hashes are in the evidence index. This visual inspection
does not constitute keyboard traversal, IME, screen-reader or contrast qualification.

The separate 28.218-second library lifecycle exited 0 and reached all eight
callback stages. It exercised acknowledged history enable/disable, navigation
while worker requests were busy, empty local playlists and pending confirmation
handling. Final history state was disabled; file loads, unrelated catalog changes
and catalog resets were all zero. No new post-run database inspection is asserted
for this run. Pagination, import/export fault handling and resource budgets remain
separate acceptance work.

The two same-binary local playback lifecycle runs passed their finite motion
check: 833/9,216 grid pixels changed under default target alternation and
852/9,216 under the explicit stable-target experiment. Both also retained the
watch-page wheel/fullscreen/offscreen-progress lifecycle checks and one persistent
file load. [Stable-target evidence and limits](stable-video-target.md) records
exact timings, counters and the completed guest stream-refresh run. Two explicit
snapshot readbacks test changed composed video pixels, not continuous compositor
pacing or perceptual A/V synchronization, and are excluded from performance
samples. No power, resource-budget, other-platform or human-account gate is passed
by these finite checks.


The same release also passed the 85.198-second stable-target guest subtitle and
clear-data lifecycle. The native harness asserted a blank video image, empty
private title/channel, neutral UI clocks, completed native stop and blocked
search admission during clearing. Read-only post-exit SQLite/file inspection
found empty local collection/history tables, privacy defaults restored and no
VTT files. [Clear evidence and scope](evidence/2026-09-29-motion-stable-clear-summary.json)
exclude the private profile path and make no real-account, forensic-erasure or
performance claim.

### Raster library native capture and first regression attempt

Source `0f311dde095ca2ddd1199c734fb22096343e024a` built release
`1e2d82b1c603e64ddcc849dbc3a4629687d9458d98b2efefe7268256971d1cf4`.
All 125 build inputs were independently compared to that commit. Locked tests
passed 284 Rust cases (three external cases ignored), 112 Python cases,
formatting and strict workspace/all-target Clippy. The locked release built.

The [actual native capture](evidence/2026-09-29-raster-first-library.png) was
inspected at 1320×860 logical window size: the official Lucide icons, centered
search, sidebar, three-column raster cards and page controls render. All content
is conspicuously labeled local test fixture. Six thumbnail rectangles intersect
the viewport, with nine near-viewport images admitted; neither number is thirty.
The separate capture exited zero after 21.320 seconds and is not a benchmark.

The finite library test passed all five real 100-row SQLite page/hover checks,
then failed at the deferred End-key handoff assertion. The local player test
passed observed mute/unmute and failed the existing scroll-restoration assertion:
its newly inserted Escape test called a focus helper that intentionally resets
watch scroll. These failures are preserved, not counted as passes. Corrected
focus timing and diagnostic interaction required the new build/native repeats below.
[Source, commands-result metadata and complete scalar logs](evidence/2026-09-29-raster-first-summary.json)
associate both failed runs with this exact binary. No real account was used.

### Corrected raster keyboard and playback repeats

Source `ca21de92a94abf209b76576c8b584751d3d20b4f` has 125 captured input hashes
verified against unchanged committed Git source by the lead before native runs. The debug
executable is `f68e5cfe0cdc6581fb59fdab64706833f6656ba9468c16d97e0940a88446d66f`;
release is `96ad51a9d54014204b49925e00d8e59a47f01780513f7c011e033aee65d82645`.
The central checks passed 285 Rust tests (three explicit ignores), 114 Python
tests, formatting and strict workspace/all-target Clippy. This is source/build
association, not a reproducible native dependency closure.

| Functional diagnostic | Debug seconds | Release seconds |
|---|---:|---:|
| Offline raster pages, hover and keyboard | 44.825 | 43.887 |
| Local video, mute, editor/player keys and lifecycle | 21.723 | 21.549 |
| Public guest refresh and finite audio probes | — | 71.606 |
| Public guest caption selection and quality reattachment | — | 71.730 |

All six runs exited zero without forced termination. [The evidence summary](evidence/2026-09-29-raster-corrected-summary.json)
links the full scalar logs, metadata, source inventories and SHA-256 checksums.
No private paths, signed URLs, credentials, caption bodies or argv were exported.

The library sequence passed five actual 100-row database page/hover checks, then
deferred End/Home/PageDown delivery, rapid pending navigation, Tab exit/reentry,
resize retaining item 99 and cancellation by search. Page checkpoints still show
only six intersecting raster thumbnails and nine near-viewport references, not
thirty; final keyboard scrolling is not a settled idle sample. Local playback
passed the restored watch scroll, player/editor Escape routing, observed mute/
unmute with unchanged volume, pause/seek, resize/minimize/restore and persistent
one-file-load assertions. The two finite composed-video snapshots detected
1,291/9,216 changed grid pixels in debug and 838/9,216 in release. Readback was
diagnostic-only, not the playback path or a performance sample.

The guest refresh and caption repeats are qualified in [stream-refresh.md](stream-refresh.md)
and [captions.md](captions.md). These successes do not erase the retained first
failures, qualify human accounts, prove audible/perceptual sync, or satisfy
resource, screen-reader, IME or additional-platform gates. No new screenshot was
taken; the inspected raster screenshot above retains its original source identity.

### Neutral browsing focus and retained raster idle outcomes

The ordinary `d7c58acd` browsing build leaves initial keyboard focus on the
stable shell rather than placing an unsolicited caret in Search. Explicit
search editing retains its normal caret; slash and Tab remain keyboard entry
paths. This is production focus behavior, not a measurement-only hidden cursor.

The new settled/minimized resource pair used the actual 10,000-item fixture,
100-row current page, nine decoded near-viewport images and **six intersecting
thumbnail rectangles**. After the explicit drained-fixture marker and ten-second
warm-up, both retained all 60 samples. Mean CPU was 0.016667% of one core settled
and zero sampled deltas minimized; mean RSS was 129.933/130.073 MiB. The separate
OS footprint ledger averaged 147.606/147.731 MiB and is not additive to RSS.

The full new logs show unchanged page/thumbnail/model counters after readiness,
zero remote thumbnail starts/media loads, six total UI draws and zero video
draws. The window became occluded at 1.657 seconds; native minimized state was
confirmed after startup and again before quit at 85.075 seconds, with no
intervening restore. No new screenshot or pixel
visibility claim is supplied by these resource samples. The native-child video
diagnostic was not enabled for either ordinary browsing run.

Earlier `ca21de92` results are preserved: settled mean CPU 1.248202% failed the
idle release ceiling; its minimized attempt is invalid because it restored at
60.345 seconds, changed pages/images, finished unminimized and exited 1. Its
cause remains undetermined. The new pair does not relabel that failed evidence
or satisfy the thirty-visible, screen-reader, IME or additional-platform gates.
See [full resource scope and mean/p95/peak table](performance.md#raster-library-idle-retained-failures-and-neutral-startup-focus)
and the [complete hashed evidence export](evidence/2026-09-29-raster-idle-export.json).

## Readable application messages — frozen native validation

The footer retains its compact status line and now provides a keyboard-focusable
Message action with an explicit accessible label. It opens a shared compiled
Slint panel with the complete selected message in a read-only text control,
wrapping, text selection and keyboard-accessible vertical scrolling.
A decoder/presentation warning is shown together with the ordinary status,
rather than hiding the latter in the expanded view. The panel captures a stable
string when opened, so a worker completion cannot replace an error while the
user is reading it. There is no notification-history buffer or timer.

The popup has a visible Close action and uses Slint's ordinary Escape dismissal.
Its visibility participates in native-child overlay suppression, and the finite
native lifecycle driver now has two extra checkpoints for message hiding and
Escape/fresh paused reveal. The expanded16-stage check passed on frozen `d78332c2`, application SHA256
`e32cfa128282c781a80eeba06a8259207b07f54da946056b77185fa04efb88b9`,
in47.469seconds with clean owned-process teardown. Its14 external owning-window
captures include the Message panel at15.5seconds: complete diagnostic text is
visible and wraps, the Close action is visible, and the native video is hidden.
Escape dismisses the panel; the subsequently captured paused frame is
byte-identical to the pre-popup frame. Fullscreen and resized captures preserve
upright video and visible shared controls.

[Exact frozen source, log, capture hashes and scope](evidence/2026-09-29-ui-workflows-native-export.json)
are retained separately from the earlier a45ff67614-stage passes. This run does
not exercise text-selection operation, long-message scrolling, Close-button
activation, screen readers or real IME input; it supplies finite visual and
Escape/lifecycle evidence, not resource or production-native qualification.

## Visual refresh (2026-09-30)

The shared shell now follows familiar YouTube proportions with a quieter,
monochrome finish, as SPEC.md's restrained neutral/limited-red guidance asks:

- 56 px header: a red play tile beside a tightly set wordmark, a 40 px
  segmented search pill (a leading glass appears while it has focus), and a
  tonal account pill.
- Guide: 40 px rounded rows with a red selected icon and bold label, a
  "Your library" section with dividers, and a stacked icon-over-caption
  mini-guide below 1100 px. Captions are short; accessible labels are unchanged.
- Actions: neutral translucent state layers (`hover`/`pressed`/`strong`), pill
  shapes for tonal/filled variants, inverted monochrome primary buttons and one
  blue focus ring (`Colors.focus`) for controls, cards, parking scopes and fields.
- Cards: 12 px thumbnails, 15 px two-line titles with the channel line following
  the real title height, compact duration/playlist badges, and round channel
  avatars. Related rows now show the channel name.
- Watch page, comments, library tabs, settings, account and popups use the same
  tokens; dialogs share one opaque `Colors.dialog` surface with 16 px corners.

No hover animation was added (reduced motion is not yet observed). The video
host keeps square corners so rounded clipping does not add a per-frame layer.
Row geometry constants (card text band, 106 px related rows) are unchanged.

Validation: `cargo build --locked -p serein`, `cargo fmt --all -- --check` and
`cargo test --locked -p serein` (225 unit + 13 controlled-widget tests) passed.
Native `--snapshot` captures on the development Mac were inspected for home,
guest search (dark 1320×860 and light 1000×760 mini-guide) and settings (light).
Playback could not be presented in that capture session (display clock
unavailable), so the transport over live video, the populated watch details and
the related column still need visual confirmation with playback running.
Screen-reader and keyboard traversal passes were not repeated.
