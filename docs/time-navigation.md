# Explicit time navigation

The shared Watch transport offers Jump to time for an active finite-duration
video, including a local file. Enter seconds, minutes:seconds, or
hours:minutes:seconds; the final seconds field accepts a decimal fraction.
Malformed fields, negative values, exponent notation, times beyond the current
video and values above the bounded 32-bit seconds range are rejected. Seeking
preserves the player's current pause intent.

The dialog pins the native load identifier and optional account generation when
opened. Submission checks both again, requires a current stable playback clock,
and checks the live account lease. Media events close a stale dialog after
replacement or revocation. There is no independent progress timer, model reset
or follow-up seek attached to an unrelated file-loaded event. A successful native
command admission closes the dialog; native seeking remains event-driven.

The focused clock parser regression is authored and compiled, not executed in
this feature pass. Native keyboard, popup focus and seek behavior require later
functional qualification. Timestamped YouTube links use their separate typed
initial-load intent described in [timestamp links](timestamp-links.md).
