# Saving a video into a local playlist

The watch page's Save action no longer requires an existing local playlist. For
an active guest YouTube video, the shared Slint popup offers an existing-playlist
selector and a new-playlist name field. Selecting a destination alone does not
write anything: Save or Create and save is the explicit action. The popup keeps
pending, failure and committed-success messages visible; Close does not cancel
an already accepted local write. This flow cannot mutate a YouTube account.

Opening the popup captures the video identity, native load request and a bounded
snapshot of the displayed local playlist IDs. Submission rechecks that the same
guest load is still current, has actually started, and is not stopped, failed,
seeking or superseded. A different video, quality replacement, account playback,
local-file playback, resource fixture, or native-child diagnostic cannot reuse
the captured target. Refreshing the ordinary library model cannot redirect a
selected popup index to a different playlist.

The existing dedicated SQLite worker accepts a bounded command containing the
explicit destination and normalized video metadata. A new playlist and its first
video are created in one transaction: failed validation or insertion rolls back
the playlist as well. Existing-playlist saves retain the existing idempotent
video upsert behavior. Signed media and thumbnail URLs are not persisted.

Save replies carry an operation serial distinct from generic library and
preference responses. Only the matching committed response displays success;
unrelated or repeated replies cannot finish a newer save. While an accepted save
is pending, the popup prevents duplicate actions. Playback may change after
submission without silently canceling that already-authorized local write.

Regression tests cover SQLite rollback after an injected video-insert
failure, invalid names and metadata leaving no empty playlist, real worker
failure/success correlation and reopening its database, stale or failed native
load rejection, and duplicate/old response rejection. The combined workspace
suite passed 336 Rust tests (four external checks ignored); final app tests
passed 139, and formatting plus strict full-workspace all-target Clippy passed
after integration. Native results are recorded below; screen-reader behavior
remains unvalidated. No account or resource acceptance gate is claimed.

The first coordinated check passed 137 application tests and 28 storage tests
(one explicitly ignored Keychain integration check) plus strict application
Clippy. That result includes the atomic save, worker and stale-load regressions;
it predates the final popup styling and newly linked native diagnostic. The
subsequent frozen release and its native result are identified below.

## Explicit finite native check

`--save-smoke-test --url <public-video-url> --data-root <new-absolute-directory>`
is a separate 75-second diagnostic, not an extension that silently adds writes to
a previously read-only smoke. It refuses an existing root, other diagnostics,
helper overrides, alternate media transports, local files and startup page
changes. Window size/theme and finite diagnostic logging are permitted. It never
connects an account. The selected public video must remain available and be long
enough to reach the 30-second readiness checkpoint; provider/network errors are
failures, not synthetic metadata or skipped stages.

Eight finite checkpoints verify the requested real provider video and empty
private library, reject an empty name, invoke the same shared popup callbacks to
create and save, then save again into that existing playlist. Three new bounded
worker reads verify the committed playlist/video identity and exactly one video
membership. After a fresh popup captures the same video, an actual native stop
must make its Save callback reject the stale load without a new write. Every
checkpoint must finish before the normal 75-second watchdog for success. The
profile is retained for post-exit inspection; no SQLite I/O is performed on the
UI thread. Captures inspect the popup separately from the callback assertions.

An optional explicit `--snapshot <new-root>/save.png` uses the existing one-shot
Slint capture at 37 seconds, while the acknowledged Save popup remains open. Only
a direct `.png` child of the newly admitted private root is accepted in this
mode; sibling, nested, parent-traversal and non-PNG paths are rejected. The
existing encoder thread writes with `create_new`; ordinary snapshot timing
remains 15 seconds. This finite diagnostic CPU readback is never a playback or
performance implementation. The image still requires actual visual inspection.

## Observed native result

The frozen `d78332c23602b1bfaec87688eccba657a208b9be` release
(SHA-256 `e32cfa128282c781a80eeba06a8259207b07f54da946056b77185fa04efb88b9`;
138 committed build inputs, identical before/after inventory) passed this
75-second diagnostic on 2026-09-29. The supervised run took 77.177 seconds including
startup/wrapper overhead, exited 0, completed all eight stages, and confirmed
absence of both owned application and wake-assertion process groups.

The actual public video was Blender's “Big Buck Bunny 60fps 4K - Official Blender
Foundation Short Film” (`aqz-KE-bpKQ`). Playback was observed at 1920×1080/60fps
with `videotoolbox`, Opus 48kHz and `avfoundation`; this functional run makes no
performance claim. Post-exit read-only SQLite inspection confirmed one explicit
diagnostic playlist, exactly one matching video membership after two saves,
public title/channel/duration metadata, no history/follows or account-session
files, and unchanged off-by-default privacy settings. The private 49,152-byte
database remains local and is not a public evidence artifact.

The 37-second snapshot was actually opened and reviewed after cleanup. At 760×600
logical pixels (1520×1200 PNG), the popup, two-line title, local-only disclosure,
committed-success message and Done control are legible and fit without overlap.
The diagnostic invokes the real callbacks rather than typing a name into the
editor; its blank name field and disabled Create button are therefore an
acknowledged diagnostic state, not keyboard-input evidence. The capture hash is
`68dd6e611d14c69b04e4840713ce423c7cb664a193ec9c0e161468aca1c89ff8`.
Original artifacts remain unchanged under `artifacts/ui-workflows-v1/save-release`.
The [public export manifest](evidence/2026-09-29-local-save-manifest.json) records
verified original and exported hashes, including the normalized harness and
exact source inventory. Public evidence includes the
[result](evidence/2026-09-29-local-save-summary.json),
[native log](evidence/2026-09-29-local-save-native.log),
[committed database assertions](evidence/2026-09-29-local-save-database-assertions.json),
[inspected popup](evidence/2026-09-29-local-save-popup.png) and
[visual review](evidence/2026-09-29-local-save-visual-review.json).
No private profile paths or SQLite files are exported. The automatic result's
original `snapshot_visual_inspection_performed: false` is preserved; the
separate later review records the actual inspection.
