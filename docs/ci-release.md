# CI and source previews

Latest feature checkpoint: `6b3729b46e8ff6ac250307084aa5a21a5677e5f9`
adds channel handles, local playlist search and explicit watch-link sharing.
Formatting, strict all-target Clippy and the locked debug workspace build passed.
Four new focused regression tests were compiled, not run. No native diagnostics,
live provider/account checks, performance measurements, release build or source
preview ran in this fast feature pass. [CI run 36607352745](https://github.com/ViceVerse-cz/yt/actions/runs/36607352745)
could not start either job: both annotations cite failed account payments or a
spending-limit issue. No hosted build/test evidence exists for this source.

Previous feature checkpoint: `dd26ab4f89db2bc9ffd1a9481e613c0c46323435`
adds automatic inline controls, remembered PiP geometry and retry handoff guards.
Local formatting, locked workspace compilation, strict all-target Clippy and
locked debug build passed. Per the requested fast feature workflow, no test
suite, native diagnostic, performance measurement, release build or source
preview was run for this checkpoint. Historical evidence below remains scoped
to its recorded source. [CI run 36605325904](https://github.com/ViceVerse-cz/yt/actions/runs/36605325904)
failed before any step; both annotations report failed account payments or a
spending-limit issue. No hosted build/test result exists for this checkpoint.

[CI](../.github/workflows/ci.yml) runs on pushes to `main`, pull requests, manual
dispatch and calls from the release workflow. It uses `rust-toolchain.toml`
(Rust 1.98.1 with rustfmt and Clippy), the committed `Cargo.lock` and the selected
production Slint features. It never enables every renderer feature at once.

The matrix checks macOS ARM64 (`macos-26`) and Linux (`ubuntu-24.04`). Each job
checks formatting, strict Clippy, automated Rust tests, Python tooling tests and
a locked release workspace build. Ignored human-account, Keychain and external
network tests stay ignored. Python tooling runs under an explicitly selected
Python 3.14.7, including the source archive job. CI checks the required process
supervision primitives before building. The initial run caught that Python 3.12
on macOS lacks `os.waitid`; it became available there in
[Python 3.13](https://docs.python.org/3.13/library/os.html#os.waitid).
Jobs do not launch GUI, performance or usage tests.
No account credentials, signing identities or external service secrets are needed.

macOS installs Homebrew libmpv and checks client API >=2.5. Ubuntu's system mpv
is too old for that API. [The CI installer](../scripts/ci/install-mpv-linux.sh)
builds unmodified official mpv 0.41.0 from a SHA-256-verified archive into an
isolated runner directory with EGL, X11 and Wayland enabled. It records its
Meson build options and native dependency inventory. Native cache keys include
the installer and actual installed package versions; Rust caches also include
those native inputs. Dependencies installed through the OS package managers
can advance; their exact inventory is printed in each run.

The Linux build includes LuaJIT because mpv 0.41 defines `ytdl`, `osc` and its
built-in script controls only when Lua support is compiled in
([upstream option definitions](https://github.com/mpv-player/mpv/blob/v0.41.0/options/options.c)).
The application explicitly sets those controls to `no`, along with inherited
script loading. The first Linux CI run built libmpv with Lua disabled and caught
20 native media initialization failures from unavailable options; it passed
168 app tests, 10 core tests and 37 other media tests before stopping (one media
test explicitly ignored). The corrected native build preserves the application's
privacy settings and test coverage. The
[corrected run](https://github.com/ViceVerse-cz/yt/actions/runs/36574672656)
passed every check on both platforms, including the unchanged media tests and
both locked release workspace builds.

The corrected macOS ARM64 job for source `036da68` completed successfully:
formatting, strict Clippy, 369 Rust tests (four explicit integrations ignored),
163 Python tests and the locked release workspace build. Its Homebrew mpv was
0.41.0_9, reporting client API 2.5.0. This is distinct from the development
machine's package revision. The source archive helper was also exercised on
the actual initial snapshot `97d0263`: all 784 tracked files were included and
the generated asset checksums verified. The manual draft-creation workflow has
not been dispatched; no GitHub release or tag has been created.

The Ubuntu24.04 job passed 351 Rust tests (one explicit external integration
ignored), all 163 Python tests, formatting, strict Clippy and the locked release
workspace build. Its native inputs included mpv 0.41.0 / client API 2.5.0,
FFmpeg development packages `7:6.1.1-3ubuntu5`, libplacebo `6.338.2-2build1`
and LuaJIT `2.1.0+git20231223.c525bcb+dfsg-1ubuntu0.1`. Platform-specific test
counts differ because macOS adapters have their own tests. No runtime desktop,
account, performance or usage test was run.

The subsequent [PiP/account-recovery run for c195894](https://github.com/ViceVerse-cz/yt/actions/runs/36581318438)
also passed every job: macOS 379 Rust tests (four ignored), Linux 361 (one
ignored), and 163 Python tests on each runner, formatting, strict Clippy and
locked release builds. The exact-commit source archive was generated and its
checksums verified locally; the new PiP sources and unchanged Lucide icon/license
were present. This remains a source preview, with no release/tag dispatched.

The [local Home run for 42c937e](https://github.com/ViceVerse-cz/yt/actions/runs/36585092789)
passed both jobs: macOS395 Rust tests (four ignored), Linux377 (one ignored),
163 Python tests each, formatting, strict Clippy and locked release compilation.
The source archive for that commit also passed local checksum/content checks,
including the new Home modules and schema-v5 migration. Native Home evidence
is documented separately in [local-home.md](local-home.md).

Compilation with both Linux backends is not an X11 or native Wayland runtime
qualification. Neither CI job establishes working hardware decoding, account
capabilities, resource budgets, accessibility or portable installation. A
separate `windows-latest` job fetches the pinned libmpv development archive
([build inputs](build-inputs.md#windows-libmpv-input)), sets `MPV_DIR` and runs
formatting, strict Clippy, the automated Rust tests and a locked release build
for `x86_64-pc-windows-msvc`. It skips the Python tools, which require Unix
process supervision, and never opens a native window. It also runs the ignored
synthetic Credential Manager roundtrip, which is safe on the ephemeral runner.
Its first full pass is [run 36793555368](https://github.com/ViceVerse-cz/oxplay/actions/runs/36793555368)
(577 Rust tests, five ignored); see [platform matrix](platform-matrix.md#windows-port-x86_64-pc-windows-msvc).
See [platform matrix](platform-matrix.md) and [progress](progress.md).

Actions use verified full commit IDs. Workflow tokens default to read-only
repository access; only the final release job can write repository contents.
Checkout does not retain Git credentials. Tests and compilation run before
that job, with no write token. The other Oxplay application's workflow informed
the release job structure; its unrelated dependencies and signing/packaging
pipeline are not used here.

## Create a preview release

Run **Release (manual)** on `main`. Leave the tag empty to use the next unused
`v<version>-dev.N`, or supply one such as `v0.1.0-dev.2`. Allowed suffixes are
`dev`, `alpha`, `beta` and `rc`, followed by a numeric component. The base version
must match `[workspace.package].version` in the committed `Cargo.toml`. Stable tags
are intentionally rejected while release qualification is incomplete. The workflow
does not change versions; make any version update in a reviewed commit first.

Jobs, following the structure of the other Oxplay application's release pipeline
(plan, build matrix, publish):

1. **source** resolves the tag and archives the exact checked-out commit.
2. **checks** runs the same CI workflow in parallel with the builds.
3. **macos** (`macos-26`, ARM64) installs Homebrew mpv and yt-dlp, runs
   `scripts/package_macos.py bundle --build --bundle-helpers`, audits the result with
   `scripts/verify_package.py` and zips it with `ditto`.
4. **linux** (`ubuntu-24.04`, x86_64) builds the locked release binary against the
   isolated mpv 0.41.0 and `scripts/ci/package-linux.sh` tars it with that libmpv.
5. **publish** runs only when every job passed. It verifies the source checksums,
   regenerates `SHA256SUMS` across all assets, creates the tag and a **prerelease**
   (never `latest`). It is a draft unless **publish** was ticked.

Assets: `oxplay-<tag>-source.tar.gz`, `oxplay-<tag>-macOS-ARM64.zip` with its
`.inventory.json`, `oxplay-<tag>-Linux-X64.tar.gz`, `release.json`,
`RELEASE_NOTES.md` and `SHA256SUMS`. Unlike the other application, no Apple
secrets are required because nothing is Developer ID signed or notarized: the macOS
bundle is ad-hoc signed and quarantined on download (`xattr -dr com.apple.quarantine
Oxplay.app`). The pipeline adds no conventional-commit versioning, nightly channel,
Windows build or package repositories. Draft assets expire from Actions storage
after 14 days; the attached release assets remain.

The tag creation fails if a tag already exists; no existing tag or release is
overwritten. If tagging succeeds but release creation fails, inspect that run and
the tag before recovery. A non-main dispatch skips all release work, and a failed
CI or build job prevents tag and release creation.

For a local source preview from a committed checkout:

```sh
python3 scripts/release_source.py --output /tmp/oxplay-source-preview
cd /tmp/oxplay-source-preview
shasum -a 256 -c SHA256SUMS
```

The output directory must not already exist. Local generation makes no network
requests and does not create a tag or release. Workflow dispatch must be explicit;
normal pushes never publish release assets.

## Binary release blockers

The uploaded binaries are development builds, not release approval. Developer ID
signing, notarization, clean-machine portability, complete native/helper notices and
corresponding-source coverage remain incomplete. The Linux tarball is experimental
and depends on system FFmpeg and yt-dlp. Linux/X11, native Wayland and Windows need
independent runtime validation. See [packaging](packaging.md),
[licensing](licensing.md) and [source coverage](source-coverage.md).

## Initial repository snapshot

The initial GitHub commit contains the current reviewed source snapshot. Earlier
development commits are retained in a local archive branch, not pushed as public
history. Historical hashes in evidence documents identify those local checkpoints;
they are not advertised as remotely fetchable commits. Evidence remains unchanged
apart from removing a personal filesystem path from explanatory prose.

## Guest artwork cache checkpoint

Source `c6e5c14` passed [both CI jobs](https://github.com/ViceVerse-cz/yt/actions/runs/36588787500).
macOS passed 411 Rust tests with four explicit external integrations ignored;
Linux passed 393 with one ignored. Both passed 163 Python tooling tests,
formatting, strict Clippy and locked release workspace builds. No performance,
usage, desktop or live-account test ran in CI. The local exact-commit source
preview includes `artwork.rs`, schema v6 and its documentation; all generated
asset checksums were verified. The release workflow was not dispatched, and no
tag or release was created.

## Playlist keyboard and controlled settings checkpoint

Source `fa67cba` is pushed. [Its CI run](https://github.com/ViceVerse-cz/yt/actions/runs/36594008344)
failed before either job started. Both check annotations say recent account
payments failed or the spending limit must be increased; the macOS check also
notes ARM64 runner capacity constraints. No checkout, build or test step ran,
and there are no runner logs. This is a hosted-runner admission failure, not a
passing CI result or evidence of a source compilation failure. Repository billing
needs resolution before rerunning that exact source.

Local locked checks passed 414 Rust tests (four explicit integrations ignored),
163 Python tooling tests, formatting, strict Clippy and debug/release builds.
The final native playlist keyboard diagnostic passed all twelve stages in both
builds; its scope and captures are documented in
[library reconciliation](library-reconciliation.md#keyboard-name-editing-and-confirmations).
The exact-commit source preview and every checksum verified locally, including
the new diagnostic and shared controls. No performance/usage test ran, and no
release/tag was published. Linux compilation for this source remains unverified.

## Inline controls, borderless PiP and collection windows checkpoint

Source `aba082ebda0765e7444570ae40402a7cccf50b37` is pushed.
[CI run 36598316363](https://github.com/ViceVerse-cz/yt/actions/runs/36598316363)
failed before either job executed a step. Both annotations again report failed
account payments or a spending-limit issue; macOS also has an ARM64 capacity
notice. No remote compilation or test result exists for this source. Hosted
runner admission remains blocked until repository billing is resolved.

Local formatting, strict Clippy, 428 Rust tests (four explicit integrations
ignored) and locked debug/release workspace builds passed. The final local
PiP exercise passed nine stages and three correlated native frame/stacking
probes in both builds. The collection diagnostic passed fourteen stages and
persisted-state checks in both builds. Scope and captures are documented in
[PiP](picture-in-picture.md#inline-controls-and-borderless-update) and
[collection windows](library-reconciliation.md#created-playlist-windows-beyond-the-first-page).
No performance or usage benchmarks were run. Python tooling was unchanged in
this slice; its preceding 163-test pass remains historical evidence.

The local exact-commit source preview includes `collection_window_smoke.rs`,
`playlist_window_tests.rs` and both PiP modules. The source archive, release
metadata and notes all passed their SHA-256 manifest verification. No workflow
dispatch, tag or release was created. Linux compilation for this source remains
unverified.

## Controlled selectors and caption ordering checkpoint

Source `dfc30db1173d712af0afd7c7aa05fc740d17b679` is pushed.
[CI run 36601158670](https://github.com/ViceVerse-cz/yt/actions/runs/36601158670)
failed before either job ran a step. Both annotations report failed account
payments or a spending-limit issue; macOS also has an ARM64 capacity notice.
There are no hosted compilation or test results for this source. Repository
billing remains the runner-admission blocker; no automatic rerun was requested.

Local formatting, strict all-target Clippy, 443 Rust tests (four explicit
integrations ignored), 163 Python tooling tests and locked debug/release builds
passed. The Rust total includes five tests of the real compiled shared controls
using the development-only Slint mock backend. No pixels or native OS input are
involved in those tests. CI's ordinary debug test command includes them; release
builds omit their element metadata and the test target. Native null-output
caption tests cover pending-On/Off ordering, command saturation, keep-open EOF,
exact completion and lease ownership.

The final release passed the five-stage public guest-caption check and nine-stage
local PiP check, with three native frame/stacking probes for PiP. Their evidence
and limitations are documented in [captions](captions.md#correlated-off-completion-and-controlled-selection)
and [PiP restoration](picture-in-picture.md#restore-the-original-display).
No account, performance or additional-platform qualification is inferred.

The exact-commit local source preview contains `controlled_widgets.rs`,
`subtitle_off.rs`, the updated lockfile and integration notes. Source archive,
release metadata and notes all passed their SHA-256 manifest verification.
The release workflow was not dispatched; no tag or release was created.
Linux compilation for this source remains unverified.

## Account expiry and library pagination checkpoint

Source `068bc94030f41b13cf74ce4e28424688adddeb67` is pushed.
[CI run 36603317441](https://github.com/ViceVerse-cz/yt/actions/runs/36603317441)
failed before either Linux or macOS ran any step. Both check annotations report
failed account payments or a spending-limit issue; macOS also reports ARM64
capacity constraints. No automatic retry was requested and no hosted test/build
result is claimed for this source.

Local formatting, strict all-target Clippy, 448 Rust tests (four explicit
integrations ignored), 163 Python tooling tests and locked debug/release builds
passed. Both native offline library checks passed all 22 checkpoints, exited zero
and left no owned process groups. Post-exit SQL confirmed the two unchanged
playlist names, 101 saved fixture entries and history off. See the
[functional manifest and logs](evidence/2026-09-29-library-page-functional.json).
Account expiry/no-replay and pause-aware refresh regressions are synthetic;
they do not qualify a live account or live expired-stream replacement.

The local exact-commit source preview's archive, metadata and notes passed their
SHA-256 checks. Archive bytes for account reconciliation, refresh, the extended
native diagnostic and Cargo.lock match the committed files. The release
workflow was not dispatched; no tag or release was created. Performance/usage
tests remain paused, and Linux/Windows runtime qualification remains open.
