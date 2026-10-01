# Downloads

The project owner decided to allow explicit downloads (SPEC §11.1). A public
video can be saved for offline viewing one video at a time. Nothing downloads in
the background: there is no automatic, scheduled, playlist, channel or feed
download, and playback buffering is still never written to disk.

## Starting a download

- **Video cards** (Home, search results, public channel/playlist pages and the
  watch page's related list): right-click, or press Menu / Shift+F10, and choose
  **Download**. `card_menu.rs` re-reads the card's public item by surface and
  index, like the other card commands.
- **Watch page**: the compact download button next to Share and Save. It is
  enabled only for a loaded guest (public) video; account playback, local files
  and the native-child diagnostic cannot be downloaded.

A short toast confirms "Added to Downloads" or explains why not (already
downloaded, already queued, queue full, unavailable). The request uses the
**default maximum quality** from Settings (2160p–144p), not the current
video's temporary quality choice.

## Downloads page

The guide's **Downloads** entry (after History, same 36/40 px dense row) opens
page 5. It shows:

- **In progress**: title, creator, a static progress bar (no animation, so a
  download never drives redraws by itself), percent, size, speed and time left
  when known, and **Cancel**. Queued jobs say "Waiting to start"; merging says
  "Merging video and audio…".
- **Downloaded**: thumbnail, title, creator, size, date, height and length, with
  **Play**, **Show in Finder** (Explorer on Windows, "Show in folder" on Linux)
  and **Delete**. Delete asks "Delete this video from your device?" with
  Delete/Keep in place; confirming removes the media file, its artwork and its
  manifest entry.
- An empty state, and a note when quality is limited (no FFmpeg) or the
  manifest needed recovery.

**Play** opens the file through the local-file path (`local_media_ui.rs`): the
same bounded validation worker, stop-before-load handoff and lavf container
whitelist as **Open video file**, but the watch page shows the stored title and
"*creator* · Downloaded". No YouTube request is made, so it works offline. A
download is not a watch tab and is not added to history.

## How it works

`crates/youtube/src/download.rs` runs one download through the existing
supervisor (`supervisor.rs`), extended with a streaming mode: stdout is consumed
as bounded lines (16 KiB each) and only a 64 KiB stderr tail is kept. The helper
gets the same isolation as playback resolution (`--ignore-config`,
`--no-plugin-dirs`, no cache, no cookies from browsers, no remote components, an
empty proxy, the reviewed Deno runtime, a cleared environment, its own process
group) and the shared rate-limit cooldown. It does not take the single playback
extraction slot, so a long download never delays opening a video. Deadlines:
5 minutes without any output, 6 hours overall. Cancellation, a deadline, app
exit and every early return kill and reap the whole process group.

Format selection (`download::selection`):

- **With FFmpeg**: `bestvideo[height<=H][protocol=https]+bestaudio[protocol=https]`,
  falling back to a progressive file at or below `H`, sorted
  `height,fps,vcodec:h264` like playback. Streams are merged into MP4 when the
  codecs allow it, otherwise Matroska (`--merge-output-format mp4/mkv`), via
  `--ffmpeg-location` with the explicit path.
- **Without FFmpeg**: only `best[height<=H][protocol=https][vcodec!=none][acodec!=none]`
  and `--fixup never`. YouTube usually offers such a file only at 360p, so the
  page says quality is limited and the entry is marked "Limited quality (no
  FFmpeg)". If no such file exists at or below the ceiling, the download fails
  with an explanation instead of exceeding it.

FFmpeg is looked up only at `HelperPaths::ffmpeg`: `Contents/Helpers/ffmpeg`
in a bundle, otherwise `/opt/homebrew/bin/ffmpeg` (macOS) or `/usr/bin/ffmpeg`.
PATH is never searched. Release bundles currently ship only yt-dlp and Deno, so
**packaged builds use the single-file fallback** until FFmpeg is reviewed for
bundling.

Live streams are excluded with `--match-filters !is_live`. Downloads are always
anonymous: no account cookies are passed, so members-only, private,
sign-in-required and DRM-protected videos fail ("Downloads use guest access
only") instead of being authorized.

Progress comes from `--newline --progress-template` and `--print` markers
(`OXPPLAN` before download, `OXPPROG` lines about twice a second, `OXPDONE` with
`%(.{id,title,channel,uploader,duration,height,ext})j` after the move). A
`Tracker` combines the separate video and audio streams into one bar weighted by
the planned size, or equally when no size is known, capped at 99 % until all
streams arrive.

## Queue and storage

`crates/app/src/downloads.rs` owns a coordinator thread. At most **two** jobs
run at once; up to 50 more wait in order. Job threads only run the helper and
publish the latest progress (coalesced; one UI wake until it is acknowledged).
The coordinator alone moves files and writes the manifest. The coordinator
thread is idle until Downloads opens or a download starts, and the directory is
created only when the first download starts.

Layout, inside the app data directory (the same `Oxplay` directory as
`library.sqlite3`; `--data-root` moves it too):

```text
downloads/                 0700
  manifest.json            0600, {"version": 1, "items": [...]}
  <video id>.<mp4|mkv|webm|m4v|mov>
  thumbnails/<video id>.jpg   ≤320×180, re-encoded from the helper's artwork
  .staging/<video id>/        per-job helper output; removed afterwards
```

Each entry stores id, title, creator, length, size, completion time, height,
the limited-quality flag, the file name and the artwork name. No signed URL,
cookie or provider response is stored. Writes go to a temporary file, are
fsynced and then renamed over the manifest. Loading is tolerant:

- entries whose file is gone (deleted outside Oxplay) are dropped, sizes are
  refreshed, and missing artwork is forgotten;
- media files named `<video id>.<ext>` without an entry are recovered as
  "Recovered video *id*";
- a damaged manifest is renamed to `manifest.json.damaged` and the present
  files are recovered;
- a manifest with a newer version is shown read-only and never rewritten;
- only `<id>.<ext>` names are ever opened, so an edited manifest cannot point
  outside the folder.

Partial files are removed after cancellation or failure, at the next load after
a crash, and at exit (`downloads_ui::State::shutdown` runs after the event loop).

Downloaded files are user files. **Clear local data does not delete them**;
use Delete on the Downloads page or remove the folder. Their names and the
manifest's titles can reveal viewing interests on this device (see
[privacy](privacy.md)).

## Validation

Pure-Rust tests, no network:

- supervisor streaming: line delivery, line bound, stderr tail, idle timeout,
  cancellation killing the group;
- marker parsing, progress weighting, format selection with and without FFmpeg,
  staged-file discovery, and a synthetic helper script for success, rate-limit
  cooldown, missing output, wrong id and cancellation;
- manifest round trip and permissions, recovery of missing/unindexed files,
  damaged and newer manifests;
- queue order, the two-job limit and cancellation; and the manager with a
  synthetic helper: two running and one queued, cancelling queued and running
  jobs, finalization, a duplicate refusal and deletion;
- status/size/date text.

The controlled-widget test `download_menu_item_guide_entry_watch_button_and_downloads_page_route_commands`
covers the card-menu Download command, the guide entry, the empty state, the
progress bar's accessible value, Cancel/Play/Show/Delete with confirmation and
Keep, and the watch-page button including its disabled state.

`cargo run -p oxplay-youtube --example download -- --yt-dlp … --deno … [--ffmpeg …] [--height N] URL DIR`
is a manual, network-using diagnostic for the runner.

## Manual runner check (2026-10-01, macOS, debug build)

With Homebrew yt-dlp 2026.08.19, Deno and FFmpeg, the example downloaded the
CC BY 3.0 *Big Buck Bunny 60fps 4K* (Blender Foundation, `aqz-KE-bpKQ`,
10:34) anonymously:

| Ceiling | FFmpeg | Result |
|---|---|---|
| 360p | yes | merged MKV, H.264 360p + Opus, 28.4 MB, 48.6 s, WebP artwork, title/creator reported |
| 144p | yes | merged MKV, 14.4 MB, 26.4 s; 30 progress updates, never moving backwards across the video→audio switch |
| 720p | no | progressive MP4, H.264 360p + AAC (format 18), 28.5 MB, 52.2 s: quality limited as documented |

Earlier the same day every guest extraction from this machine was refused with
"Sign in to confirm you're not a bot"; the runner reported
`AuthenticationRequired` without retrying. This check covered only the
runner, not the app's queue, page or offline playback of the result.

## Limits

- Windows fails closed (the supervisor has no job-object implementation yet), so
  downloads show as unsupported there.
- Downloads are not resumed after a restart; an interrupted job must be started
  again. There is no free-space check before a download.
- FFmpeg is not bundled, so packaged builds are limited to progressive files.
- Native screen-reader behavior, the platform file managers' reveal, real
  offline playback of a download in the GUI and resource use during two
  concurrent downloads have not been measured.
