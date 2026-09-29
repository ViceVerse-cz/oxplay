# CI and source previews

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
repository access; only the final release job can write repository contents.
Checkout does not retain Git credentials. Tests and compilation run before
that job, with no write token. The other Serein application's workflow informed
the manual dispatch and platform matrix structure; its unrelated dependencies
and packaging pipeline are not used here.

## Create a source preview

Run **Source release (manual)** on `main`, with a new tag such as
`v0.1.0-dev.1`. Allowed suffixes are `dev`, `alpha`, `beta` and `rc`, followed by
a numeric component. The base version must match `[workspace.package].version`
in the committed `Cargo.toml`. Stable tags are intentionally rejected while
release qualification is incomplete. This workflow does not change versions;
make any version update in a reviewed commit first.

The workflow archives its exact checked-out commit, runs the same CI checks,
then creates a new tag and a **draft prerelease**, never a public/latest release.
It uploads only:

- A source tarball from `git archive`, including the lockfile, workflows,
  toolchain declaration and tracked license files.
- `release.json` associating the tag with the exact source revision/toolchain.
- `RELEASE_NOTES.md` describing its experimental scope.
- `SHA256SUMS` covering those three assets.

The helper reads manifests from the archived commit. Untracked local files,
credentials, `target`, artifacts and the local upstream checkout are excluded.
It does not bundle downloaded dependency source or claim to provide complete
corresponding source for any separately distributed binary. Draft assets expire
from Actions storage after 14 days; the attached release assets remain.

The tag creation fails if a tag already exists; no existing tag or release is
overwritten. If tagging succeeds but release creation fails, inspect that run
and the tag before recovery. Use a fresh prerelease number for another dispatch;
the workflow never silently reassigns the existing tag. A non-main dispatch
skips release work. A failed CI job prevents tag and draft creation.

For a local preview from a committed checkout:

```sh
python3 scripts/release_source.py --tag v0.1.0-dev.1 --output /tmp/serein-source-preview
cd /tmp/serein-source-preview
shasum -a 256 -c SHA256SUMS
```

The output directory must not already exist. Local generation makes no network
requests and does not create a tag or release. Workflow dispatch must be explicit;
normal pushes never publish release assets.

## Binary release blockers

The developer macOS bundle is not uploaded by CI or release workflows. Signing,
notarization, clean-machine portability, complete native/helper notices and
corresponding-source coverage remain incomplete. Linux/X11, native Wayland and
Windows need independent runtime validation. See [packaging](packaging.md),
[licensing](licensing.md) and [source coverage](source-coverage.md). This workflow
does not convert those open gates into release approval.

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
