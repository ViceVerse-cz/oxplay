# Implementation progress — 2026-09-29

Working native slices exist; this is **not a release-qualified product**. SPEC.md
is unchanged and remains the contract. No supported platform is declared. All
application screens use one shared compiled Slint UI and an in-process Rust core.

## Current source and newest native validation

CI and the first repository push are the current user-requested slice. The
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
executable is `target/release/serein`. Earlier source `ea237f0` also passed its
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

Continue account-scoped playback recovery through the existing authenticated
resolver, preserving its session/selection authority, active quality and manual
backoff without entering guest recovery. Complete native keyboard handling
checks for playlist creation/renaming and failed-stream restart. Resource
experiments remain paused. The required30-visible-thumbnail geometry,
complete overlay/subtitle/A/V checks and full production soak remain open.
Keep unavailable human/platform tests explicit; synthetic or guest results never
substitute for them. Historical evidence remains linked and preserved in Git.
