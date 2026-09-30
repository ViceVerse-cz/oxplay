# Offline dependency source coverage

`scripts/source_coverage.py` audits local source availability for an **existing**
developer bundle. It does not clear a distribution, replace its SBOM, or claim
complete corresponding source. SPEC §15 and AC-16/AC-23 still require the actual
distributed versions, source, patches, build material and license review.

The default audit runs no Cargo, Git, Homebrew, native helper, package code or
network operation. It neither extracts archives nor copies source trees. It reads the
packaged build manifest, verifies the packaged Cargo graph and application source
archive hashes, and reads `Cargo.lock` from that bounded archive. Its report binds
to the manifest hash; an attacker-controlled manifest is not a publisher trust
root. Use the separate package verifier and independently trusted artifact digest
before relying on these associations.

```sh
python3 -B scripts/source_coverage.py \
  --evidence artifacts/Oxplay-bc2e53f.app/Contents/Resources/BuildInfo \
  --output artifacts/source-coverage-bc2e53f.json
python3 -B scripts/test_source_coverage.py
```

`--cargo-home` and `--brew-cache` select explicit local cache roots. Defaults are
the user's `.cargo` and `Library/Caches/Homebrew/downloads`. Only known registry,
Git checkout and download-cache locations are inspected; browser profiles,
Keychains and credentials are not searched. Reports contain package labels,
hashes and recipe line numbers, not local absolute paths, URLs, usernames or
exception details. Output creation refuses an existing destination. No report
or source payload is uploaded.

## What each result means

- **Registry `verified_archive_bytes`:** a local `.crate` hash matches the selected
  package checksum in the packaged lockfile. This establishes archive-byte
  availability. An extracted directory without a matching archive is recorded
  separately as present but unverified; its editable checksum file is not a
  substitute for the lockfile's archive checksum.
- **Git `checkout_head_matches_unverified_worktree`:** a local checkout's direct
  or symbolic HEAD names the exact locked revision. No Git command is run, and
  the tool does not verify Git objects, modified/untracked files or submodules.
  This result deliberately does not count as verified source completeness.
- **Native `verified_archive_bytes`:** a local source/resource/patch candidate
  matches a literal URL/SHA-256 declaration from the exact packaged Homebrew
  formula. The formula is checked against its recorded package hash. Binary
  bottles never count as source. Cache selection uses a URL-hash prefix or exact
  download basename; final acceptance always requires the content hash.
- **`absent`, `hash_mismatch`, `hash_budget_or_file_limit` or unsupported input:**
  missing coverage remains explicit, with no automatic download or guess.

The mpv recipe has two independently tracked required patch revisions:
`75b2ccfeb1ce4ed5a40ac9860fa74f3d1265e13f` (VapourSynth compatibility) and
`c5d391adba7bd024954d0df1e0405f5749f4d4ca` (macOS audio-device-change fix).
Their SHA-256 values come from the inspected installed `mpv 0.41.0_10` formula.
A changed recorded hash is rejected; a missing recipe declaration is reported.
Cached patch bytes, when present, still do not prove application to source.

Formula parsing intentionally recognizes only adjacent literal HTTPS URL/hash
pairs. It does not execute Ruby or pretend to understand computed URLs, Git
resources, conditional resource sets or all build inputs. A native formula with
no recognized pair therefore has no established coverage. Deno's embedded
Rust/V8/JavaScript sources and notices remain explicitly incomplete even if its
top-level source archive is available. The Homebrew package list and Deno's
top-level MIT text are not a complete embedded-component source/license audit.

## Bounds and follow-up

Metadata reads are limited to 8 MiB, individual hashed inputs to 128 MiB, and
total source/archive hashing to 256 MiB by default (`--max-hash-mib`, 1–2048).
Hashing uses at most 1 MiB blocks. Directory enumeration, Cargo package counts,
Git checkout counts and archive member/declared byte counts are bounded.
Symlinks and special-file reads are rejected or omitted; archive traversal,
links, duplicate lockfiles and mismatching evidence are rejected. Inputs should
remain quiescent. This audit is not a defense against an adversary concurrently
replacing a cache tree or an atomic installer/updater.

Useful next steps are to obtain the exact absent source archives and patches
through an explicit maintainer workflow, verify hashes, retain them in a source
distribution inventory, then reproduce the dependency builds. Full Slint source,
native build flags/resources, toolchain inputs, generated code, notices and any
applicable installation information need separate review. The existing dispatch
notice gap and objc2 SDK-policy questions are not resolved by this collector.
No generated report should be labeled complete corresponding source merely
because all recognized rows have locally available bytes.

Synthetic tests cover package/lock associations, absent and tampered archives,
budget exhaustion, symlinks/FIFOs, unsafe tar members, Git HEAD qualification,
literal recipes, both mpv patches, bottle exclusion and metadata ambiguity.

## Executed local audit — 2026-09-29

All 12 synthetic collector tests passed. The existing packaging boundary suite
passed 14 tests, including the new source-snapshot regression; the artifact
verifier suite also passed all 14 tests. No native process, build or network
request ran. The initial real audit rejected Homebrew's `python@3.14` label;
the label validator now permits the required `@` while still rejecting path
separators, with a regression assertion. No successful report was written by
that failed attempt.

The actual audit of the unchanged `Oxplay-bc2e53f.app` developer artifact hashed
43,995,665 bytes, below its 256 MiB budget. Of 334 selected Cargo packages,
314 registry archive checksums matched, 14 Git dependencies had matching local
checkout HEADs but unverified worktrees, and six workspace crates were associated
with the recorded application source archive. The 51 packaged Homebrew recipes
yielded 70 recognized literal source/resource/patch inputs; none was found with
verified matching bytes in the selected local download cache. Both required mpv
patch declarations were present and their cached payloads absent. Installed
binary bottles were excluded. This is a report of recognized cache candidates,
not proof that no source exists elsewhere or that every recipe input was parsed.

The exported [report](evidence/2026-09-29-source-coverage-bc2e53f.json), also retained
in the ignored local artifacts, has SHA-256
`443c4c94cc23cf1785a12f0476a7cab55bd9f9f90687faef626305f215655b90`.
It binds to packaged build-manifest SHA-256
`2ac9679dffe3acb59e3f53a270c47199523cf771ec35df4e6e247120475fc500`
and application-source archive SHA-256
`25170c02d344794d7ed88bed3965105a60e5649349bb597138f1b343ab025b96`.
The report contains no absolute home/private paths. No artifact or cache input
was modified, and this historical artifact has not been relabeled as current
source or complete corresponding source.

For future bundles, `package_macos.py` now includes `SPEC.md` and
`tools/media-baseline` in the application source snapshot. That fixes the missing
source behind the archived standalone-baseline reproduction instructions while
retaining original ownership/licensing. The snapshot test confirms unrelated
tools, ignored artifacts and symlink payloads remain excluded. The old artifact
audited above remains unchanged; this adjustment is not a new packaged-runtime
or clean-machine validation.


The [evidence summary](evidence/2026-09-29-source-coverage-summary.json) records
the collector script hash, tests, counts and input artifact association. A repeat
after adding plain-tar header preflight produced a byte-identical report. That
preflight bounds extension-record payload declarations before `tarfile` reads
PAX metadata; compressed application-source archives are not accepted because
the packager emits plain tar. The synthetic oversized-PAX regression passed.
The collector source was not yet committed when this report was exported, so
its hash is recorded without inventing a source revision.

## Opt-in immutable Git source export

`source_coverage.py --export-git-sources NEW_DIRECTORY` now has a focused offline
export path in `scripts/git_source_export.py`. After the coordinated soak ended,
all 19 focused synthetic exporter tests passed, including actual local Git reads
of isolated synthetic object databases. The real selected-dependency export
below subsequently passed. The earlier matching-HEAD-only report remains a
historical record; it has not been rewritten to claim object verification.

The normal audit remains subprocess-free. The opt-in path consumes only Git
revisions selected by the already verified packaged Cargo graph and archived
lockfile. Identical revisions are exported once, with their selected package
labels. It reads Cargo's local bare object databases through a fresh private
repository, without loading their configuration, remotes, replacement refs or
the editable checkout. Lazy fetching, external protocols and hooks are disabled;
chained object alternates are rejected. It neither downloads nor mutates caches.

The exporter runs only `/usr/bin/git` built-in object readers. Batched object
size/read commands are followed by independent SHA-1 checks of the exact commit,
all ordinary trees and blobs. Raw tree traversal must equal the complete Git
listing. It then writes its own normalized, uncompressed PAX tar with fixed
ownership/time metadata and preserved regular/executable modes. Committed
`export-ignore` and `export-subst` attributes are deliberately not applied:
ordinary committed bytes must be present unchanged. A second archive read checks
member coverage, modes and every blob/symlink object identity before success.
The tar byte SHA-256 and verified root-tree identity appear in the manifest.

Absolute, traversing, drive/colon, control-character and metadata paths are rejected. Symlinks
are stored without dereferencing; both lexical targets and composed link chains
must remain within the archive root, including an additional Unicode-normalized,
case-folded lookup for common case-insensitive filesystem behavior. Ambiguous
member aliases, cyclic and over-deep chains fail closed. This does not qualify
extraction on every filesystem or under an arbitrary third-party extractor.
Gitlinks become empty directory entries and explicit **uncovered submodule**
records, never automatically fetched source. Git LFS pointers remain exact
committed pointers and are listed as unmaterialized assets. These are immutable
Git-tree archives, not a claim of complete corresponding source, authentic
publisher identity, legal clearance or reproducible dependency compilation.

Limits are 16 selected revisions, 64 local object databases, 30,000 objects/tree
entries per revision, 64 path levels, 128 MiB per blob and 512 MiB of unique
object payloads across the export. Expanded tree payloads across all archives are
separately bounded at 512 MiB because multiple paths can reference one blob.
Git reader commands have a 20-second deadline within a shared 120-second command
budget, bounded disk output and kill/reap cleanup; filesystem operations and OS
scheduling do not have hard real-time guarantees. No shell, build, filter,
archive extraction or archive-supplied code runs. The Unix supervisor fails
closed without the required unreaped-child observation API.

Only a new private directory is accepted. A retained `INCOMPLETE` marker and
diagnostic files identify unsuccessful exports. Successful output consists of
`git-source-manifest.json` and the listed `git-REVISION.tar` files. Private
diagnostic repositories/object streams can contain absolute cache paths and
commit metadata; **do not publish the whole working directory**. The public
manifest uses revision/package/archive labels, not local cache paths. Missing
local revisions are explicitly unavailable rows and receive no archive.

Opt in only on an independently trusted existing package evidence directory.
Required proof is an actual archive whose complete contents match the locked
immutable tree; another successful checkout-HEAD check is insufficient. Source completeness
for submodules, generated code, toolchains, native dependencies and Deno remains
separate work even if this export succeeds.

The first central test attempt exposed Darwin's `EPERM` result when signaling
an unreaped zombie-only process group. Cleanup now reaps the pinned leader and
requires an `ESRCH` group-absence probe; persistent permission failure is never
treated as absence. Synthetic tests cover both outcomes. A focused follow-up
also observed Git exit with `SIGXFSZ` under a process-wide file-size limit. Output
is now bounded by streamed pipe capture instead, without imposing that global
file-write limit. These failed attempts produced no qualified source export.

## Executed immutable-tree export — 2026-09-29

The explicit exporter ran twice against the existing `Oxplay-bc2e53f.app`
evidence, creating separate ignored directories. Both runs independently read
the local object database and produced byte-identical public manifests and
archives. The 14 selected Git packages share locked Slint revision
`cf3b07d4917e6759a63b0c03913a2594ec653414`; every selected package/version/revision
also matches the current workspace lockfile. This cross-check does not promote
the historical artifact to the current application build or cover newer registry
dependencies absent from its graph.

The verified root tree is `77c0790f0a3878682eac1b86707428a1499b2663`.
The archive contains **6,428 tree entries**, with **36,749,882 bytes** of committed
file/symlink payload. Raw commit/tree/blob verification covered **36,867,941
bytes** of unique object payload. No gitlinks or Git LFS pointer blobs appeared
in this tree. That observation does not establish generated, external-resource,
build-tool or transitive dependency completeness.

The normalized tar is **41,410,560 bytes**, with SHA-256
`b3b310b8704bfde1580a8727670ee6bed5063afee6375cdcb13dd706187f6bbd`.
It remains local at
`artifacts/git-source-bc2e53f-20260929/git-cf3b07d4917e6759a63b0c03913a2594ec653414.tar`;
no archive was uploaded or extracted. The independent archive reread verified
ordinary file modes/content, directories and symlinks against the complete
object-verified tree, without using an editable checkout.

The shareable [manifest](evidence/2026-09-29-git-source-export-manifest.json),
[audit](evidence/2026-09-29-source-coverage-git-export.json) and
[provenance summary](evidence/2026-09-29-git-source-export-summary.json) bind the
archive to the exact packaged manifest, graph and lockfile, and record collector
source-file hashes. Manifest SHA-256 is
`c78a0a7afa2a7bf64a4d386868ef49e2df98477ecbb05eb7d74213612aebfcb2`.
Public evidence was checked for absolute home/private temporary paths and
credential-header markers. Private diagnostics were not exported. No Cargo
build, native application, network request or credential access ran.

This closes the narrow **local immutable Git-tree archive availability** gap for
those selected packages. It does not supply missing native/Deno source closure,
authenticate an upstream publisher, demonstrate reproducible compilation or
clear the application for distribution. Those remain separate acceptance gates.
