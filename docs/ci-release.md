# CI and source previews

[CI](../.github/workflows/ci.yml) runs on pushes to `main`, pull requests, manual
dispatch and calls from the release workflow. It uses `rust-toolchain.toml`
(Rust 1.98.1 with rustfmt and Clippy), the committed `Cargo.lock` and the selected
production Slint features. It never enables every renderer feature at once.

The matrix checks macOS ARM64 (`macos-26`) and Linux (`ubuntu-24.04`). Each job
checks formatting, strict Clippy, automated Rust tests, Python tooling tests and
a locked release workspace build. Ignored human-account, Keychain and external
network tests stay ignored. Python tooling runs under an explicitly selected
Python 3.12, including the source archive job.
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
