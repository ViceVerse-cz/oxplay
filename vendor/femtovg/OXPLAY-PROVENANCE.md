# Vendored FemtoVG

This directory was extracted from the unmodified crates.io release archive
`femtovg-0.27.0.crate` on 2026-10-01. Its SHA-256 is
`31eebf3ab1b76359ccd5f850c07a7c6b2722c64d9127c40e02358e83f3ff9b1a`,
matching the workspace Cargo.lock registry checksum. The upstream project is
https://github.com/femtovg/femtovg; release archive URL:
https://static.crates.io/crates/femtovg/femtovg-0.27.0.crate.

Original release Cargo.toml, Cargo.toml.orig, source, tests, and MIT/Apache-2.0
licenses are preserved. Local changes are limited to
`src/renderer/wgpu.rs` and this provenance file:

- WGPU pipelines survive one unused nonempty flush, then expire on the
  second unused flush. This avoids recompiling distinct clear/scene pipelines
  when Slint flushes its clear separately before a rendering notifier.
- The map is capped at 256 entries after encoding, evicting older idle keys
  first. A two-age sweep allocates/sorts nothing and leaves draw order,
  pipeline keys, shader code, and synchronization unchanged.
- Tests exercise actual clear/image PipelineState keys, alternating warmup,
  idle expiry/reuse, age-prioritized overflow, and continuous key churn without
  requiring a GPU.
- CPU profiling found repeated viewport-buffer/group creation. An eight-entry
  exact-float-bit viewport cache now reuses immutable 16-byte uniform buffers
  and their bind groups across clear/scene flushes. The cache belongs to one
  renderer with one fixed binding layout; it contains no image/video/glyph
  texture references. It uses the same two-unused-flush expiry and bounded
  older-first policy. Additional GPU-free tests cover clear/layer/screen
  alternation, fractional dimensions and display scale, immutable retained
  handles through eviction, idle expiry, and dimension churn. CPU evidence
  and source aggregation are under artifacts/performance-2026-10-01.

The caches can transiently hold all pipeline states and viewport dimensions
used while encoding one flush; pruning after encoding bounds retained
residency. The viewport cap retains at most eight groups with eight 16-byte
uniform buffers (128 bytes of uniform payload, plus GPU/driver object overhead).
Recorded WGPU commands retain their resources independently of these maps. The
fixed cap is intentionally conservative: image identities are absent from
PipelineState, so changing video textures does not multiply keys. Pathological
scenes with over 256 active states remain correct but may recreate evicted
states on the next flush.

The patch is prepared for matched release profiling. Its runtime performance
benefit and Linux/Windows hardware behavior must not be inferred from the
GPU-free unit tests.
