# Search-cache allocation profile

Two short native profiles on 2026-09-29 support the specific gradient-allocation
mechanism behind the [search-field cache experiment](search-cache.md). With the
cache off, a sampled border-rendering stack reaches `GradientStore::lookup_or_add`
and `glTexImage2D`. With `--search-cache`, neither function appears anywhere in
the sampled call graph. This is qualitative evidence of the intended bypass,
not proof of zero allocations and not a resource-efficiency result.

Both runs used the same frozen release executable, SHA-256
`ac87f2c3ad53ff066fcb5596ec9ffc614c5f0e81c737700ed25f1a7f8628ea80`, from the
`artifacts/search-cache-e1725df/bin` snapshot. Each ran the same existing generated
local H264/AAC fixture for 25 seconds, at 1320×860 logical window size with 30
explicitly labeled related-video fixture rows. Whole-UI caching was off in both;
only the second run enabled `--search-cache`. A private local data directory was
used per run. No screenshot or GPU-timing mode was enabled. A bounded
`caffeinate -u -t5` wake preceded each launch.

At approximately twelve seconds after process creation, `/usr/bin/sample`
collected five seconds of stacks from that exact owned process with a requested
one-millisecond interval. Both sample commands and applications exited zero.
This instrumentation perturbs scheduling; the runs are expressly excluded from
resource or dropped-frame acceptance measurements.

| Sampled observation | Cache off | Search cache on |
|---|---:|---:|
| Main-thread inclusive sample weight | 2,415 | 2,398 |
| `GradientStore::lookup_or_add` node weight | 6 | Not observed |
| Nested `glTexImage2D` node weight | 5 | Not observed |

The uncached path was:

```text
GLItemRenderer::draw_border_rectangle
  -> Canvas::stroke_path_internal
  -> GradientStore::lookup_or_add
  -> ImageStore::alloc
  -> GlTexture::new
  -> glTexImage2D
```

These are nested, inclusive sampled-stack weights. They must not be added
together or read as allocation counts, calls per second, CPU percentages, or
GPU duration. The cached profile's absence of these stacks is consistent with
the source-reviewed cached-layer path skipping the border's drawing work during
steady playback. Rare invalidations can still recreate or redraw resources
outside this short observation. No renderer or frame-copy path changed.

Both native logs observed VideoToolbox decoding of H264 1920×1080 at 60 fps,
AAC 48 kHz with AVFoundation output, Playing state, and no engine error. Decoder
drops stayed zero. Catalog row changes stayed zero; the single reset was the
initial fixture installation. The uncached run's VO drops increased from two at
10 seconds to four at 25 seconds; the cached run remained at two. This difference
is not a pacing result because the profiler was active between those checkpoints.

The raw profiles and native logs remain ignored under
`artifacts/search-cache-profile/{off,on}`. The
[sanitized evidence summary](evidence/2026-09-29-search-cache-profile.json)
contains hashes, sample weights, the allocation path and selected nonsensitive
engine counters. It excludes process IDs, loaded-image maps, arbitrary memory,
local directory names and unrelated process data.

The separate same-binary ABBA resource experiment determines whether caching
improves measured CPU/RAM. Removing an observed allocation path can be useful
mechanistically without producing a measurable CPU benefit; this profile does
not override that experiment or justify enabling the cache by default.
