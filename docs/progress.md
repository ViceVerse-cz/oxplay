# Implementation progress — 2026-09-29

Working native slices exist; this is **not a release-qualified product**. SPEC.md
is unchanged and remains the contract. No supported platform is declared. All
application screens use one shared compiled Slint UI and an in-process Rust core.

## Current source and validation

Connected accounts now get an optional **Recommended** Home view: the signed-in
`FEwhat_to_watch` feed is read through the existing account worker/transport.
It is shown as the normal 20-per-page video grid, with the local **Saved** Home
one chip away. Guest Home is unchanged, and startup never reads account data.
Promoted items and Shorts are excluded. Selection plays with guest access,
account rows never become the related list, and sign-out clears them. See
[local Home](local-home.md#account-recommendations-connected-accounts) and
[account provider](account-provider.md#home-recommendations-implemented-not-qualified).
Validation: `cargo fmt --all -- --check` and
`cargo clippy --locked --workspace --all-targets -- -D warnings` passed.
`cargo build --locked -p oxplay` passed. With `--no-fail-fast`,
`cargo test --locked --workspace` passed 494 tests with 4 ignored. This includes
new synthetic parser, client, worker, pager and compiled-UI chip tests. One
unchanged `oxplay-media` test failed in this environment: local libmpv rejected
its synthetic y4m fixture (engine error -17). That crate and its inputs are
untouched by this change. No real account feed, native run or screenshot was
used, so account qualification (AC-08 scope) remains pending.

The header/watch/history/comments correction centers shared header controls and
aligns the real AppKit traffic-light centers to its 48px height. The native
adapter preserves system button ownership/spacing/actions and updates existing
view frames through public APIs, with guarded ancestor assumptions. Creator
hover is bounded to avatar/name/count; subscriber counts are genuine optional
metadata. Recommended cards hide the author line. Public channel headers now
show their verified portrait and compact subscriber count through the existing
bounded worker. English date/count formatting adds no runtime dependency.

Comments automatically fetch the first public guest page after accepted selection,
using a generation-checked single-shot handoff. The persisted schema-7 setting
can cancel/hide them. Comment loading has section-local pending state and does
not disable unrelated navigation/player controls. Rows use compact author/date,
selectable body, optional genuine creator badge and likes; Copy/reply clutter is
removed. History uses dated compact video rows, watched progress and existing
cached thumbnails. It makes no artwork network request, retains bounded typed
rows and releases images on viewport/surface changes. Play/Remove targets are
separate and keyboard accessible.

Validation: `cargo test -p oxplay --test controlled_widgets --locked` passed all
13 tests, including header/search centering, bounded creator action geometry,
comment-setting acknowledgement/rollback and history artwork/removal hit separation.
Two focused comments tests, one history formatting test, two sanitized provider
profile/count tests, two comment-preference storage tests and one formatter test
passed. Formatting, strict workspace all-target Clippy, locked workspace build,
`git diff --check` and all 30 icon/license manifest hashes passed. The new refresh
icon is unchanged official Lucide from the previously pinned revision. These
are focused functional tests, not a full suite or resource qualification.

A fresh isolated offline native shell check exited 0 and produced an inspected
[shared-header capture](evidence/2026-09-29-header-shell.png). That Slint snapshot
shows centered shared controls but excludes native window decorations and cannot
verify traffic-light hit regions, native blur or system frame corners. Presenter
setup still failed with macOS display-clock error `-6661`; no moving-video or
active decoder claim follows. No performance/resource, live provider/account or
additional-platform test was run. Next: native traffic-light mouse/resize/PiP
restoration checks on an active macOS display, then live public profile/automatic
comment navigation. Release/platform/account gates remain open.

The preceding pushed source `2b047ca` [CI run](https://github.com/ViceVerse-cz/yt/actions/runs/36623317787)
failed before either job began. GitHub's annotation reports failed recent account
payments or a spending-limit restriction; it supplies no hosted test/build evidence.


The latest correction keeps playback running while browsing and moves the same
video host to a bottom-right in-app mini-player. This is separate from global
floating PiP; both use the existing window/player/context. Accepted title,
creator, description/comments and a bounded guest-related snapshot survive
navigation. Hidden thumbnail references are released independently from that
retained metadata. Creator name/avatar opens its typed public channel explicitly.
Account/private video metadata never enters the guest related model.

Video background clicks toggle pause; PiP background drags use a five-pixel
threshold so a drag cannot also pause, while transport buttons retain input.
The drag top bar, routine paused/playing text and permanent bottom status strip
are removed. Errors remain inline or accessible from Settings. The shared header
is 48px, with a 38px rounded search field. Whole-window neutral translucency and
native blur are requested by default where available, per the user's preference;
video remains opaque, and native blur remains an experimental request. See
[in-app mini-player](mini-player.md), [PiP](picture-in-picture.md) and
[appearance](window-appearance.md).

Pointer regression tests found Slint's default `PopupWindow` policy closes on
inside clicks. All app panels now explicitly close on outside clicks. Speed and
quality choices stay inline in one settings popup and retain authoritative state
until acknowledgement. The focused compiled-UI tests cover inside mouse clicks,
busy/rollback, keyboard Back/Escape, video/control hit separation, PiP dragging,
mini-player geometry/data retention and both creator hit targets. These are
headless shared-UI tests, not native rendering, decoder or live-account evidence.
Validation on macOS: `cargo test -p oxplay --test controlled_widgets --locked`
passed 10 focused tests (0 failures); `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`, and
`cargo build --workspace --locked` passed. `git diff --check` passed.
No performance/resource run, full test suite or new native capture was performed.
Current native blur/drag/layout and uninterrupted media timing still need native
functional qualification; the historical macOS display-clock `-6661` failure was
not re-tested. All production/platform/account/resource gates remain open.

The preceding correction integrates the custom 56px shared header with the real
macOS traffic lights through a transparent, hidden native titlebar and full-size
content view. The system frame/corners remain native; PiP restoration reapplies
the content-view style. Windows/Linux retain their native-frame fallback until
separate integration validation. The watch-page scrollbar no longer overlays
the player; wheel, trackpad and focus-driven scrolling remain available.

Selecting a video now opens its watch page in the same UI callback, using known
metadata and static pending-only skeletons. The old picture/metadata is retired;
initial guest/account requests retain generation/session cancellation and exact
native stop barriers. Inline failure/retry state replaces an indefinite pending
view. See [opening a video](watch-loading.md) and [window appearance](window-appearance.md).

Source inspection confirmed image HTTP/decode/cache work and media HTTP/DNS
initialization already run off the UI thread. The UI image completion loop now
publishes at most two results per callback, with coalesced bounded continuations.
This avoids draining successive completion batches in one callback; it is not a
measured attribution of the reported stalls or an extraction-speed claim.
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
and `cargo build --workspace --locked` passed on the macOS host. Initial concurrent
integration checking briefly saw a not-yet-written media method; the completed
source passes final checks. One thumbnail wake-coalescing regression was authored
and compiled, not run. No test suite, native capture, live account operation or
performance/usage run is part of this pass. The prior display-clock `-6661`
blocker has not been re-tested; integrated traffic lights, skeleton transitions,
scroll gestures, rapid selection and account-stop handoff still need targeted
native functional qualification. Existing production/platform gates remain open.

The preceding UI correction fixes the timeline/volume fill origin, removes the
large seek focus rectangle, and holds accepted absolute seek intent until the
existing native confirmation settles. Observed elapsed time remains separate.
The creator avatar/name and Share/Save actions now share a vertically centered
row; the control-visibility ellipsis is removed. Genuine public channel avatars
use verified channel metadata through a bounded, cancellable background worker;
account/local/unavailable artwork uses the attributed Lucide fallback.

Theatre mode (T, or the rectangular player control) expands the same player,
hides navigation, and puts related videos below inline description/comments.
It retains the window, presenter and playback session. Normal windows now use
native system frames: macOS supplies traffic lights and system-rounded corners.
PiP still temporarily removes decorations and restores the original frame.
Optional translucency/experimental blur remain available. History, local
subscriptions and playlists use compact top-aligned content; playlist editing
and bulk import/export/backup/clear actions sit behind explicit disclosures.

See [watch layout](watch-layout.md), [avatars](channel-avatars.md),
[local library](local-library-ui.md), and [window appearance](window-appearance.md).
Formatting, strict all-target Clippy and the locked workspace build passed.
Short offline capture attempts were blocked by macOS display-clock creation
error `-6661`; they do not validate the changed layout. Detailed scope is
recorded in watch-layout.md. No dependency,
feature or lockfile change is required; the theatre icon is unchanged from the
existing pinned official Lucide revision. Performance/usage tests are skipped
at the user's request, and release/platform qualification remains outstanding.

Source `1fe1735` is pushed. [CI run 36616692541](https://github.com/ViceVerse-cz/yt/actions/runs/36616692541)
could not start either macOS ARM64 or Linux job: GitHub reports failed account
payments or a spending-limit restriction. This is not a hosted compilation/test
failure and supplies no new test evidence. The next concrete step is visual
verification of watch/theatre/native-frame and library interactions on an active
macOS display, followed by the outstanding live guest-avatar path check.

The preceding inline-description/search/window slice passed formatting, strict
all-target Clippy and a locked workspace build. Source `57a6be1` is pushed.
[CI run 36612353509](https://github.com/ViceVerse-cz/yt/actions/runs/36612353509)
could not start either job because of the repository account-payment/spending-limit
restriction; it provides no hosted build or test result. Its custom titlebar has
been replaced by native decorations in the current correction.

The preceding visual slice follows the user's YouTube player reference: shared
controls sit at the video's bottom edge over a restrained dark fade, with a
thin red timeline, play/volume/time at left, and captions/settings/PiP/fullscreen
at right. Existing attributed Lucide assets render at 24px. Elapsed/total time
replaces the previous elapsed/remaining visual label; remaining time remains in
the accessible description. Narrow players retain elapsed-only text and mute,
with volume still available through settings.

The shared media slider keeps its incoming playback value bound, owns only a
local drag preview, handles left-button cancellation, and exposes keyboard and
accessibility range actions. The existing inactivity/focus controller holds the
transport during dragging. No new timer, player, decoder, renderer, dependency
or icon asset is introduced. `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`, and
`cargo build --workspace --locked` passed. No native, test-suite or performance
check ran for this visual pass. Native subtitle placement and compact/screen-reader interaction
remain unverified for the new bottom-edge position.

The preceding batch adds six user-facing workflows: in-app local video opening,
local subtitle attachment, playlist duplication, copy/move between local
playlists, timestamped YouTube playback, and chapter/precise-time navigation.
Three agents implemented independent slices while the primary agent integrated
and reviewed media policy, lifecycle ownership and the time dialog. See
[local media](local-media.md), [local input policy](local-media-policy.md),
[playlist organization](local-library-ui.md), [timestamp links](timestamp-links.md),
[chapters](video-chapters.md), and [Jump to time](time-navigation.md).

Picker validation runs on a bounded worker. Installing a local file retires
remote authority and waits for the existing native stop barrier. Clearing local
data synchronously cancels file selection and blocks new admission. Local native
loads get an explicit lavf container/subtitle whitelist; remote transport policy
is unchanged. Playlist writes use SQLite transactions with pinned identities and
paged destination selection. Timestamp intent travels with its resolution/retry;
chapter and time-dialog seeks recheck the current native load and account lease.
Dismissed time dialogs revoke their pending action. No dependency, lockfile,
framework feature or asset change was required.

Initial compilation caught two ambiguous collection types; strict linting caught
one collapsible conditional. Those were corrected. Final `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings` and
`cargo build --workspace --locked` all passed.
Ten focused regression tests were authored for parsing, retry routing, storage
rollback and chapter admission; they are compiled, not executed. No test suite,
native diagnostic, live provider/account operation, release build, or performance/
usage measurement is part of this user-requested fast implementation pass.
Native picker, popup/keyboard, subtitle, chapter, malformed-file/egress and
account-to-local transitions still require functional qualification. The product
and all additional platforms remain unqualified for production release.
Source `2b27782` is pushed. [CI run 36608907512](https://github.com/ViceVerse-cz/yt/actions/runs/36608907512)
could not start either macOS or Linux job because of the repository
account-payment/spending-limit restriction. No hosted compilation or test
result exists for this source. The next implementation slice is an explicit
user-controlled playback queue, retaining autoplay off and exact-load ownership.

The preceding feature slice adds three shared-UI workflows: public channel handles,
local playlist search, and explicit video-link sharing. A handle resolves through
the existing guest worker, then its real stable channel ID owns subsequent
navigation. Playlist search uses literal, parameterized title/channel matching
on the SQLite worker with bounded pages and acknowledged filter state. Sharing
constructs only canonical watch links, optionally with the current time, and
retires its pinned selection when playback or account authority changes.
See [handles](channel-handles.md), [local library](local-library-ui.md), and
[sharing](sharing.md). Formatting, strict all-target Clippy and the locked debug
workspace build passed. Initial compilation caught a Slint popup focus-scope
error; the popup now uses the supported forward-focus contract and the final
checks passed. Four focused source regressions were added and compiled, but not
executed. No test suite, live provider/account check, native diagnostic,
performance measurement or release build ran for this fast implementation pass.
Source `6b3729b` is pushed. [CI run 36607352745](https://github.com/ViceVerse-cz/yt/actions/runs/36607352745)
could not start either job because of the repository account-payment/spending-limit
blocker. No hosted compilation or test result exists for this source.

The preceding feature slice adds automatic inline control visibility: video
pointer/keyboard activity reveals the transport, and a single three-second
inactivity timer hides it during playback. Paused, hovered, focused and popup
interaction keeps controls available; entering Watch or accepting a new native
load reveals them again. No polling loop or catalog update is introduced.
Borderless PiP remembers its compact size and position within the session,
resolving the saved display against currently available monitors. Failed-stream
guest/account retries now preserve accepted pause intent and recheck the exact
failed load before replacement, preventing a stale retry from taking over newer
playback. See [PiP and controls](picture-in-picture.md) and
[recovery](guest-recovery.md).

That earlier slice followed the requested fast feature workflow. `cargo fmt --all -- --check`,
`cargo check --workspace --locked`, strict all-target Clippy and the locked debug
workspace build passed. Initial compilation caught incompatible Slint layout
constraints; those were corrected before the passing checks. New runtime
interaction, monitor transitions and recovery qualification remain pending.
No test suite, native diagnostic, performance/usage benchmark or release build
was run for these changes. Prior results below apply to their recorded source.
Source `dd26ab4` is pushed. [CI run 36605325904](https://github.com/ViceVerse-cz/yt/actions/runs/36605325904)
ran zero steps: both jobs were denied by GitHub's account-payment/spending-limit
check. Local checks above are the only build/lint evidence for this slice.

The preceding slice propagates identity loss during post-write account
reconciliation, clearing stale connected UI and account playback. Unconfirmed
writes retain a generic in-memory warning across explicit reconnect, without
replaying requests or transferring private identifiers to another session.
See [account provider](account-provider.md). Automatic guest stream refresh now
checks accepted pause intent and exact active load before extraction, closing
the asynchronous pause/navigation gap; see [pause intent](pause-intent.md).
The library diagnostic has 22 finite stages, including actual 101-entry storage
and video-page Rename cancellation. A deterministic worker regression saturates
both channels and verifies shutdown preserves accepted writes and excludes
rejected work. Native UI saturation remains distinct from that worker test.
Local validation passed 448 Rust tests (four explicit integrations ignored),
163 Python tooling tests, formatting, strict all-target Clippy and locked debug
and release builds. The debug native run passed all 22 checkpoints; post-exit
SQL confirmed the unchanged names, two playlists, 101 items in the original
playlist and history off. The release check also passed all 22 stages at
760×600/light with the same persisted results and inspected confirmation capture.
Both runs exited zero and their owned process groups were absent afterward.
No performance/usage test was run. [Public functional evidence](evidence/2026-09-29-library-page-functional.json)
records the exact source/binary hashes and complete native logs.
Source `068bc94` is pushed. [CI run 36603317441](https://github.com/ViceVerse-cz/yt/actions/runs/36603317441)
failed before either job ran a step; both annotations cite failed account payments
or a spending-limit issue. No hosted build/test result exists for this source.
Its local exact-commit source preview and all asset checksums verified; no tag,
release or workflow dispatch was created.

The preceding slice fixes acknowledged state in captions, quality, Appearance and
catalog filters, and gives the local Save dialog an explicit destination draft.
It also prevents diagnostic themes leaking into saved preferences through volume
updates. See [shared controls](controlled-widgets.md). PiP restoration now prefers
the main window's original connected monitor; actual multiple-monitor behavior
remains unqualified.

Caption Off now waits for older native subtitle operations, its exact command
reply and a fresh native sid query. The UI retains its observed track until that
correlated completion, including at keep-open EOF. Stop and replacement revoke
stale authority. See [caption ordering](captions.md#correlated-off-completion-and-controlled-selection).
The final local suite passed 443 Rust tests (four explicit integrations ignored),
163 Python tooling tests, formatting, strict Clippy and the locked debug build.
This includes five headless tests of the actual compiled shared controls and
native null-output caption regressions. The first integrated suite passed 441
before review added the EOF fix and its two regressions. The final locked release
build also passed. The release public guest-caption check passed all five stages,
including exact Off acknowledgement and paused quality reattachment. The release
PiP check passed nine stages and three correlated native probes on the same
window, restoring its original frame/position; its inspected capture retains
inline controls and local subtitles. These do not qualify multiple displays or
account playback. Owned native test processes were reaped. Source `dfc30db` is
pushed. [Its CI run](https://github.com/ViceVerse-cz/yt/actions/runs/36601158670)
failed before either job ran a step; both annotations cite failed account payments
or a spending-limit issue. No hosted build/test result exists for this source.
The exact-commit source preview includes the new shared-control tests, native
Off barrier and lockfile; all asset checksums verified. No tag or release was
created, and no performance/usage test was run.

The preceding slice moves playback controls inside the video and makes PiP
borderless, with a shared drag area and restoration of the original native
frame. It also fixes created-playlist selection beyond the first 100 through
bounded bidirectional collection windows. See [PiP](picture-in-picture.md#inline-controls-and-borderless-update)
and [collection windows](library-reconciliation.md#created-playlist-windows-beyond-the-first-page).

The local workspace passed 428 Rust tests (four explicit integrations ignored),
formatting and strict Clippy. An existing caption test raced FILE_LOADED against
PLAYBACK_RESTART; four caption fixtures now wait for the exact paused load's
restart, preserving all caption/lease assertions and production admission rules.
All seven focused caption tests passed before the complete suite. The 14-stage
native collection exercise passed in debug; read-only SQL confirmed 206 final
playlists, the expected create/delete results and history off. The ordinary
light watch capture visibly places transport over video. The debug PiP repeat
passed all nine stages and three own-PID native frame/stacking probes; same-window
borderless geometry and normal-frame restoration were verified. The first PiP
attempt failed initial playback readiness before any PiP operation and remains
recorded. Final locked debug and release builds passed. The release PiP check
passed all nine stages and three native probes, including borderless 480×270
geometry, readable subtitle/control capture and restoration. The release
collection check passed all fourteen stages at 760×600/light with the same
persisted results. All owned native test processes were reaped. Source `aba082e`
is pushed. [Its CI run](https://github.com/ViceVerse-cz/yt/actions/runs/36598316363)
failed before either job could execute: both annotations report failed account
payments or a spending-limit issue. No remote build/test result exists for this
source. The exact-commit source preview includes both new diagnostic/regression
modules and PiP changes; all asset checksums verified. No tag/release was created
and no performance or usage test was run.

The preceding slice fixes [playlist keyboard editing and confirmation focus](library-reconciliation.md#keyboard-name-editing-and-confirmations).
The editor stays focused/read-only during writes, Escape safely cancels unsubmitted
renames, and confirmations focus Cancel then restore their originating control.
Accepted video pagination retires stale Rename UI; rejected playlist selection
returns both the dropdown index and label to its acknowledged identity, including
after deferred callbacks. Shared controlled widgets also keep saved playback,
quality, artwork and history preferences synchronized. All screens remain shared
compiled Slint; no platform-specific application frontend was added.

Local validation passed 414 Rust tests (four external integrations ignored), all
163 Python tooling tests, formatting and strict Clippy. A PID readiness race in
an existing DNS test fixture was fixed with a complete-record handshake, keeping
production supervision and reaping assertions unchanged. The final debug native
check passed twelve injected-key stages and exited 0; its inspected capture shows
one correct confirmation focus outline. An earlier strengthened check caught the
disabled-control focus bug before the passing fix. Read-only SQL confirmed the
two expected persisted playlists and history off. The final debug and release
checks also passed deferred selector rollback and a subsequent real accepted
selection; the compact 760×600 light release capture was inspected and shutdown
returned 0. Both native processes were reaped. Source `fa67cba` is pushed.
[CI](https://github.com/ViceVerse-cz/yt/actions/runs/36594008344) could not start
either job: GitHub reported failed account payments or a spending-limit issue.
No remote test/build result exists for this source; prior green runs remain
historical evidence. The exact-commit source preview and all asset checksums
verified locally; no tag or release was created.
This tests Slint key delivery in a native window, not OS/physical keyboard, IME
or screen-reader qualification. No performance/usage benchmark was run.

The preceding slice adds a [bounded guest artwork cache](artwork-cache.md), with
Off/32/128/256 MiB Settings controls and schema-v6 persistence. Saved Home videos
read it without HTTP fallback. Image identities survive unchanged refreshes;
reindexed rows reject old completions. Clear local data waits for caption and
artwork cleanup before SQLite deletion, then discards retained public rows.
No account-derived artwork or credential material enters this cache.

The integrated local suite passed 411 Rust tests (four external integrations
ignored), all 163 Python tooling tests, formatting and strict Clippy. The native
debug Home exercise passed twelve stages including cached artwork, unchanged
refresh, navigation, local mutations and the actual clear callback. Its
1000×720 dark screenshot was inspected. Post-exit read-only inspection found
schema 6, zero saved entries/collections and only the empty cache lock file.
Evidence is in `artifacts/artwork-debug-v1`. The native process exited 0; the
first post-exit inspection used a wrong table name, then was corrected and
completed without changing the database. The locked release build passed the
same twelve stages at 760×600/light, exited 0, and left no cached artwork or
saved entries. Its capture and the ordinary light Settings screen were inspected;
evidence and executable hash are in the cache notes. Source `c6e5c14` is pushed;
[CI](https://github.com/ViceVerse-cz/yt/actions/runs/36588787500) passed macOS
(411 Rust tests, four ignored) and Linux (393, one ignored), plus 163 Python tests,
formatting, strict Clippy and locked release compilation on each. The exact-source
archive and checksums were verified locally; no tag or release was published. Performance/usage tests remain paused at the user's request.

The preceding slice implements [local-first Home](local-home.md). Home is now a
distinct route with a bounded, deduplicated page of saved local videos; clicking
Home from a public catalog returns to local content. SQLite schema v5 adds the
lookup index without changing existing data. Local reads have their own tickets,
and committed mutations invalidate old cursors. Deferred refresh survives busy
playback and cancellation without a recurring timer or automatic retry loop.
The shared catalog/group models reconcile unchanged pages without notifications;
metadata/membership changes retain model identity and restore focused video IDs.

The final integrated test suite passed395 Rust tests (four ignored), all 163
Python tooling tests, formatting, strict Clippy and the locked release build. Debug native Home passed all11 offline functional stages and
exited cleanly: three bounded pages, unchanged Home rereads, navigation, committed
Save/Remove/Import/Delete and correct updated rows. Its 1000×720 capture was
inspected; the synthetic labels remain visible and no remote thumbnails/playback
started. These direct worker mutations do not claim coverage of the Save dialog
or native picker. Evidence is in `artifacts/home-v1`. The final release also
passed all11 stages and clean shutdown at760×600 in the light theme; its capture
was inspected and its executable hash is recorded in the Home notes
(`artifacts/home-release-v1`). An ordinary fresh-profile release launch also
exited successfully, with its compact light-theme empty state visually inspected
(`artifacts/home-empty-v1`). The ordinary empty profile retained schema5 and zero
local content rows. Source `42c937e` is pushed; its
[CI run](https://github.com/ViceVerse-cz/yt/actions/runs/36585092789) is green:
macOS395 Rust tests (four ignored), Linux377 (one ignored), and each passed163
Python tests, formatting, strict Clippy and locked release compilation. The
exact-commit source archive and checksums were verified locally, including the
Home modules and schema-v5 migration. No release/tag was published.
No performance/usage benchmark was run. Remote-account/platform/packaging and
resource acceptance gates remain open.

The preceding slice adds [floating picture in picture](picture-in-picture.md) to the
same shared Slint window and implements explicit authenticated playback recovery.
PiP preserves the existing player, window and presenter; P/Escape/close restore
the main layout. Official Lucide supplies the new icon. Debug and release native
nine-stage checks passed on macOS, including pause/seek/resume/resize, and a
separate owned-window query verified the actual floating layer and restoration.
Both captures retain visible subtitles and the diagnostic fixture label. Native
Wayland and the experimental native-child presenter keep PiP unavailable.

The existing real account-provider flow now has clearer Google/YouTube browser
sign-in and explicit session-file import instructions. Failed account playback
can retry through its authenticated resolver with exact load/session authority,
provider cooldowns and no guest fallback. Final review also corrected fractional
retry-deadline wakeups in both guest and account paths so a quiescent failed file
cannot retain a permanently disabled Retry control. Synthetic regressions cover unverified
identity, repeated authorized resolution, cooldowns and revocation. The 900×720
account screen was visually inspected in guest mode and exited cleanly
(`artifacts/account-ui-v2`); an earlier invalid numeric page argument was rejected
before startup. No human credentials or live account were used. Account acceptance
remains open and must be locally authorized.

Local validation passed: locked workspace tests (379 passed, four explicitly
ignored external integrations), strict workspace/all-target Clippy, formatting,
locked debug and release builds. Release executable SHA256 and native evidence
are recorded in the PiP notes. Source `c195894` is pushed to `ViceVerse-cz/yt`.
Its [CI run](https://github.com/ViceVerse-cz/yt/actions/runs/36581318438) passed both
jobs: macOS 379 Rust tests (four ignored), Linux 361 (one ignored), and each
passed 163 Python tests, formatting, strict Clippy and locked release builds.
The exact-commit source archive was also generated locally and checksum-verified,
including the new PiP source/icon/license and excluding local artifacts.
No GitHub release or tag was created.
No performance or usage benchmark was run.

CI and the initial repository push are complete. The
new [workflows](ci-release.md) check macOS ARM64 and Linux compilation/tests
with the pinned toolchain, and provide an explicit manual draft source preview.
Binary releases remain blocked on existing packaging/licensing/platform gates.
Local actionlint, shellcheck, Rust formatting and all 163 Python tooling tests
pass, including seven exact-commit source-release regressions. The initial
snapshot `97d0263` is pushed to `ViceVerse-cz/yt`; 45 prior development commits
remain on the local `archive/local-development` branch. The
[first remote run](https://github.com/ViceVerse-cz/yt/actions/runs/36573332667)
passed macOS formatting, strict Clippy and all 369 Rust tests, then exposed the
CI-selected Python 3.12's missing macOS `os.waitid` primitive. CI now pins the
locally tested Python 3.14.7 and checks supervision support before compiling.
Linux's official libmpv source build/API check and strict production-feature
Clippy passed, followed by 168 app and 10 domain tests. Media tests then found
that disabling Lua at build time removes options required by the application's
script-disable policy (37 media passes, 20 initialization failures). The CI mpv
build now includes LuaJIT; runtime scripts remain explicitly disabled. Neither
failure is hidden by skipping tests. The
[corrected run for 036da68](https://github.com/ViceVerse-cz/yt/actions/runs/36574672656)
is green on both runners: macOS passed 369 Rust tests (four ignored), Linux
passed 351 (one ignored), and each passed all 163 Python tests, formatting,
strict Clippy and locked release compilation. The ignored integrations remain
explicitly external/human-authorized. No performance or usage test was run.
The source archive was generated and checksum-verified locally from the actual
initial snapshot; manual GitHub draft creation has not been dispatched. No
release/tag or binary package was published. Production and native platform
qualification remain open.

Performance/resource experiments are paused at the user's request. Current work
focuses on product usability: generation-scoped catalog loading/error states,
bounded empty-state text, clearer modal dialogs, an explicit draft-preserving
playlist rename flow, and observed playback loading/buffering/seeking labels.
Media failures now explain the engine's static error category without exposing
stream URLs. The experimental native child suppresses itself for Slint's built-in
popups as well as explicit dialogs. Official Lucide assets remain unchanged.
The home screen now offers explicit Music, Science and Gaming searches without
fetching anything automatically. The shared Play control can restart a failed
guest file through fresh resolution, retaining exact load identity, account
isolation and provider backoff. Nonretryable resolution failures disable that
restart for the failed load; account media is never retried as a guest.
Formatting, the locked development build, strict workspace/all-target Clippy
and all369 Rust tests pass (four explicit external integrations ignored).
The first integrated run exposed a pre-existing asynchronous test race: a
command reply plus the old Paused snapshot did not confirm seek completion.
The regression now waits for the actual seek/pause readiness barriers, with
the original timeout and production guards unchanged. The native offline error
check passed all11 stages, exited cleanly and reaped both owned process groups.
Its760×600 dark-window capture was inspected: the fixture label persists,
the video-error context is correct, the dialog is readable and the background
is dimmed. Working-tree development evidence is retained in
`artifacts/usability-v1`. This does not validate live failures, the new rename
flow through physical keyboard input, delayed native tooltips, performance or
additional platforms.
The follow-up home-screen capture was also inspected at760×600: all three
search shortcuts fit, the default guest/privacy state is visible, and the
application exited cleanly. Five new deterministic retry regressions pass;
actual failed-stream-to-success native recovery remains pending. No performance
or usage measurement was run after the user's instruction to pause them.
The locked release build of source `50822c1` completed successfully; the updated
executable is `target/release/oxplay`. Earlier source `ea237f0` also passed its
locked release build. These are runnable development releases, not distribution
or cross-platform qualification.

The next source slice implements explicit, generation-scoped guest Retry and
incremental local-library updates. Typed page-read tickets reject stale success
and failure responses; unchanged rows emit no Slint model notifications. The
Save diagnostic now checks actual acknowledged database reads independently of
model changes. The integrated suite passed354 Rust tests (four explicit external
integrations ignored), including157 app tests. Formatting and strict
workspace/all-target Clippy pass; four actual model-listener regressions also
passed after a test-only lint correction. The first compilation's unnecessary
`Debug` derive failure and the initial lint finding are retained in the work
logs. The frozen `578a14a` release built in2m05s with all141 committed inputs
unchanged; SHA256 `47cd30afd3f5453083d0af276af9dcd664b9cbc4a9a4a2ea54d3ee304248a404`.
Its first Retry diagnostic passed all11 functional stages in22.052s, but actual
PNG review found a missing fixture label overwritten by decoder updates. That
failure is preserved. Fixture identity now has a separate UI property, with
native assertions for persistence and full-message inclusion. All157 app tests
pass after the correction. Corrected native Retry and Save/library runs remain
pending.
See [guest recovery](guest-recovery.md) and
[library reconciliation](library-reconciliation.md).

Newest natively validated source `d78332c23602b1bfaec87688eccba657a208b9be` implements related-video
keyboard navigation, transactional first-playlist Save and a readable status
dialog. The locked release build completed in 1m56s; all138 build inputs matched
the commit before and after building. Release SHA256 is
`e32cfa128282c781a80eeba06a8259207b07f54da946056b77185fa04efb88b9`.
The workspace suite passed336 Rust tests (four explicit external integrations
ignored); final app tests passed139 after the last UI changes, with formatting
and strict workspace/all-target Clippy passing. Unchanged Python tooling last
passed156 tests on d7c58acd.

This release passed the genuine public-video local Save workflow in77.177s:
all eight stages, one committed playlist/video after repeated Save, rejection
after the captured playback was stopped, and clean owned-process shutdown.
Read-only post-exit SQLite checks confirmed privacy defaults and no history,
follows or account session files. Its dark760×600 popup capture was inspected.
Related-video navigation passed all16 release stages in43.747s, including
compact nested scrolling, fullscreen restoration and actual player focus after
retirement; the owned fixture was removed and both process groups were absent.
These are finite functional checks, not account, screen-reader or performance
qualification. See [local Save](local-playlist-save.md) and
[related focus](related-focus.md). The expanded native-child lifecycle capture
also passed all16 stages in47.469s, with14 owning-window captures and clean
owned-process/fixture teardown. The new full-message dialog was readable with
the native child suppressed; closing it restored the identical paused frame.
Fullscreen and compact resize captures were inspected. This restricted local
diagnostic does not qualify tooltips, subtitles, audible synchronization or
arbitrary media for the native-child presenter.

## Previous media and resource checkpoint

Previously validated source `a45ff67692e0f25e22c188dc1583c3490d94a542` passed
324 Rust tests (four external integrations ignored in the ordinary suite),
formatting, strict workspace/all-target Clippy and the locked release build
(1m47s). All135 build inputs matched that commit before/after build. Release
SHA256 is `0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9`.
The unchanged Python tooling previously passed156 tests on d7c58acd.

The pause-intent/EOF correction, shared Action variants, fullscreen/compact mute
and playback settings, and finite geometry observations are now built and
natively exercised. Default local playback passed in22.456s; the10,000-row raster
library regression passed in43.495s. Both restricted native-child lifecycle runs
passed all14 checkpoints, including paused fullscreen exit, overlay suppression,
resize, minimize/restore and stopped blanking. Fourteen external owning-window
captures and one separate light760×600 Slint snapshot were inspected. This is
finite functional evidence, not complete input, subtitle or A/V qualification.
See [corrected exact evidence](evidence/2026-09-29-native-child-corrected-export.json).

The previous d7c58acd native-child run **failed stage23** when fullscreen
hide/show overwrote user pause from stale observed state. Its failure remains
in [first-attempt evidence](evidence/2026-09-29-native-child-first-export.json).
The four separate synthetic TLS/Keychain/DNS integrations passed on d7c58acd;
no real account was used. These historical checks are not a rerun on a45ff676.

A same-binary default/native/native/default comparison completed with all four
admission checks and clean exits. CPU means were50.658/46.437/43.568/48.784%
of one core, with zero additional warm VO/decoder drops. Native presentation
reduced Slint draws from3838–3839 to239–240 per minute, but both native means
still miss the25% CPU target. The candidate remains restricted and is not the
default. See [measurement scope](performance.md#native-child-comparison--corrected-a45ff676).
The later UI workflow changes above are excluded from these older resource
measurements. Related navigation's three earlier debug failures remain retained,
including the evidence for the pinned-Slint disabled-focus flag workaround.

A subsequent matched private-mpv timer ON/OFF/OFF/ON experiment on the restricted
a45 native path measured44.759/36.665/36.026/44.541% CPU means, with zero warm
VO/decoder-drop increments. Both OFF repeats were lower, but still missed the25%
target. The app and installed library remain unchanged; this is not production
qualification or an energy result. Complete samples, loaded-library functional
proof and pixel-equivalence checks are in the
[timer experiment](experiments/mpv-pass-timers.md#native-child-matched-onoffoffon-resource-result).

That frozen media checkpoint includes:

- Neutral startup focus and event-driven offline thumbnail quiescence, plus
  bounded sampler readiness admission before warm-up.
- A compiled, restricted macOS native-child video presenter and finite lifecycle driver,
  retaining all controls in Slint. It admits only a verified owned copy of the
  synthetic MP4; the existing presenter remains default. See
  [experiment scope](experiments/native-video-child.md).
- Bounded URL parsing and rejection of ambiguous duplicate video identifiers.
- Atomic no-clobber publication of completed SQLite snapshots; see
  [backup ownership and tests](local-backups.md).
- Stronger synthetic TLS, isolated Keychain and actual DNS-helper integration
  checks; see [external checks](external-integration-checks.md). No real account
  or user credentials have been exercised.
- Optional separate [macOS footprint](macos-footprint.md) observations and a
  bounded [Deno source inventory](deno-source-inventory.md) tool. The footprint
  ABI/self-query passed; separate application footprint observations are recorded
  in the idle and playback evidence, with attribution limits.

The added direct app `ring` and media build `cc` dependencies use versions already
present in the lockfile. Cargo refreshed the two package dependency lists with
only those edges; no dependency version changed. All subsequent checks use
`--locked`. The historical binary results below retain their original counts.

## Earlier validated source and online slices

Source `ca21de92a94abf209b76576c8b584751d3d20b4f` produced release
`96ad51a9d54014204b49925e00d8e59a47f01780513f7c011e033aee65d82645`.
All 125 build-input hashes were checked against the commit. Locked workspace
checks pass **285 Rust tests** (three external tests intentionally ignored),
**114 Python tests**, formatting, strict workspace/all-target Clippy and release
build. Native release tests pass: local playback/mute/editor shortcuts/fullscreen
scroll (21.549 s), 10,000-item raster library paging/hover/virtualized keyboard
(43.887 s), genuine guest refresh with fresh-position/audio checks (71.606 s),
and genuine guest caption selection/cache/paused quality reattachment (71.730 s).
These are functional tests, not resource, screen-reader, account or A/V-perception
qualification. First failed attempts remain in [UI evidence](ui-validation.md).

The earlier frozen source ee59eaa47289382b6784547e8bece1b38e5e60a5 produced release SHA256
464fdcf1432f1425698cf2892f2fd7f3e942c4e132dfb176c5c45bcb5582b0a8.
All 119 captured build inputs independently match that commit. Central checks
passed 261 Rust tests, with three explicitly ignored external tests, formatting,
strict workspace/all-target Clippy and the locked release build. Native local
lifecycle passed in 21.071 seconds, including startup player focus and finite
motion (838/9,216 changed grid pixels). Earlier failed focus attempts remain in
[focus evidence](focus-intent.md); their results have not been overwritten.

The one-hour local mixed-use soak completed: 60 cycles, 12 loads, 120 playing
checkpoints and 3,600 resource samples, with clean process exits and no observed
owned survivors. Hardware decoding remained VideoToolbox and unrelated catalog
notifications remained zero. Aggregate RSS averaged 232.690 MiB and peaked at
257.313 MiB; twenty predeclared phase-matched minute pairs showed median growth
of 9.456%, with four pairs above 10%. This is not a leak-free pass. The separate full-hour process-level attribution repeat also completed on that
preserved binary: 3,600 samples, clean exit and no observed owned survivors.
The completed public export attributes median matched absolute RSS growth of18.551MiB to the app and2.773MiB to its temporally attributed decoder service; this narrows attribution without proving a heap or graphics leak. The memory gate remains open.
The mixed-use CPU mean of 34.539%
is not a steady-playback qualification. Account/search/extractor/fallback work
and the large-raster workload were excluded. See [attribution](soak-attribution.md).

The preceding f6dea1361cdd59d2f7a26315a6002e5c2a25fc85 checkpoint also passed
63 Python tests. Its isolated native preference write (24.772 seconds) and
restart/clear verification (24.284 seconds) passed: saved 720p/1.5×/37, restored
on restart, changed to 480p/2×/62 and reset to 1080p/1×/100. Post-exit inspection
confirmed schema 4, empty content tables and privacy defaults. Its local focus
test failed; the corrected ee59eaa result above is separate. See
[playback preferences](playback-preferences.md).

## Implemented and observed slices

- Slint runtime/compiler are coherently locked to verified upstream master
  revision cf3b07d4917e6759a63b0c03913a2594ec653414. macOS Winit/FemtoVG and
  Apple M1 OpenGL 4.1 have been observed. See [dependencies](dependencies.md).
- Embedded local and genuine public YouTube video/audio, pause, seek, resize,
  fullscreen, selected captions, quality replacement, expiry recovery and clean
  shutdown have source-associated native evidence. Persistent GPU targets use
  borrowed textures; macOS uses a demand-paced native display clock. Direct
  VideoToolbox H.264 decoding is observed. These facts do not establish an
  end-to-end zero-copy path or perceptual A/V synchronization. See
  [video integration](video-integration.md), [captions](captions.md),
  [replacement](stream-refresh.md) and [media networking](media-network.md).
- Real cancellable guest search and typed channel/playlist pages, bounded
  supervised extraction, public descriptions/comments and known promoted-renderer
  filtering exist. Native details/comments checks returned genuine pages.
  Guest and authenticated live ad tests remain distinct and incomplete. See
  [guest catalog](guest-catalog-ui.md) and [comments](comments.md).
- The shared UI has a 56px header, centered search, 232px sidebar/72px rail,
  responsive virtualized cards, dominant watch page and light/dark/system themes.
  Icons are unchanged official Lucide SVGs with upstream revision, checksums and
  full ISC/Feather MIT notices. Real guest, small watch/settings and library
  captures have been inspected. See [UI validation](ui-validation.md).
- SQLite local collections/follows, opt-in retained history, paginated reads,
  import/export, backup and coordinated clear use an asynchronous bounded worker.
  Isolated native navigation and clear checks passed. This does not yet qualify
  every library operation or the large-raster resource workload.
- Explicit account import/consent, identity verification, capability reporting,
  subscriptions/playlist reads, mutations and reconciliation are implemented.
  macOS persistence uses an encrypted envelope and Keychain key. Disconnect
  invalidates authenticated work and media leases. Account playback uses guarded
  in-process readers; guest fallback is never automatic. Account captions,
  comments and private local persistence are unavailable. **No real account or
  credential has been used; no account acceptance gate has passed.** See
  [account media](account-media.md).
- The historical bc2e53f developer bundle passed offline helper probes,
  signature/inventory checks and DNS-backed public playback using only bundled
  or system libraries. No tracked helper survived. It predates current changes;
  this development-host result is not clean-machine or release qualification.
  See [packaging](packaging.md).

## Current implemented changes and scope

The corrected release includes an explicit offline 10,000-item SQLite/raster fixture; observed mute
state with the official Lucide volume-x icon; software/copying/unknown-decoder
warnings; a guard for shortcuts while an editor owns focus; serialized seek
confirmation with bounded timeout recovery; full published-comment-prefix
validation; retained catalog pagination after invalid input; and keyboard
reachability/focus restoration for virtualized feed cards. The earlier search
typing checks did not detect Escape bubbling from text editors, so they do not
qualify the new guard. See [keyboard controls](keyboard-controls.md) and
[library fixture](library-resource-fixture.md).

The first native library attempt passed five actual 100-row page/hover phases
(zero unrelated model changes; six intersecting thumbnails), then failed the
off-screen End focus assertion. The separate local test passed observed mute/
unmute, then failed because its new Escape setup reset the saved scroll under
test. Both issues were corrected and the separate current release runs pass; neither
failed lifecycle is counted as passing.
A separate native raster capture exited zero and was inspected. All first-attempt
logs, exact125-input provenance and the labeled capture are retained in
[UI evidence](ui-validation.md).

The completed-local-soak analyzer has run against the full evidence. Other new
evidence tools have synthetic coverage: an
isolated preparation plan for the proposed mpv GPU-timer experiment, and an
opt-in locked Git source exporter. The immutable Slint tree export has now passed twice with byte-identical archives
for all fourteen selected locked Git packages. Private timer ON/OFF builds have
compiled, installed and passed actual loaded-library/local-playback checks.
Their isolated ON/OFF/OFF/ON comparison measured mean CPU
52.168/46.620/50.100/50.943% of one logical core and mean aggregate RSS
190.026/187.033/187.301/189.722 MiB. One OFF repeat still failed the 50% ceiling;
all missed the 25% target. The patch remains diagnostic-only and the normal
application still links installed mpv. No complete corresponding-source or
licensing-clearance result is claimed.
See [timer proposal](experiments/mpv-pass-timers.md) and
[source coverage](source-coverage.md).

## Measured resource result and open gates

The actual raster fixture's first settled-library sample on `ca21de9` measured
1.248% mean one-core CPU and 130.656 MiB mean RSS. It fails the 1% idle ceiling
even with only six intersecting thumbnails; it does not exercise the required
30-thumbnail acceptance case. Its separate minimized attempt restored at
60.345 seconds and correctly failed the final minimized-state assertion. That
mixed invalid run is excluded from minimized performance claims. Both raw runs
remain in `artifacts/raster-native-v2/resources` and the public export linked below.
Source review found startup focus forwarded into the blinking search editor.
The neutral-focus fix is built in d7c58acd. Its separate settled repeat measured
0.0167% mean one-core CPU,129.933MiB mean RSS,147.606MiB separately measured
physical footprint, and six total UI draws across85seconds. All60 footprint
samples were available; the settled model/generation remained unchanged after
readiness. This is only the observed six-thumbnail viewport, not the30-visible
gate. The minimized repeat remained hidden through shutdown with0% sampled CPU,
130.073MiB mean RSS and147.731MiB mean footprint; zero at this sampling resolution
is not proof of zero work. [Full public evidence](evidence/2026-09-29-raster-idle-export.json)
retains both corrected runs and both prior failed/invalid attempts.

The latest completed C2/83a07b4 stable-target comparison used off/on/on/off.
Mean CPU was **54.385/44.431/54.146/53.724% of one logical core**; mean aggregate
RSS was **189.306/189.365/189.311/189.352 MiB**. Every warm interval added zero
VO/decoder drops. Fewer image publications did not yield a repeatable CPU gain:
three means exceeded the 50% release ceiling, and all missed the 25% target.
The experiment remains off. Clock staging and optional UI caches also remain
off; their tested CPU gains were not repeatable. These runs used 30 labeled
related rows with empty thumbnails. See [performance](performance.md),
[stable targets](stable-video-target.md), [clock staging](clock-staging.md),
[related cache](related-cache.md) and [search cache](search-cache.md).

Historical standalone and browser measurements are retained in
[standalone baseline](standalone-baseline.md) and [browser baseline](browser-baseline.md).
Geometry/source differences, shared memory/services and incomplete browser
endpoint auditing prevent exact subtraction or superiority claims. Energy is
unmeasured. Empty-shell idle samples do not pass the settled-thumbnail gate.
SPEC budgets and quality requirements have not been relaxed.

Release blockers remain: repeatable playback/idle ceilings; the required large
raster library and full soak workloads; perceptual A/V and color/subtitle
qualification; keyboard/screen-reader/IME/DPI checks; authorized human account
and non-Premium advertisement tests; whole-process networking/security coverage;
portable helpers and protected storage; clean installation; complete license
notices/source closure and distribution review. The missing dispatch notice,
objc2 SDK-policy questions and Deno embedded-component inventory remain open.
The selected application/Slint GPL route has not changed. See
[licensing](licensing.md), [licensing follow-up](licensing-followup.md),
[source coverage](source-coverage.md) and [platform matrix](platform-matrix.md).
Windows, Linux/X11 and native Wayland have no runtime qualification here.

## Next concrete work

Next add a shared keyboard-shortcut reference and explicit local-file opening
through the existing player. Continue account/library usability work. The new
control visibility, remembered PiP geometry and retry handoff still need runtime
qualification when functional testing resumes. Actual UI queue-saturation coverage remains separate from the passing
real-worker test. The shared controlled-widget and video-page Rename checks do
not replace OS keyboard, IME or screen-reader qualification. Physical keyboard/IME/screen-reader qualification
remains open despite the passing injected-key slice.
Real-account identity, reads, explicitly authorized writes and expiry still need
local human qualification; synthetic checks cannot pass those gates. Resource
experiments remain paused. The required30-visible-thumbnail geometry,
complete overlay/subtitle/A/V checks and full production soak remain open.
Keep unavailable human/platform tests explicit; synthetic or guest results never
substitute for them. Historical evidence remains linked and preserved in Git.
