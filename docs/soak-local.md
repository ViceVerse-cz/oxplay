# Completed local mixed-use hour — 2026-09-29

The local functional diagnostic completed **60 cycles over a full hour**, with
120 playing checkpoints, 12 explicit file loads, and a clean exit. The full SPEC
soak gate remains **open**: aggregate RSS continued growing, and this workload
does not cover remote search/extractor cancellation, account operations,
decoder fallback, or the large raster library case.

## Frozen input and scope

- Source: `ee59eaa47289382b6784547e8bece1b38e5e60a5`.
- Executable SHA-256:
  `464fdcf1432f1425698cf2892f2fd7f3e942c4e132dfb176c5c45bcb5582b0a8`.
- Exact local fixture SHA-256:
  `d5bd6130435aad2f07b00ff102c56d04f649c70e479ec762fe58f659b9baaba0`.
- Sampler SHA-256:
  `00a4e38039fb6523988a044b5de20a8853f3e131d1c5d662a5aed7d0941d03de`.
- External harness SHA-256:
  `65a6dc2fd8129a450e129ac2d0cc56fbd471a7b25b41f4bec50dd7585834ebc3`.

The run used the Apple M1/macOS host described in [performance.md](performance.md),
the generated H.264 1920×1080/60 AAC clip, thirty explicitly labeled static demo
rows, a fresh isolated local profile, and the default media/UI pipeline. Optional
caches, staged progress, stable-target and GPU timing experiments were off. A
bounded external assertion kept the display awake through intentional paused
intervals; it did not prevent the test's explicit minimize/restore actions. No
account credentials, live provider content, screenshots or per-frame CPU copies
were part of this workload.

Before freezing the artifact, the implementation session independently compared
all 119 captured build-input hashes with the named Git revision and checked the
executable identity. The published [source inventory](evidence/2026-09-29-soak-local-source.json)
retains those hashes and the original association note. This does not assert a
new persisted verification report or an independently rebuilt native dependency
closure. The binary predates subsequent mute, seek-confirmation, keyboard and
offline-library changes.

## Functional observations

The [finite schedule](soak-test.md) exercised controls hide/show, stationary
pointer input, pause and paused seek, resume, feed/watch navigation, two window
sizes, native minimize/restore, alternate-cycle fullscreen, and reload every
fifth cycle. All 120 checkpoints observed H.264 1920×1080/60 with
`hwdec-current=videotoolbox`. Unrelated catalog changes stayed zero; the one model
reset installed the initial demo fixture. The final display-clock counters were
300 starts and 300 stops, with no active clock or media display-sleep assertion.

The driver waited until its terminal stop was confirmed, the display clock was
inactive and progress timing had stopped, then printed `local-functional-pass`
at 3,603.040 seconds. The external harness completed in 3,604.715 seconds, exit
code zero, without forced termination. The later generic shutdown code submits
a **second** stop after leaving the event loop, so the final session dump's
`stop_pending: true` describes that new request. It does not negate the driver's
earlier confirmed cleanup; nor does that final dump independently prove the
second request's completion.

The external 15-second descendant/birth-time audit and final known process-group
check reported no observed survivors. This is bounded sampled evidence, not a
guarantee that every very short-lived or reparented process was observed. No
extractor, JavaScript runtime, authenticated helper, or network media request was
needed for this local workload. The resource sampler attributed exactly two
processes in each of its 3,600 samples, including newly observed VideoToolbox
services through temporal attribution; exclusive service ownership was not
proved.

## Resource observations and open gate

Sampling began at `2026-09-29T07:26:42Z`, with zero warm-up and 3,600 one-second
samples across the mixed-use schedule. These are not steady playback or idle
qualification samples:

| Measure | Mean | p95 | Peak |
|---|---:|---:|---:|
| Aggregate sampled RSS | 232.690 MiB | 254.625 MiB | 257.313 MiB |
| CPU, one logical core = 100% | 34.539% | 63.432% | 68.996% |

All minute RSS medians increased. Twenty approximately matched minute pairs
(11–30 compared with 41–60) had median growth **9.456%**, ranging from
**9.126% to 11.665%**; four pairs exceeded 10%. The data therefore does not pass
the leak-free requirement. Minute 30→60 median RSS grew from 234.609 to
256.047 MiB. Neither a memory leak nor harmless caching has been causally
established. Shared-page double counting, temporal decoder attribution and the
lack of per-process/private-footprint observations limit the diagnosis. See
[the attribution review and next measurement](soak-attribution.md).

Native decoder-drop counters remained zero. VO-drop counters are reset by
seeks/loads and were sampled at transition checkpoints; their maximum of 110 is
not a cumulative loss count or a steady-frame-pacing result. Final renderer
counters recorded 302 target allocations and 7,936,128 tracked target bytes;
those do not inventory all driver, decoder, Slint or compositor allocations.
Perceptual audio/video sync, energy, other codecs and other OS targets remain
unqualified.

## Retained evidence and privacy audit

The [export manifest](evidence/2026-09-29-soak-local-export.json) records full
SHA-256 values and byte sizes for every published file, plus the original
artifact identities. Its own SHA-256 is
`049653fb31a00505c5d42de9a9e94eab426a25356a47f6256f35ba5c743413a5`.

- [Complete resource samples](evidence/2026-09-29-soak-local-resource.json.gz):
  deterministic gzip, level 9, timestamp zero; compressed SHA-256
  `c0c6160a5e3faf6ffcd2e3bddec6c9725e7ca8e77955ed3315a334ac4a5c5713`.
  Decompression reproduces all original 3,838,824 bytes, SHA-256
  `8a0643180b8fca1c14b33fcb3b883107270cb801cb55e757942be38658ecb16c`.
- [External summary/helper audit](evidence/2026-09-29-soak-local-summary.json).
- [Frozen source hashes](evidence/2026-09-29-soak-local-source.json).
- [Full scalar native log](evidence/2026-09-29-soak-local-native.log).
- [Analysis and all matched-minute comparisons](evidence/2026-09-29-soak-local-analysis.json).

JSON was parsed, all 3,600 samples retained, gzip round-trip and deterministic
compression checked, and selected text scanned for local absolute-path prefixes,
URIs and credential-header markers. None were present. All 196 distinct
host-audit names were validated as bounded printable executable basenames, with
no paths or argv. No redaction was needed in the selected files. The private
profile locator, executable and redundant raw helper/sampler files remain only
in ignored artifacts. Original artifacts were preserved unchanged.
