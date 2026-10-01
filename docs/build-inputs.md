# Reproducible application input inventory

`scripts/build_inputs.py` replaces ad hoc source-file hash captures with one
bounded offline inventory. It does not build the application or associate an
existing executable with its inputs. All thirteen synthetic tests pass. A first actual repository capture recorded
133 inputs (1,610,223 bytes) and correctly reported a modified tree; that capture
predates the final lint-only focus edit and is not final binary provenance.

After the hold, run:

```sh
python3 -m unittest discover -s scripts -p 'test_build_inputs.py'
python3 scripts/build_inputs.py --root . --output artifacts/build-inputs-new.json
```

The output must be a new file. It contains only relative input names, sizes,
modes, content hashes, exact HEAD revision and comparison states. Its scope is
every non-ignored untracked or tracked file under `crates/` (including native
glue, `.slint`, icons and build scripts), `Cargo.toml`, `Cargo.lock`, either
standard Rust toolchain filename, and `.cargo/config`/`.cargo/config.toml`.
Tracked files remain included even if ignored. `git ls-files --cached --others
--exclude-standard -z` supplies the working list; HEAD's selected tree adds
committed files removed from the index so staged deletions cannot disappear.

Raw file hashes and executable modes are compared to HEAD blob IDs, independently
of staged content and Git clean filters. Each input is marked `matches_commit`,
`modified`, `added_index`, `untracked` or `missing`. Unrelated changed documents
do not make identical application inputs dirty: `repository_clean` is deliberately
unknown. `inputs_match_commit` only describes this selected scope. A reproducible
inventory of a dirty tree still has exact SHA-256 inputs, but is not presented as
the committed source tree.

Input paths cannot traverse symlinked parents or special files. Reads reject
mutation detected through file size/timestamps/mode, and Git revision/membership
is checked again at the end. This is not a filesystem snapshot: freeze source
throughout capture and the separately logged locked build. The tool cannot prove
that an earlier binary used those bytes; `binary_build_association_verified`
always remains false. Binary SHA/build-command evidence must be recorded
separately rather than inferred from a clean inventory.

Limits are 20,000 files, 16 MiB per file, 128 MiB accepted total input, 8 MiB per
Git response, ten seconds per Git command and a shared sixty-second capture
budget. File reads check that deadline before and after each chunk; these checks
are cooperative and cannot interrupt a single blocked filesystem operation.
There is no hard whole-capture real-time guarantee. The process-group supervisor retains the child leader until cleanup and
requires positive group disappearance after reaping. Git hooks/fsmonitor/global
configuration, lazy fetching and external protocols are disabled. Local repository
configuration and ignore rules remain trusted inputs to Git's ordinary listing.
Only the reviewed Unix supervision path is supported; no Windows qualification
is implied.

Ignored files, environment variables, Cargo configuration supplied elsewhere,
external/generated inputs, compiler/SDK/native-library identities and dependency
corresponding source are separate provenance requirements. The inventory does
not scan a user's home directory, compile dependencies, fetch Git objects, or
run application/native helpers. It is an application input record, not a full
build-environment or licensing SBOM.

## Windows libmpv input

Windows has no pkg-config. `crates/media/build.rs` links libmpv from `MPV_DIR`
when the target OS is Windows (pkg-config remains the default everywhere else).
The directory must contain `include/mpv/client.h` declaring client API 2.5 or
newer (checked by the build script), the runtime `libmpv-2.dll`, and an import
library: an MSVC `mpv.lib` if present, otherwise `libmpv.dll.a`, linked
verbatim. The release workflow generates `mpv.lib` with
`packaging/windows/mpv_import_lib.py` (dumpbin/lib.exe) and so links it; CI
links `libmpv.dll.a` directly. The latter's COFF short-import members are accepted by MSVC
`link.exe`, so no import library is regenerated. The build script copies the DLL
into its `OUT_DIR` so `cargo run`/`cargo test` find it; packages must ship
`libmpv-2.dll` beside `oxplay.exe`.

CI and packaging use exactly one pinned archive, fetched and verified by
`scripts/ci/install-mpv-windows.sh` before extraction:

| Field | Value |
|---|---|
| Source | [shinchiro/mpv-winbuild-cmake release `20260928`](https://github.com/shinchiro/mpv-winbuild-cmake/releases/tag/20260928) |
| Asset | `mpv-dev-x86_64-20260928-git-e470f8986e.7z` (mpv git `e470f8986e`, client API 2.5) |
| SHA-256 | `81795d759e01016f1550fd71651a1a5d59ab5c28ef31c0b6793224e9cff39459` |
| Contents | `libmpv-2.dll` (static FFmpeg, OpenSSL TLS with Windows store support), `libmpv.dll.a`, `include/mpv/*.h` |

This is an mpv development snapshot, not the 0.41.0 release used on Linux; the
official mpv 0.41.0 Windows assets contain only the player, not libmpv. The
x86-64 baseline build is used rather than the `-v3` (AVX2) variant. shinchiro
retains only about 30 releases, so the pin will eventually disappear upstream:
CI caches the extracted archive by installer hash, and a refresh must update
tag, asset name and SHA-256 together in the installer and this table. The
archive's FFmpeg/mpv source correspondence and licensing (GPL build) must be
audited before any Windows binary distribution; see [licensing](licensing.md).
