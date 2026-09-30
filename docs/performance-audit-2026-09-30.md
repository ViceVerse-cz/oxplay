# Performance audit, 2026-09-30

This audit inspected media rendering/scheduling, UI publication and worker
queues, guest catalog/watch parsing, image fetch/decode/cache paths, cancellation,
and local-library paging. It makes eight focused corrections:

- Disabled or unavailable artwork writers are checked before PNG encoding.
  Enabled writers retain their protected atomic publication and disk budget.
- Comment portraits decode directly to their 88×88 bounds. Small images and
  already-normalized cached thumbnails retain their pixels instead of being
  resampled or enlarged. Fetch hosts, response/decode limits, concurrency and
  generation checks remain enforced.
- Guest InnerTube and caption requests await cancellation notifications instead
  of waking every 20 ms. Cancellation broadcasts to clones, registers before
  checking state, and survives cancellation before the first poll. Synchronous
  cancellation checks remain available. Account session checks and supervised
  process cleanup are outside this change.
- The offline library fixture switches to its feed only after an acknowledged
  video-page read. Previously, selection invalidation published an empty feed
  and navigated away before the real page could load. The strict sampler rejected
  that baseline with zero resource samples and zero decoded thumbnails.
- The paused-video fence regression uses three generated I420 frames in an AVI
  container accepted by the existing local-media policy. Its former Y4M fixture
  was rejected by the format whitelist before exercising the fence. The test
  still verifies restart identity, seek exclusion, audio-only replacement and
  failed replacement; no production format permissions are added.
- The virtualized feed uses explicit row positions and an independently placed
  pagination footer. A taller final row previously changed Slint's estimated
  row heights, leaving the last focused card outside the thumbnail viewport
  after rapid End/Left/Right navigation. Card trees remain limited to the
  near-viewport rows of the bounded catalog.
- Progress polling and clock publication use the seek-control and elapsed-text
  bounds rather than the entire transport background. At the compact watch
  scroll limit, an 11-pixel strip of background can remain visible while both
  clock consumers are offscreen. That strip previously kept 250 ms progress
  requests active. A compiled-widget regression verifies offscreen suppression
  and resumption when the player is revealed.
- The local-media lifecycle fixture uses a 1000×600 window for its compact
  related-list test. At 760×600, the current watch layout actually exposes two
  related rows, contradicting the fixture's offscreen assertion. The wider
  compact layout keeps those rows fully offscreen before scrolling; the zero
  work, scroll, fullscreen, focus and lifecycle checks remain enforced.

Local-store paging already uses keyset cursors and the playlist/video indexes.
A supplementary Python SQLite check with 100,000 synthetic history rows found
indexed searches at first, middle and deep cursors; an alternative predicate had
no material advantage and was not adopted. This is supplementary inspection,
not a benchmark of the app's bundled SQLite build or every library workload.

## Measurements

Resource and microbenchmark results are recorded in the accompanying
[audit summary](evidence/2026-09-30-performance-audit.json). Raw native/sampler
logs, fixture data and executable copies remain in ignored
`artifacts/performance-audit/`; the summary records their hashes and selected
scalar evidence without exporting private profile paths.

| Workload | Mean CPU (% of one core) | Mean RSS (MiB) |
| --- | ---: | ---: |
| Empty Home | 0.17 | 123.7 |
| Local 1080p60 playback | 45.26 | 194.6 |
| Paused local playback | 0.06 | 181.5 |
| Offline library (final grid) | 0.00* | 142.4 |
| Minimized offline library (final grid) | 0.06 | 142.8 |

Idle and paused runs were collected at `5cd61e8`; library runs use `225eef0`,
including the grid-geometry correction. Playback uses the final code at
`1b4333e`, including the clipping and local-smoke setup corrections. The final
full Rust suite and native functional check cover that source as well.

*Zero means no CPU-time increment was observed at the sampler's resolution
during this finite window. It does not establish zero resource consumption.*

Playback used hardware decoding, with zero decoder drops and three
video-output drops across the complete 80-second session; there is no separate
warm-window drop counter. The library published 12 near-viewport thumbnails,
with nine geometrically intersecting the viewport, from a real 100-row page of
a 10,000-row SQLite fixture. Compositor visibility and GPU completion were not
measured. Baseline playback averaged 46.92% CPU and 196.9 MiB RSS; the earlier
corrected-source run at `5cd61e8` averaged 46.16% CPU and 194.4 MiB RSS. These
small whole-app differences do not isolate the fixes from run variability.

Release microbenchmarks use deterministic synthetic PNG inputs and the median
of 21 batches. They compare the former encoding/resizing operations with the
corrected code, in one process. They do not include network latency, GPU upload,
real-page throughput or system energy. The 48-pixel portrait case fell from
472.16 to 39.61 µs/image (92% less time);
the 180- and 1024-pixel cases improved by 27% and 17%. The disabled-cache path
avoided a 336.70 µs/image PNG encode. These are local operation costs, not app-wide
speedups. The microbenchmarks ran at `6440df5`; the measured image code is
unchanged in the final source. Cancellation race/broadcast/drop tests
use explicit future polling and waker counts, without a timing-dependent pass.

Resource runs use the checked-in sampler, one-second samples, a five-second
idle/library warm-up and ten-second playback warm-up. The 1080p60 local H.264/AAC
fixture plays through VideoToolbox/AVFoundation. A bounded external display-wake
and display-awake assertion support this development-host test. New decoder
services use temporal attribution; aggregate RSS may count shared pages twice.
No screenshots, GPU queries or stack sampling run during resource measurement.

The baseline source is `f831a22`. The final source also includes subsequent
main-branch UI commits `3cee068`, `5ac3298` and `4901842`. Consequently the
whole-app runs are observations/regression checks, not an isolated speedup
comparison of these fixes. The old library fixture never reached readiness, so it has no valid
baseline resource measurement. Display-asleep failures and a preparation-overlap
attempt are excluded. The summary identifies successful and excluded attempts.
The first corrected library lifecycle reached thumbnail readiness but then
became occluded and failed its strict visibility checkpoint; that failure is
retained separately.

No 25%-CPU playback target, 30-visible-thumbnail gate, long-soak leak freedom,
energy improvement, account qualification or other-platform qualification is
established. Experimental presenters and rendering cache flags remain default
off. Existing source-specific results and SPEC budgets keep their scope.

Validation passed: 599 Rust tests (six opt-in tests remain ignored), 163 Python
script tests, formatting, strict Clippy across all targets and a locked release
workspace build. The release image microbenchmarks separately passed their two
opt-in cases. Native fixtures exercise real SQLite paging and raster publication,
keyboard focus/resize/cancellation, and local media/captions/lifecycle behavior.
These checks do not qualify remote provider/account behavior.

## Reproduce

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --release --workspace
python3 -m unittest discover -s scripts -p 'test_*.py'
cargo test --locked --release -p serein thumbnails::performance:: -- --ignored --nocapture --test-threads=1

scripts/generate-fixture.sh
# Choose a NEW absolute private directory for the offline library fixture:
python3 scripts/prepare_library_resource_fixture.py /absolute/new/fixture "$PWD/artifacts/local-1080p60.mp4"
python3 scripts/measure.py --warmup 5 --seconds 30 --library-fixture-ready --output artifacts/library-audit.json -- ./target/release/serein --library-resource-fixture /absolute/new/fixture --quit-after 45
./target/release/serein --library-resource-fixture /absolute/new/fixture --library-resource-smoke-test
./target/release/serein --data-root /absolute/new/local-profile --local "$PWD/artifacts/local-1080p60.mp4" --subtitle "$PWD/artifacts/local.srt" --smoke-test
```

Native commands require an awake graphical macOS session. Close each fixture
process before starting the next run, and finish compilation before resource
sampling. Use separate new profiles for ordinary idle and local playback checks.
