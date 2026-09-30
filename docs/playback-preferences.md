# Playback preferences

The shared speed selector and maximum-quality selector now keep local defaults across application restarts. Settings also provides a separate **Default maximum quality** selector for new videos; changing it does not replace current playback. Its displayed value follows the newest accepted pending preference write and rolls back on failure. Defaults remain 1× and 1080p. Supported saved values are the existing UI choices: 0.5×, 1×, 1.5×, 2× and 1080p, 720p, 480p, 360p, 240p, 144p. These are typed domain values; arbitrary database values do not become engine commands or extractor format strings.

SQLite schema 4 adds checked quality-height and speed-milliseconds columns in the existing transaction-based migration. Older databases receive the same private defaults without changing existing collections, volume, theme, or history consent. Clearing local data resets these columns with the other preferences. Account credentials remain outside this database.

Initial guest and explicitly authorized account selections use the current saved or queued quality default. Authenticated requests retain their existing account selection/session authorization checks and never fall back to guest resolution. A quality replacement preserves the current playback position and records its ceiling after native load submission is accepted. A failed save leaves that video's requested quality in effect, reports the persistence failure, and restores the previous default for future selections; it does not launch another replacement. Stream-expiry refresh continues the active video's ceiling without rewriting preferences.

Speed persistence requires an observed native mpv `speed` value after the request, not just successful command enqueueing. One pending change and a single three-second deadline bound the operation. The shared selector displays observed state and is disabled while a change is pending. Failure or timeout leaves the saved preference unchanged; a failed persistence operation attempts one restoration to the acknowledged saved speed. There is no polling or retry loop. A close before native confirmation does not record an unconfirmed speed.

Preference writes use the existing bounded off-thread library worker. Each accepted write has a nonsecret correlation token, so an older failure cannot discard a newer write with identical values. Updates merge pending preference fields rather than overwriting a queued privacy choice. Initial preference hydration runs once; later collection summaries cannot reset a volume or speed change. Startup URL resolution waits for hydration, preventing an existing saved ceiling from being bypassed during startup. An explicit search during hydration receives a useful retry status.

The explicit local-file startup now arms the same bounded focus-intent mechanism before showing the window. It can wait for native activation and player creation, but later keyboard/pointer input, focus loss, or occlusion cancels it. The existing native local diagnostic now checks that initial player focus exists before its first synthetic keyboard action. The frozen f6dea13 native run failed this new assertion before its first synthetic keyboard action; see the retained failure below. Accessibility focus changes that bypass the public native input filter remain qualified as described in [focus-intent.md](focus-intent.md).

## Validation

Focused validation on 2026-09-29 passed: 6 core tests, 21 storage tests (one explicit Keychain test ignored), 91 application tests, and the existing 38 media tests (one explicit hardware test ignored). A subsequent real libmpv null-output test passed, verifying observed 1.5× speed survives two local loads and stop, then resets to 1× before another load. Strict workspace all-target Clippy passed; strict media Clippy also passed after that final test addition. The compiled shared Slint UI passed `cargo check --locked -p oxplay`.

Regression coverage includes version-3 migration, all 24 quality/speed combinations across database reopen, clear-to-default behavior, invalid storage values, failed migration rollback, native speed-property validity, new-observation acknowledgement, correlated duplicate preference writes, all supported initial resolution ceilings, and startup focus cancellation. The native restart and coordinated-clear checks below passed; persistence-failure presentation remains pending native validation. No real account credentials or account requests were used, and these results make no performance or additional-platform claim.

## Finite offline native diagnostic

`--preferences-smoke-test write --data-root /absolute/new-diagnostic-root` creates a new private diagnostic profile and marker; it refuses an existing leaf. The `verify` phase reuses only a private root with the exact private, regular, non-symlink diagnostic marker. Both phases require at least 24 seconds. Do not use a normal application data directory.

The write phase invokes the real Settings quality callback plus the player speed and volume callbacks, then asserts native speed observation, shared selector state, library-worker acknowledgement, and the next-selection resolution policy (720p, 1.5×, volume 37). A second application invocation with `--preferences-smoke-test verify` asserts hydration of those saved values before making another callback-driven change to 480p, 2×, volume 62. It then invokes the real clear-local callback, attempts preference changes during its admission barrier, and checks native, UI, and acknowledged persisted defaults after completion. Skipped timer stages or an early exit fail `Smoke::finish`.

This diagnostic makes no extractor requests and loads no media. It validates the shared policy used by initial guest/account selection, not an authenticated resolver or account acceptance. Both native phases have now passed as recorded below. The filesystem-admission test (including FIFO, Unix socket, symlink, private permissions and marker contents) and finite-stage completion test pass; strict application all-target Clippy and shared Slint compilation also pass.


## Frozen f6dea13 native results

Source `f6dea1361cdd59d2f7a26315a6002e5c2a25fc85` is associated with release
SHA256 `b6d39791e3535466abaeb9b7cce2a737577af6c2b11d6efc126c75129f8435e3`.
All 119 captured input hashes independently match the commit. Central validation
reported 261 Rust tests passing, three ignored, 63 Python tests, formatting,
strict workspace all-target Clippy and a locked release build.

On macOS, write exited 0 after 24.772 seconds and saved 720p, 1.5× and volume 37.
A separate verify invocation exited 0 after 24.284 seconds: startup restored those
values, callback-driven changes reached 480p/2×/62, and coordinated clear restored
1080p/1×/100. Native speed was observed, not inferred from enqueue success. The
post-write harness captured schema 4 and saved values. Independent read-only
inspection after verify found schema 4, defaults 1080p/1000 speed-millis/100,
all five optional privacy flags off, empty local tables and no VTT files. Both
phases had zero media loads or catalog changes/resets; they performed no extractor
or account requests.

The separate local playback lifecycle **failed**, exiting 101 after 3.210 seconds
at `explicit local startup did not focus the player before smoke input`. Before
that failure, its finite composed-video comparison passed with 838/9,216 changed
grid pixels. Motion success does not override the failed focus assertion: the
full local lifecycle and soak readiness are not qualified at this checkpoint.
A later correction must carry its own source/build association and native result.

[Sanitized logs, metadata, source hashes and storage audit](evidence/2026-09-29-preferences-native-audit.json)
retain both successful preference phases and the local failure. Private profile
paths and raw databases are excluded. These are finite functional tests, not
resource, real-account, other-platform or accessibility acceptance.


The later `ee59eaa`/`464fdcf…` release corrected the initial native activation
handling and passed the 21.071-second local startup/focus lifecycle, including
838/9,216 changed motion pixels. [Focus correction and retained failure chain](focus-intent.md)
identify its separate 119-input source capture. This follow-up does not erase
the f6dea13 failure or relabel the earlier preference phases with a new binary.
A separate hour-long soak is underway, with no completed result asserted here.
