# CI and releases

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
capabilities, resource budgets, accessibility or portable installation. Windows
is not in this build matrix while its native integration remains unfinished.
See [platform matrix](platform-matrix.md) and [progress](progress.md).

Actions use verified full commit IDs. Workflow tokens default to read-only
repository access; only the release workflow's `publish` job (and the Pages
deploy job, when configured) can write. Checkout never retains Git credentials.

## Releases: nightly and production channels

The release pipeline is modelled on the reference project's
(`~/Code/Serein`): one manually dispatched workflow with a **nightly** and a
**production** channel, a reusable Linux package workflow, an optional signed
package repository workflow, and a Homebrew cask kept in this repository.

| Workflow | Trigger | Purpose |
| --- | --- | --- |
| [`release.yml`](../.github/workflows/release.yml) | manual (`workflow_dispatch`): `channel`, `dry_run` | plan, build every platform, publish |
| [`linux-packages.yml`](../.github/workflows/linux-packages.yml) | called by the release; PRs touching packaging; manual | deb, rpm, Arch, AppImage, tarball |
| [`package-repositories.yml`](../.github/workflows/package-repositories.yml) | called by the release when configured; manual | signed apt/dnf/pacman repositories on GitHub Pages |
| [`ci.yml`](../.github/workflows/ci.yml) | called by the release as its `checks` gate | tests, Clippy, release build |

### Run a release

Open **Actions → Release (manual) → Run workflow** on `main`, choose the
channel and leave **dry_run** off. Publishing refuses any other branch.
Tick **dry_run** (allowed on any branch) to build, package and assemble the
complete release, including the source archive, checksums and notes, without
creating a commit, tag, release, cask update or repository deploy; its
`release-preview` artifact and step summary show what would be published.

### Versions and notes

[`scripts/release.py`](../scripts/release.py) is a standard-library port of the
reference project's semantic-release planner:

- The next stable version comes from the commits since the last stable
  `vX.Y.Z` tag: `feat` → minor, `fix`/`perf`/`revert` → patch, `!` or
  `BREAKING CHANGE:` → major; `docs`, `chore`, `ci`, `build`, `test`, `style`
  and `refactor` alone do not release. **Adaptation:** a subject that is not a
  conventional header counts as a patch, because this history is plain English.
- With no stable tag yet, the first stable release is the committed workspace
  version (`0.1.0`), not semantic-release's `1.0.0`. Prerelease tags such as
  `v0.1.0-dev.2` and nightly tags never count as releases.
- Nightly: `X.Y.Z-nightly.YYYYMMDD.RUN` (the *next* stable version, UTC date,
  workflow run number), tag `vX.Y.Z-nightly.YYYYMMDD.RUN` on the planned
  commit. Each run creates a new GitHub prerelease, never `latest`; old
  nightlies are not replaced or pruned. No version commit is made.
- Production: `X.Y.Z`. `publish` creates one `chore(release): X.Y.Z [skip ci]`
  commit (workspace `Cargo.toml`, `Cargo.lock`, `Casks/oxplay.rb`) on top of the
  planned commit through the Git data API and fast-forwards `main`
  (`force: false`); if `main` moved while packages built, the release stops
  before tagging. Tag `vX.Y.Z` points at that commit; the release is `latest`.
- Notes group Features, Bug Fixes, Performance, Reverts, breaking changes and
  (adaptation) plain-subject "Changes", with authors, PR links and a
  "New Contributors" section. Nightly notes cover changes since the nearest
  release of either channel; production notes since the last stable release.
  A Downloads section states which platforms, helpers and signing were used.
- `prepare` applies the planned version to every workspace-versioned package
  in `Cargo.toml`/`Cargo.lock` and hands them to all builds as the
  `release-manifests` artifact, so `--locked` builds carry the release version.
  Builds and the source archive fail if the plan or channel changes.

### Jobs

1. **prepare** (read-only): release tooling tests, plan, manifest version,
   detection of optional repository credentials.
2. **checks**: the complete CI workflow.
3. **build** matrix: `macos-26` (ARM64) runs the existing
   `package_macos.py bundle --build --bundle-helpers` (Homebrew libmpv plus the
   reviewed bundled Python/yt-dlp/EJS/Deno runtime), signs and notarizes when
   configured, refreshes the inventory, runs `verify_package.py` and zips with
   `ditto`. `windows-latest` (x86_64) fetches the pinned libmpv, yt-dlp and Deno,
   generates `MPV_DIR/mpv.lib`, builds `oxplay.exe` with a static CRT, stages the
   portable zip and builds the per-user NSIS `-Setup.exe`.
4. **linux**: `linux-packages.yml` (below).
5. **publish** (only write job; skipped in dry runs) / **preview** (dry runs):
   production version commit, source archive of the tagged commit
   ([`release_source.py`](../scripts/release_source.py)), `SHA256SUMS.txt` over
   every asset, notes, then a draft release that becomes visible once every
   asset uploaded. Nightlies then commit the updated cask (`[skip ci]`).
6. **repositories**: `package-repositories.yml` when `PACKAGE_SIGNING_KEY` and
   `PACKAGE_SIGNING_FINGERPRINT` are configured.

`concurrency` allows one publishing release at a time (dry runs queue per
branch) and never cancels a running release. Rust caches are read by release
jobs and saved only from `main`.

### Assets

| Asset | Contents |
| --- | --- |
| `oxplay-<tag>-macOS-ARM64.zip` (+ `.inventory.json`) | `Oxplay.app` with relocated libmpv closure, bundled helper runtime and build evidence |
| `oxplay-<tag>-Windows-X64.zip`, `-Windows-X64-Setup.exe` | `oxplay.exe`, `libmpv-2.dll`, `yt-dlp.exe`, `deno.exe`, notices |
| `oxplay-<tag>-Linux-ubuntu-24.04-oxplay_<ver>_amd64.deb` | private libmpv 0.41 in `/usr/lib/oxplay` |
| `oxplay-<tag>-Linux-fedora-44-oxplay-<ver>.x86_64.rpm` | Fedora's libmpv |
| `oxplay-<tag>-Linux-arch-oxplay-<ver>-x86_64.pkg.tar.zst` | Arch's libmpv |
| `oxplay-<tag>-Linux-X64.AppImage` (+ `.zsync`), `-Linux-X64.tar.gz` | libmpv with its non-system closure |
| `oxplay-<tag>-source.tar.gz`, `release.json` | exact tracked source of the tagged commit |
| `SHA256SUMS.txt` | every asset above |

Package layouts, helper selection and library bundling are described in
[packaging](packaging.md#release-packages-by-platform).

### Optional secrets and settings

Nothing is required for a release. Without the settings below, releases still
publish: the macOS app stays ad-hoc signed and repository publishing is skipped.

| Name | Kind | Enables |
| --- | --- | --- |
| `MACOS_CERTIFICATE_BASE64` | secret | Developer ID Application certificate + private key (`.p12`, base64) |
| `MACOS_CERTIFICATE_PASSWORD` | secret | password of that `.p12` |
| `MACOS_SIGNING_IDENTITY` | secret | full `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_SPECIFIC_PASSWORD` | secrets | notarization |
| `PACKAGE_SIGNING_KEY` | secret | ASCII-armored private GPG key for apt/dnf/pacman repositories |
| `PACKAGE_SIGNING_FINGERPRINT` | variable | that key's full fingerprint |

Signing runs only when all six Mac secrets exist. It uses an ephemeral keychain
removed afterwards, masks the generated keychain password and prints no secret
([`sign-release.sh`](../packaging/macos/sign-release.sh), with an offline
regression in `test_sign_release.sh`). Repository deploys also need
**Settings → Pages → Source: GitHub Actions**. The production commit and the
nightly cask commit are pushed with `GITHUB_TOKEN`; a branch protection rule
on `main` must allow GitHub Actions to push (or the job fails before tagging).

### Pinned inputs

Third-party downloads are pinned by URL, size bound and SHA-256 in
[`scripts/fetch_pinned.py`](../scripts/fetch_pinned.py): yt-dlp 2026.08.19 and
Deno 2.9.7 (the same versions as the macOS Homebrew route), the
`shinchiro/mpv-winbuild-cmake` `20260928` mpv-dev archive, appimagetool 1.9.1,
type2-runtime 20251108 and rustup-init 1.29.1. mpv 0.41.0 source is pinned in
`scripts/ci/install-mpv-linux.sh`. Update a pin (and `packaging/licenses/`)
only in a reviewed commit using the upstream digest. shinchiro prunes old
builds; if the pinned archive disappears the Windows job fails closed until the
pin is updated. Homebrew (macOS) and distribution packages (Fedora/Arch/Ubuntu
build dependencies, NSIS via Chocolatey 3.11) are verified by their package
managers but float with those repositories.

### Recovery

A failure before `publish` changes nothing. If the production commit reached
`main` but the release failed, either finish it with the same tag manually or
revert that commit; a rerun plans the next version from the last stable tag.
An existing tag is never reused or moved.

### Linux packages workflow

`linux-packages.yml` also runs on pull requests that touch packaging:

- **Ubuntu 24.04 runner**: builds mpv 0.41.0 from the verified source (cached
  with the same key scheme as CI), builds `oxplay` once, then the `.deb` (via
  `dpkg-shlibdeps`, private `libmpv.so.2` with `RUNPATH=$ORIGIN`) and the
  AppImage/tarball, and runs the synthetic signed-apt regression.
- **fedora:44 and archlinux containers**: an unprivileged user installs the
  pinned rustup-init and the checked-in toolchain, builds against the
  distribution libmpv (client API ≥ 2.5 enforced) and builds/inspects the
  rpm or Arch package.

Every package is inspected before upload: metadata, dependencies, payload file
set and bytes, modes and owners, no maintainer scripts, desktop entry
validation and an `ldd` closure check. Nothing is installed or launched.

### Differences from the reference pipeline

Adapted: the planner is Python instead of Bun + semantic-release (plain commit
subjects are patches; the first release uses the workspace version); Mac
signing is optional instead of required and signs the nested helper closure;
Linux deb/AppImage bundle libmpv; the Linux distribution set is Ubuntu 24.04,
Fedora 44 and Arch; Windows is x86_64 only; a source archive is attached
(GPL); a `dry_run` input exists. Skipped: Flatpak (needs offline Cargo
vendoring and an mpv/FFmpeg module), openSUSE (its official FFmpeg lacks common
codecs), Windows ARM64 and macOS x86_64 (no qualified inputs), and the in-app
AppImage/Windows updater the reference project ships.

For a local source archive from a committed checkout:

```sh
python3 scripts/release_source.py --output /tmp/oxplay-source
cd /tmp/oxplay-source && shasum -a 256 -c SHA256SUMS.txt
```

### Release blockers

Releases remain experimental builds, not qualified products. Without the Mac
secrets the app is ad-hoc signed and quarantined on download; Windows and Linux
binaries are unsigned (SmartScreen will warn). Clean-machine portability,
complete native/helper notices and corresponding source for libmpv/FFmpeg
builds remain open, and Linux/X11, Wayland and Windows playback need runtime
validation. See [packaging](packaging.md), [licensing](licensing.md) and
[source coverage](source-coverage.md).

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
