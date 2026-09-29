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
