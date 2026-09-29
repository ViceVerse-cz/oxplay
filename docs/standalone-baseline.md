# Matched standalone libmpv baseline

On 2026-09-29 the diagnostic-only SDL/libmpv harness completed a ten-second
functional check followed by an 85-second native run. Its new mode 6 preserves
CVDisplayLink phase during observed Playing, matching the current application
clock rather than the historical mode-5 two-empty-tick stop policy. It is not a
second application UI, is not packaged, and does not establish application release
readiness.

The measured run used a ten-second warm-up followed by sixty one-second samples.
100% CPU means one logical CPU; aggregate RSS includes the owned process tree
and newly launched VideoToolbox services under the sampler's temporal attribution
rule. The process-count peak was two. There were two preexisting VideoToolbox
services, which were excluded from that ownership rule.

| Metric | Mean | p95 | Peak |
|---|---:|---:|---:|
| Owned/temporally attributed CPU, one logical core | 39.418% | 49.000% | 50.761% |
| Aggregate RSS | 163.459 MiB | 164.109 MiB | 164.219 MiB |
| Separate host-other CPU, one logical core | 56.399% | 78.961% | 160.083% |

Native checkpoints at 10.028 and 70.012 seconds reported **two VO drops at both
boundaries and zero decoder drops**, with 3,599 additional video draws. Thus no
additional drops were observed over that approximately sixty-second warm
interval. The final 85.036-second checkpoint remained at two drops, with 5,089
video draws. The clock started once, remained active through playback without
restarting, and stopped once on clean exit. The scoped idle-display-sleep
assertion was active at every playback checkpoint. Exit status was zero with no
forced termination.

Observed configuration:

- Apple M1; OpenGL vendor Apple, renderer Apple M1, version 4.1 Metal 91.7.
- Actual drawable 1384×778; display mode 1440×900 at 60 Hz; requested window
  692×389 logical pixels. Both native and SDL swap waits were disabled.
- VideoToolbox decoder, H264 1920×1080 at 60 fps; AAC 48 kHz and AVFoundation
  audio output. These are observed engine properties, not requested-option proof.
- `BLOCK_FOR_TARGET_TIME=0`, `video-timing-offset=0.05`, advanced control disabled,
  mode 6. SDL headers report 2.32.72. Native dependency provenance is in the
  [harness README](../tools/media-baseline/README.md).
- The same existing generated local clip used in the application/browser runs;
  no remote media, subtitles, extractor or account state. A bounded
  `caffeinate -u -t5` wake preceded each run and exited before the measured interval.
  Perceptual audio/video synchronization and sound output were not independently
  measured by this sampler.

The team held all other GUI launches, builds and downloads during this slot.
That did not make the whole host idle: the audit records WindowServer at 17.586%,
WallpaperAerialsExtension at 10.846%, coreaudiod at 6.585%, and preexisting
VTDecoderXPCService activity at 4.771% mean recorded CPU. All four appeared in the
top-eight audit for every sample. Other host work included a `zeron` spike of
108.972%; its recorded mean lower bound was 6.573%. Per-name means derived from
the bounded top-eight list are lower bounds when a name was omitted in some
samples. These services are not stopped, assigned to the application, or assumed
to be caused by this player.

This is one standalone observation, not a controlled subtraction that isolates
Slint GPU or CPU stage cost. The current application and this baseline have
different window composition, control layouts and render targets; the baseline
renders directly to the default framebuffer. Independent-run differences and
shared-service costs limit a strict overhead claim. Shared pages can be counted
twice in RSS; footprint, GPU/unified-memory allocations, energy, and physical
scanout are unmeasured. Short-lived/reparented helpers can escape sampling, and
new VideoToolbox process attribution is temporal rather than proven exclusive
ownership. The native checkpoint boundaries are SDL events, not perfectly aligned
with the sampler's process snapshots.

Reproduce only in a coordinated quiet window, after compiling with the exact
command in the [harness README](../tools/media-baseline/README.md):

```sh
python3 scripts/measure.py --warmup 10 --seconds 60 --include-new-vt-services \
  --output artifacts/standalone-mode6.json -- \
  /tmp/serein-media-baseline "$PWD/artifacts/local-1080p60.mp4" 85 0 0.05 0 6 692 389
```

Source HEAD at measurement was `e1725dfe8c844ac32258beda5cc4d9a3fd3df8c4`, with the
mode-6 harness source uncommitted; the exact source and binary hashes identify
the measured version:

- Binary: `f19d1d40032a13a7f1c842aafaffd4bf3db66f9063999dccf4c4451efc4fa528`.
- C source: `cf7a2949ba531af191a4c3c8042944b3e03a2f91f2d645bd8f7c12c1e03bfdc4`.
- Clip: `d5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0`.

The sanitized evidence contains no media URL, account data, unrelated process
arguments or directory paths: [resource samples](evidence/2026-09-29-standalone-mode6.json),
[native checkpoints](evidence/2026-09-29-standalone-mode6.log),
[functional check](evidence/2026-09-29-standalone-mode6-functional.log), and
[provenance/host audit](evidence/2026-09-29-standalone-mode6-provenance.json).
