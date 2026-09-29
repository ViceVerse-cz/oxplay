# Finite engine synchronization diagnostics proposal

Current explicit audio probes confirm fresh decoder/output sample rates and an
advancing output clock after a stream replacement. They do not establish
audible/visible synchronization. mpv's
[`playing_audio_pts`](https://github.com/mpv-player/mpv/blob/v0.41.0/player/audio.c)
subtracts the audio driver's estimated queued delay, scaled by audio speed,
from written audio PTS. It does not measure the physical sound output.

The proposed next diagnostic extends only `request_audio_probe` with optional
`avsync` and `total-avsync-change` DOUBLE results: five explicit GETs, one
outstanding token, no observed property subscriptions or ordinary playback
timer. Correlate results with load, native entry and transport epoch; record the
request/completion interval because multiple async properties are not an atomic
sample. Preserve bounded partial-submission cancellation ownership. Token stride
must become eight and completion mask 31; do not overlap other command namespaces.

Use two or three finite checkpoints after observed restart/resume in an explicit
local/refresh smoke. Require current, nonbuffering Playing state and no pending
transport operation. Log observed speed, fresh audio progression and video
render progression. Report unavailable values explicitly; do not interpret
paused/startup zero as synchronized output.

In mpv 0.41's
[`update_av_diff`](https://github.com/mpv-player/mpv/blob/v0.41.0/player/video.c),
the difference is audio PTS minus video PTS plus configured audio delay and a
scheduling offset. It can be reset to zero when streams are not playing.
Correction accumulation is signed, resets with video state, and is cleared in
display-sync handling; compare it only within one uninterrupted transport epoch.
The native warning threshold of 0.5 seconds is a gross-desynchronization diagnostic,
not a product acceptance tolerance. SPEC supplies no numerical sync threshold.
The [property contract](https://github.com/mpv-player/mpv/blob/v0.41.0/DOCS/man/input.rst)
also makes these properties unavailable without both audio and video.

This remains a proposal. Actual perceptual qualification requires an appropriate
licensed/local audiovisual sync fixture and separately recorded display/audio
observation. No new synchronization probes or ongoing polling were implemented.

The source audit did identify a separate transport bug: a relative seek could
admit a fresh probe before its native SEEK event. The implementation now tracks
one exact seek command and requires command success plus SEEK then RESTART.
Load, stop, or active END_FILE cancels its ownership;
stale replies cannot revive it. A thirty-second single-shot watchdog uses the
reserved stop path and reports a recoverable reload-needed error if native
confirmation never arrives. Native command success alone is insufficient:
mpv can accept a queued seek and subsequently fail its demux seek without a SEEK
event. Relative/absolute overlap and repeated relative inputs return a busy error. Repeated absolute inputs coalesce into one latest requested position, submitted only after the current seek has completed native confirmation. The same bounded watchdog covers either seek type, allowing a remote seek time to buffer without introducing a poll.
These changes and focused state/native tests are authored, not yet executed
during the exclusive soak hold.
