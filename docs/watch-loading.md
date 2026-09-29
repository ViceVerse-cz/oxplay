# Opening a video

Selecting a guest result or a video URL now opens the watch page in the same UI callback, before stream resolution completes. Existing result metadata supplies the title and channel; unknown metadata uses static skeletons while the request is pending. Account playback uses the selected account item's title and the protected account resolver. No private account identifier is passed to a guest metadata or artwork request.

The application retires the previous picture, captions, description, chapters and share target before showing the new identity. The existing player receives an asynchronous stop, not a replacement player/context. Guest and initial account handoffs wait for the exact media stop barrier, with a finite three-second deadline. Failed stop dispatches, native replies and output-verification failures can be retried by another explicit selection; they are not polled or continuously resubmitted.

Guest requests still use the supervised latest-request slot. Another selection can replace an in-progress video request. Navigation cancels the generation, and stale results cannot navigate back to the watch page. Cancellation publication uses one replaceable UI-thread timer because generation observers run while the worker handle is borrowed; a new selection cancels that deferred update. There is no loading animation or per-frame message.

Initial resolver and presentation failures terminate pending skeletons and show an inline error. Guest retries retain the existing typed failure/backoff policy. Account retries retain the selected session generation and recheck explicit account playback admission. Disconnect discards the account retry target. Existing terminal native-file retries preserve their accepted load identity and pause intent.

This is a UI response and ownership change, not a claim that network extraction finishes immediately or a measured latency improvement. Runtime visual checks and account qualification remain separate from source/build checks; no performance benchmark was run for this change.

The watch ScrollView keeps its content geometry and wheel/trackpad/focus-driven
scrolling but uses `vertical-scrollbar-policy: always-off`; the native style's
bar was drawn inside the content area over the video. Loading placeholders do
not shimmer or run a timer. The player displays a starting state while the
engine awaits a frame; EOF and audio-only playing states do not remain loading.

Validation: formatting, strict all-target workspace Clippy, and the locked debug
workspace build passed. Native runtime, account and latency checks were not run.
The new wake-coalescing regression compiles but was not executed. The next
functional check is rapid guest selection, navigate-away cancellation, inline
failure/retry, and signed-out account pending-title removal on a native display.
