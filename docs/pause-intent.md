# Pause intent, occlusion and end of file

The first restricted native-child lifecycle failed at23seconds on d7c58acd.
A user pause command was still asynchronous when leaving fullscreen generated
occlusion. The app saved the older observed `paused=false`, then resumed on
exposure, undoing the user request. The complete failed native run is retained
in [first-attempt evidence](evidence/2026-09-29-native-child-first-export.json).

The media owner now distinguishes accepted user pause intent, the hidden-window
policy overlay, and the observed native `Snapshot.paused`. Effective native pause
is user pause OR hidden. Repeated occlusion events are idempotent. Explicit pause
writes use one correlated in-flight slot within the existing64-command bound;
newer desired values coalesce without a timer or an additional native queue.
Current command failure reports an error and takes the reserved stop path after
releasing the event-drain snapshot borrow. Stale replies cannot confirm a newer
load or cancel newer intent. Stop prevents later exposure from reviving playback.

Stream quality/expiry replacement preserves accepted user intent, independently
of hidden policy. Its fresh-position request rejects an unsettled pause; native
observations remain actual observations rather than optimistic UI state.

Automatic guest stream refresh now checks accepted pause intent before starting
extraction, in addition to the native pause observation. Admission requires the
remembered, accepted and active native load to match, ready presentation and the
loaded Watch page, with no stop or failure for that load. Browsing already pauses
playback; this closes its asynchronous acknowledgement gap. A rejected refresh
keeps its due flag for a later real resume/event rather than starting a retry
timer. A deterministic admission regression covers pause, navigation, stale load,
stop, failure and resumed playback; it is not a live expired-stream qualification.

mpv keep-open pauses natively at EOF, and `pause=no` alone does not replay.
A finite correlated `eof-reached` query reconciles the natural hold after pending
writes and exact file admission. It owns one query slot through cancellation;
load generation and intent epoch reject stale replies. There is no recurring EOF
poll. Play at settled EOF seeks to zero and defers unpause until the existing
seek confirmation completes. A rapid second toggle preserves its latest pause.
Every new file invalidates the applied-pause cache so an earlier EOF cannot
silently leave an intended unpaused load paused.

The final focused media suite passed60tests with one explicit TLS integration
ignored. The full workspace passed324tests with four external checks ignored;
formatting and strict all-target Clippy passed. Headless native coverage includes
queued pause/hide/show before event observation, queue saturation, real command
failure, delayed first drain beyond a short WAV's EOF, EOF hide/show, rapid double
toggle, replay, a second EOF, new unpaused load and clean stop. The intermediate
EOF attempt had56passes/two failures; reasons/excerpts are retained privately in
`artifacts/native-child-v1/pause-eof-intermediate-failures.md`, not presented as a
complete raw-log or exact-source association. Final focused output is retained
alongside it.

The locked release at `a45ff67692e0f25e22c188dc1583c3490d94a542`
(application SHA-256 `0734e6d46cef1c22eb527a67e0e135eedc69c63f3ca3458dd54b513ceb93afd9`)
passed the corrected native-child lifecycle in 47.425 seconds and a second
47.452-second run with finite external window captures. Both completed all
14 checkpoints and exited normally with the owned process group absent. At the
previously failing stage23, the engine remained paused after fullscreen exit;
Info/caption/Settings hiding, fresh paused reveals, resize, minimize/restore and
confirmed stopped/hidden state also passed. The default borrowed-texture local
smoke passed separately in 22.456 seconds, including pause/seek, mute, keyboard
and occlusion transitions. No unrelated catalog changes occurred during these
local playback runs (one initial fixture reset).

The [corrected evidence manifest](evidence/2026-09-29-native-child-corrected-export.json)
retains complete logs, exact source/binary provenance and capture hashes. Its
SHA-256 is `5ac8763e8e3eaae12130ba32eb241c21f406e33ece54e0d74f0016c94738f8d6`.
This qualifies the exercised pause-intent regression, not every event ordering,
perceptual A/V synchronization, production native-child use or resource budgets.
The lifecycle's stage44 confirms the stop before generic event-loop shutdown
requests another stop; its final `stop_pending=true` is from that later request.
