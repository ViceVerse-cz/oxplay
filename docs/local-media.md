# Opening local media

The shared sidebar offers **Open video file**. `Ctrl+O` / `Cmd+O` opens the same
native picker when a text editor or IME does not own the key. Local playback's
settings offer **Load subtitle file**. These actions are disabled for the
experimental native-child diagnostic presenter.

The video picker accepts MP4/M4V/MOV with an `ftyp` header and Matroska/WebM with
an EBML header. It does not import playlists, manifests, URLs, or image sequences.
Subtitle selection accepts UTF-8 SRT and WebVTT up to 2 MiB, checks their opening
structure, and is limited to the current local video. Provider caption selection
continues to own captions for YouTube playback.

The asynchronous picker does not read file contents on the UI thread. A dedicated
worker canonicalizes the explicit path, checks its extension and regular-file
type, and reads a bounded header or subtitle body. Both its request and result
queues have capacity one. Unix opens additionally reject final symlinks and use
nonblocking mode to avoid waiting on a replaced FIFO. Validation does not make a
mutable user file immutable or qualify every filesystem/platform. The worker is
joined during shutdown; it never launches a helper process.

Each selection carries a serial plus the native load, catalog generation, and
account playback selection. Newer work invalidates stale completions. Cancelling
a selection retires its result and picker task; it does not claim to interrupt
an already-running operating-system filesystem call. Data clearing cancels
outstanding selections synchronously and blocks new admission. Saved playback
preferences must finish loading before selection is admitted.

Opening a validated video cancels remote resolution, clears account playback
authority and remote metadata, and stops the existing player. Its local load is
submitted only after the player's asynchronous stop barrier completes. The same
player and presentation context remain in use, preserving volume and playback
speed. A finite deadline reports an incomplete stop instead of retrying in a
loop. A newer selection or cancellation retires the pending handoff.

Local paths are not put in the library, provider models, optional watch history,
or Share. Only the filename is shown in the watch-page title. Account collection
access remains connected independently; selecting a local file does not sign
the user out of the account.

Subtitle attachment rechecks the exact active local load. The engine retains its
path lease through command/playback ownership, and the UI reports selection only
after the engine identifies that subtitle as selected. A finite deadline reports
an unresolved operation. An already-submitted subtitle command is not presented
as cancellable; replacing/stopping playback still retires its UI authority.

The media boundary additionally forces local loads through the reviewed libav
demuxer and an explicit container/subtitle whitelist, with nested references
disabled. Header checks alone are not treated as a network-security boundary.
See the local-format policy in `crates/media/src/commands.rs` for the exact list;
existing CLI diagnostic formats have a broader list than the interactive picker.

Native picker interaction, subtitle rendering/selection, live account-to-local
handoff, negative egress, and platform qualification remain pending. Compilation
and source review do not establish those runtime results. No new performance
measurement is claimed.
