# Local mixed-use soak diagnostic

The new `soak_smoke.rs` module is an explicit, bounded **local functional** soak,
with a minimum duration of 60 minutes and maximum of 240 minutes. CLI/main
integration is implemented. Focused schedule/admission tests pass. The first
[completed full-duration local run](soak-local.md) passed its lifecycle assertions;
memory growth and remaining workload coverage leave the full SPEC gate open.

This slice addresses the local repeated-playback/navigation/lifecycle portion of
SPEC 12.3–12.4. It does **not** substitute local fixtures for live search,
extraction cancellation, expired signed URLs, account switching, codec fallback,
missing hardware support, the 10,000-item library case, or human account tests.
Those remain separate qualification runs. It makes no claim about perceptual
audio/video synchronization or resource ceilings.

## Admission and integration

Explicit CLI: `--soak-minutes 60 --local ABSOLUTE_MOVING_FIXTURE
--demo-related --data-root NEW_DIRECTORY`. Default is off. The CLI enforces:

- accept only 60–240 integral minutes;
- require an explicitly selected, prevalidated absolute local video and exactly
  30 labeled demo-related rows, with no remote/account startup content;
- require a new isolated data root, so preferences/history/account state and
  thumbnails from ordinary use cannot influence the run;
- reject other smoke diagnostics, snapshots, startup minimization, UI-page/theme/
  size overrides, and any quit deadline other than the fixed watchdog;
- optionally allow an explicit local subtitle fixture, reapplied on reload;
- require or set the exact watchdog quit deadline `minutes * 60 + 15` seconds;
- reject optional UI caches, clock staging and media rendering/timing/silent-audio
  experiment environment variables. This run uses the current default pipeline.

The module starts after fixture rows are installed and the native window is
shown, retains one owner for its lifetime, and calls `finish()` after the event
loop. `Config::validate` independently rejects short/oversized durations and
relative paths without doing UI-thread filesystem I/O. The selected clip must
last at least 60 seconds; the existing 90-second generated moving 1080p60 clip
fits the schedule. Use the separate finite motion test to qualify that fixture.

## Finite schedule and ownership

One reusable single-shot Slint timer advances through a fixed 19-action,
60-second table. At most one timer and one scalar checkpoint are retained; no
per-frame polling, screenshot loop, pixel buffers, unbounded result queue,
persistent helper or network operation is introduced. Timer callbacks hold weak
window/state/driver references. A late action beyond five seconds fails the run
rather than silently skipping required transitions or shortening the duration.

Each cycle performs:

| Seconds | Action or observed assertion |
|---|---|
| 5 | Playing checkpoint: exact expected file-load count, native identity, video readiness, dimensions/decoder/error and model counters |
| 10 / 15 / 18 | Hide controls, verify progress timer stopped and dispatch finite stationary-area pointer input, restore controls |
| 20 / 23 / 26 | Pause; settle; compare stopped display clock, application redraw requests and bounded incidental UI drawing; submit paused seek |
| 29 / 31 | Confirm observed paused seek position; resume |
| 35 / 38 | Navigate to the local fixture feed; verify offscreen playback paused; return and resume |
| 42 | Resize to 1100×760 logical pixels |
| 45 / 48 / 50 | Minimize; verify actual native minimized flag and paused/stopped-clock policy; restore |
| 53 / 55 | Restore 1320×860; every other cycle enter and exit fullscreen |
| 57 | Identical normal-window playing checkpoint |
| 59 | Seek back before EOF, or reload the same local video/subtitle every fifth cycle |

No reload occurs at the last cycle's second59. Completion waits until the full
configured duration, requests a terminal stop, blanks the image, and uses one
three-second cleanup check before exiting. Errors take the same bounded cleanup
path. Early window closure or a quit watchdog before verified completion makes
`finish()` fail. No abbreviated test run can print a full-duration pass.

Catalog row-change/reset counters must remain unchanged from the initial
explicit fixture installation. Fullscreen, resizing, controls and navigation
must not recreate the player: only the scheduled explicit reloads increase the
observed `file_loads` count. Logs contain phase/cycle/time, scalar media/UI/model
counters and decoder metadata; no file paths, URLs, cookies or pixel data.
Render-target totals remain in the existing final presenter diagnostics; this
module does not invent per-phase target or GPU measurements.

## External resource and helper evidence

Run the release binary only after normal formatting/tests/lint and short native
lifecycle/motion tests pass. Record exact source/binary/fixture hashes, window
size/scale/refresh, active decoder, audio route and display/power conditions.
The display must remain awake for this functional automation; if a bounded
external display assertion is used, record it. Do not compare that condition to
an unasserted idle-energy baseline.

Once the CLI is integrated, the existing sampler can cover the complete hour:

```sh
python3 scripts/measure.py --warmup 0 --seconds 3600 \
  --include-new-vt-services --output artifacts/soak/local-60m.json -- \
  target/release/serein --local ABSOLUTE_MOVING_FIXTURE --demo-related \
  --data-root NEW_DIRECTORY --soak-minutes 60 --quit-after 3615
```

This invocation has not yet been run to completion. Mixed-use CPU averages do not satisfy settled idle or steady playback
budgets; measure those separately for at least 60 seconds after their own warmup.
Aggregate owned-process RSS and CPU retain the sampler's shared-memory,
short-lived-child and temporal VideoToolbox-service attribution limitations.
Keep foreign-host CPU audit separate from app totals.

Compare identical second57 checkpoints after warmup/cache saturation, preferably
grouped by five-minute reload phase. Investigate a sustained RSS increase above
10%; do not dismiss it as cache growth or select favorable samples. Retain
per-second raw samples, mean/p95/peak and final native target allocation counters.

The module explicitly reports helper-leak checking as **external, not measured**.
For this local-only run, independently record owned descendants at startup,
identical checkpoints and cleanup and confirm no extractor/JavaScript/token/DNS
helpers are present or left over. VideoToolbox services may be launchd-owned and
must be reported separately, with attribution uncertainty. The present RSS/CPU
sampler's process-count peak alone is not sufficient to certify helper cleanup.

No run, benchmark result, resource improvement or comprehensive SPEC soak gate
has been claimed by adding this diagnostic.
