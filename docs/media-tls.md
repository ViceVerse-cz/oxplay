# Media TLS correction

The direct libmpv path previously omitted `tls-verify`. The exact
[mpv 0.41.0 source](https://github.com/mpv-player/mpv/blob/v0.41.0/stream/stream_lavf.c)
defaults that option to false and explicitly passes its value to libavformat.
Earlier successful HTTPS playback and native-library closure checks therefore
did **not** establish authenticated TLS. This was a security defect, not a
packaging or performance qualification detail.

The player now always sets `tls-verify=yes`. A native engine test reads back
`options/tls-verify` and requires it to be enabled. `Player::new_with_ca_file`
accepts an explicit absolute CA path. The app validates that resource before
opening its UI; missing/empty/nonregular/oversized or out-of-bundle CA resources
fail without silently changing the trust source.

macOS packages always include `Contents/Resources/Certificates/mozilla.pem`,
copied from the immutable reviewed ca-certificates installation and checked
against its source-formula hash. It is independent of helper inclusion. The
development macOS path uses that same immutable CA source, not the host's
Keychain-merged generated bundle. Other target trust paths remain unqualified.
The separate Rust HTTP range experiment uses verified Rustls TLS defaults.

The option regression and explicit native HTTPS fixtures passed. Running
`python3 scripts/test_media_tls.py` starts two bounded loopback-only HTTPS servers,
generates a disposable private fixture CA and leaf certificates, and runs one
ignored Rust test through the production Player constructor. The untrusted chain
was rejected; providing that fixture CA accepted the silent WAV; a trusted-chain
hostname mismatch was rejected before any HTTP request. Outputs were explicitly
null (no GPU window or audible audio), and the temporary keys/servers were cleaned
up. The script uses offline Cargo and rejects non-loopback fixture URL inputs.
Sanitized evidence is retained in `docs/evidence/2026-09-29-media-tls.json`.

Corrected direct public playback also reached VideoToolbox H.264 1080p with Opus
at the5/10-second checkpoints; the longer quality exercise was interrupted by
native occlusion and does not pass that lifecycle gate. See playback-quality.md.
Clean-machine bundle tests still require the corrected binary. The older
helper-only packaging artifact is not evidence for corrected media TLS, and
these certificate checks alone do not close redirect/DNS/whole-process egress
or every target platform's transport gate.

The later source audit hardened the harness's Cargo process-group cleanup and
split rejected-chain HTTP accounting from the trusted listener. Those edits and
their new offline regressions are pending execution; the historical result above
must not be read as their validation. Exact prerequisites, effects, cleanup limits
and commands are in [external-integration-checks.md](external-integration-checks.md).
