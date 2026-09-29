# Native command and replacement identity

`Player` submits bounded asynchronous commands and consumes their replies on
its UI-thread owner. Load commands use a monotonic bit-62 token namespace;
dedicated resume-position queries use bit 63. Caption commands stay below bit
62. Tokens never wrap into another namespace.

The actual mpv 0.41 `cmd_loadfile` implementation returns a map containing
`playlist_entry_id`. The media adapter correlates that result with the public
START_FILE and END_FILE event entry IDs. It does not infer the active request
from the most recently submitted command or from a global event counter. The
bounded eight-entry map may evict an old, non-active request; an unrecognized
entry reports identity zero rather than guessing. A START arriving before its
command reply remains unidentified until the matching reply arrives.

`Snapshot.load_request_id` identifies the latest accepted submission;
`active_load_request_id` identifies the native entry actually observed, and
`failed_load_request_id` identifies an asynchronous command failure only if it
belongs to that latest submission. Stop clears the requested ID. Stale load
command errors and END_FILE events for a different native entry cannot fail
the current request. File-start/load counters remain useful observations but
are not request identity.

The source contracts are mpv 0.41
[command.c cmd_loadfile](https://github.com/mpv-player/mpv/blob/v0.41.0/player/command.c)
and [client.h event structures](https://github.com/mpv-player/mpv/blob/v0.41.0/include/mpv/client.h).
Native tests submit a missing file followed immediately by a real silent WAV,
verify the final entry matches its exact request, and inject a real failing
asynchronous command tagged with old/current request IDs to separately exercise
COMMAND_REPLY error correlation. The injected command is a test of reply
handling, not a claim that loadfile itself produced that particular error.

Resume-position queries permit one pending request. The result is a token plus
an optional finite nonnegative position; unavailable or failed queries do not
substitute zero. Cancellation is token-scoped. Load/stop clear the request;
accepted seek/pause operations invalidate both pending and already delivered
samples. Late native replies are ignored. Volume changes do not invalidate a
position. A query is refused while the requested and active load IDs differ.
Pending/coalesced seeks and observed Seeking/Buffering states also refuse a
query, so a replacement cannot overtake an already accepted seek.
Native tests verify the paused file's actual position, bounded admission,
cancellation followed by another request, and seek/pause/stop invalidation.

File-scoped `loadfile` options carry initial companion audio and subtitles,
avoiding deferred FILE_LOADED actions attaching tracks to a superseding file.
Subsequent caption files have a separate pending-command lease plus the host's
full-playback lease. Selection is confirmed against the engine's observed
external filename, restricted to explicitly registered local paths; no path or
caption body enters diagnostic snapshots.
