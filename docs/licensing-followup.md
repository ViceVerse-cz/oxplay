# Remaining notice and SDK-policy questions

Source/text review on 2026-09-29. This supplements [licensing.md](licensing.md)
without changing the selected Slint GPLv3 route, package licenses or release
status. No notice was invented, dependency changed, maintainer contacted or
binary cleared. During reserved performance work, only small upstream metadata
responses and existing source files were inspected; no tests, builds, native
application, large archive fetch or source-tree hash loop ran.

## dispatch 0.2.0: the original notice remains missing

The published source revision is
`82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748`. Its original
[Cargo.toml](https://raw.githubusercontent.com/SSheldon/rust-dispatch/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/Cargo.toml)
declares MIT and names its author. The inspected
[README](https://raw.githubusercontent.com/SSheldon/rust-dispatch/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/README.md)
and [library entry point](https://raw.githubusercontent.com/SSheldon/rust-dispatch/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/src/lib.rs)
do not supply a complete grant/attribution notice. The earlier exact-tree audit
is recorded in [the retained manifest](../third_party/notices/manifest.json).

Upstream [issue 18](https://github.com/SSheldon/rust-dispatch/issues/18) is shown
as Closed, but its accessible page provides no replacement license text or
applicability statement for this release. Issue state is not evidence that the
missing material was supplied. The earlier remote reading tool could not load
GitHub's API endpoints. A later, direct unauthenticated API inspection succeeded;
its narrower findings and retained hashes are recorded below. It also recovered
no authoritative full original notice.

An author field is not an original copyright notice, and a generic MIT template
or the same author's notice from another crate does not establish its scope for
this release. In particular, the complete notice already retained for
`objc2 0.5.2` is evidence for that different package, not a replacement
`dispatch 0.2.0` notice. The missing item is an authoritative full grant and
attribution statement applicable to the exact dispatch source, or a reviewed
decision about how its existing license declaration is to be handled.

This package is actually selected. The `bc2e53f` packaged Cargo graph records
`objc2-foundation 0.2.2 -> dispatch 0.2.0`. Winit `0.30.13` explicitly enables
the foundation `dispatch` feature on macOS. Its `platform_impl/macos/event.rs:43`
and `monitor.rs:252` call `run_on_main`; foundation `src/thread.rs:115` implements
that operation through `dispatch::Queue::main().exec_sync`. Removing a manifest
feature without replacing those calls is not a complete fix. Disabling required
accessibility is not an acceptable solution and would not remove Winit's use.
These observations come from the exact already-fetched Cargo sources, not a
claim that a newer dependency has been tested.

Two concrete routes remain:

1. Obtain an explicit upstream notice or applicability clarification for version
   0.2.0 and its source commit. An authorized maintainer can ask for this material;
   no message has been sent by this audit.
2. Evaluate a deliberate, tested upstream dependency migration that removes the
   older dispatch crate while preserving main-thread execution, Winit/Slint
   integration and accessibility. No migration, framework substitution or local
   fork was implemented or tested in this review.

If authoritative notice bytes become available, retain the unmodified file,
immutable upstream commit/path or signed release association, acquisition date,
byte length and SHA-256. Bind that evidence to the existing crate archive checksum
and `.cargo_vcs_info.json` revision in `third_party/notices/manifest.json`, and test
the collector's mismatch rejection before changing the gap status. A web-rendered
excerpt is not suitable input for an invented file hash.

### Direct primary-source follow-up, 2026-09-29

The official cached crate archive is **10,229 bytes**, with the exact lockfile
SHA-256
`bd0c93bb4b0c6d9b77f4435b0ae98c24d17f1c45b2ff844c6151a07256ca923b`.
Its `.cargo_vcs_info.json` names the locked revision above and hashes to the
existing notice manifest's
`313a7abcf98d6834fdf27109ae9bf62f318efe8e4c6ac27b41fbeb2e4ff6a1f8`.
The 12 archived files contain neither a named license file nor a copyright
marker/standard MIT-grant opening in the inspected text. Phrase absence is a
source-inspection observation, not a legal conclusion about the MIT declaration.

GitHub's [locked recursive tree](https://api.github.com/repos/SSheldon/rust-dispatch/git/trees/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748?recursive=1)
is untruncated and contains 17 entries without a license/copying file. The
default branch remains `master`; its observed current commit is
[`f540a2d8ccaebf0e87f5805033b9e287e8d01ba5`](https://github.com/SSheldon/rust-dispatch/commit/f540a2d8ccaebf0e87f5805033b9e287e8d01ba5),
with root tree `6bd26acc457aa3054b5608abc4e685fd614b916a`. That untruncated
tree has the same 17 paths and likewise supplies no named license file.
GitHub's repository metadata detects no license; this does not override the
crate's MIT manifest declaration.

The [issue API](https://api.github.com/repos/SSheldon/rust-dispatch/issues/18)
reports **zero comments**, state `closed`, reason `completed`, and closure on
**2024-05-16 at 13:38:02 UTC by Jake-Shadle, the original reporter**. The
[public timeline](https://api.github.com/repos/SSheldon/rust-dispatch/issues/18/timeline?per_page=100)
has two events, with no additional pagination: an earlier cross-reference and
that closure. Neither supplies notice text or a resolving-commit field. The
state label and who clicked Close establish no replacement grant or its
applicability to 0.2.0; no unrecorded author clarification is inferred.

The [bounded evidence record](evidence/2026-09-29-dispatch-notice-followup.json)
retains request URLs/status/body hashes, both complete tree listings, crate
member hashes, the source association and selected issue metadata. Its SHA-256
is `b035d881032fd2d60e5106f5ff6857308efbb3c99e79701fc6763572a11a0947`.
Raw responses remain local; irrelevant public profile data and local filesystem
paths are omitted from the shareable record.

This pass closes the earlier API-access uncertainty, **not the missing original
notice**. No verified notice file can be added to the collector from these
materials. The next step remains an explicit upstream grant/attribution
clarification applicable to the locked release, or a separately reviewed
dependency migration preserving the required integration. No contact or version
change was performed, and `licensing.md`/the gap manifest are unchanged.

### Actual macOS dependency and the available upstream migration

The existing `bc2e53f` packaged `BuildInfo/cargo-graph.json`, current lockfile and
local release fingerprints distinguish a real macOS dependency from an
all-target inventory entry:

```text
oxplay -> slint 1.19.0 -> i-slint-backend-winit 1.19.0
       -> winit 0.30.13 -> objc2-foundation 0.2.2 [dispatch]
       -> dispatch 0.2.0
```

The same Foundation package is reachable through `accesskit_winit 0.33.2 ->
accesskit_macos 0.26.3` and through `objc2-app-kit 0.2.2`. Those graph paths do
not by themselves identify the feature activator: AccessKit's manifest requests
`NSArray`, `NSDictionary`, `NSValue` and `NSThread`, **not `dispatch`**. Winit's
macOS Foundation dependency explicitly requests it. Foundation's default feature
is `std`; `dispatch` enables its optional crate dependency. The existing release
Foundation fingerprint contains that feature and dependency, and a release
`dispatch` depfile exists. This supports actual compilation; it is not a count
of retained final-link symbols or proof that every queue branch ran.

At pinned Slint revision `cf3b07d4917e6759a63b0c03913a2594ec653414`,
`internal/backends/winit/Cargo.toml:98` requires Winit `^0.30.2`.
The selected AccessKit adapter requires `^0.30.5`, and glutin-winit 0.5.0 requires
`^0.30.0`. A small official [registry metadata](https://crates.io/api/v1/crates/winit)
read on 2026-09-29 still reports **0.30.13 as latest stable**, with
0.31.0-beta.3 as the newer prerelease. There is no newer published compatible
0.30 patch in that response to remove this dependency.

Official Winit commit
[`8b5f46d4be7d29f5b0ed91f4343b78d8c2668bd0`](https://github.com/rust-windowing/winit/commit/8b5f46d4be7d29f5b0ed91f4343b78d8c2668bd0)
uses `dispatch2` in its new
[AppKit backend](https://github.com/rust-windowing/winit/blob/8b5f46d4be7d29f5b0ed91f4343b78d8c2668bd0/winit-appkit/Cargo.toml#L22).
Its [workspace](https://github.com/rust-windowing/winit/blob/8b5f46d4be7d29f5b0ed91f4343b78d8c2668bd0/Cargo.toml)
is 0.31.0-beta.3 with Foundation 0.3.2, outside all three constraints above.
This is a concrete maintained upstream direction, **not a compatible lockfile
update for the pinned Slint stack**. A deliberate coherent Slint/integration
update or an accepted compatible upstream backport would need review and native
validation. Neither exists as a tested project change here. Adding dispatch2
directly does not remove dispatch, and renaming it does not supply the old
`dispatch::Queue` API. Its existing objc2-family notice review also remains.

No dependency, framework feature or accessibility setting changed. The practical
near-term release blocker remains the original notice/applicability decision;
the migration route is recorded without silently substituting an unofficial
fork or asserting new license rights.

## objc2 family: policy links are not complete attribution evidence

The retained policy at exact revision
[`8852b424193ca41602281b3d7540d7c8ed51e49a`](https://raw.githubusercontent.com/madsmtm/objc2/8852b424193ca41602281b3d7540d7c8ed51e49a/LICENSE.md)
distinguishes MIT-only core crates from crates offering a choice among three
licenses. It also flags SDK-derived bindings as a separate question. The current
collection retains ten such policy-only records across the newer objc2/block2,
dispatch2 and framework-binding packages, with their individual exact revisions
in the evidence manifest. That explains the package declarations but does not
turn linked generic license pages into original per-package notices.

Upstream [issue 23](https://github.com/madsmtm/objc2/issues/23) remains open and
discusses relicensing and contributor permissions. It is not evidence that the
requested new grants have already been obtained. Older complete MIT notices must
remain associated with their audited releases; neither a desired relicensing nor
a sibling crate's alternative grant silently changes the selected dependency.

The SDK concern requires more than adding a missing text file. The primary
[Xcode and Apple SDKs Agreement](https://www.apple.com/legal/sla/docs/xcode.pdf)
read in this pass identifies itself as EA2002, dated `06/08/2026`. Section 2.4
distinguishes macOS application/library distribution from the separate program
agreement for the App Store and other platforms. Sections 2.5 and 2.7 address
copying and redistribution of Apple software itself. Those distinctions warrant
review of the actual generated bindings and distributed files; they establish
neither blanket permission nor blanket prohibition for this application.
The remotely served agreement has not been associated with the precise SDK
license accepted by this build host.

Locally executable preparation for that review is to record the actual compiler
and SDK versions/terms, identify which selected generated Rust files derive from
which SDK headers, preserve original notices where present, and distinguish
application/crate source from system frameworks that are linked but not copied
into the bundle. Obtain complete authoritative notices for the selected grants
and a recorded distribution decision for the exact files. Do not put the full
Apple SDK into a corresponding-source archive merely because a crate references
it; the source-coverage report intentionally does not promise SDK redistribution.

These attribution and SDK-policy questions remain open alongside dependency
corresponding source, Deno embedded-component inventory and clean-machine
packaging. Successfully reading their primary references does not close AC-23.

## Deno: top-level notice is present; embedded inventory is not

The installed Deno 2.9.7 keg has a complete 1,074-byte `LICENSE.md`, matching the
scope of its [upstream MIT notice](https://github.com/denoland/deno/blob/v2.9.7/LICENSE.md).
The packaging collector copies the keg's top-level notices, recipe and receipt.
Its 14,178-byte Homebrew SPDX document has **14 package records, one file record
and 13 relationships**, with `NOASSERTION` license declarations: Deno appears
twice, alongside Clang and 11 native runtime dependencies. That document is not
an inventory of the embedded Rust, V8 or JavaScript components. A nonempty
`notices` field and an empty top-level `native_notice_gaps` list cannot close this
separate omission.

The inspected installed formula specifies the official `v2.9.7/deno_src.tar.gz`
SHA-256 `21069d2f4dd65b6832e3f5c373c24a43a8d35cb3d68d3841e15d0582bed39ea8`.
It builds CLI with `--no-default-features` and `deno_core/v8,v8/v8`, selects
system libffi, little-cms2 and SQLite, and supplies LLVM/Python/Ninja/GN settings.
The tagged [CLI manifest](https://github.com/denoland/deno/blob/v2.9.7/cli/Cargo.toml)
distinguishes those features from default runtime features. A default-feature or
all-target Cargo graph would therefore be an inaccurate substitute for the
selected Homebrew build. The existing bounded source-coverage report records
that exact release source archive as absent at its audit time; this small-read
follow-up did not download or inspect it.

The next useful collection is the checksum-verified release archive, its exact
lock/manifests and generated source inputs, then the macOS feature-selected
dependency/notice graph and V8/native/embedded-JavaScript source associations.
Bind the result to the preserved Homebrew recipe, receipt and helper binary
hash; retain any unprovable bottle build inputs as gaps. Do not infer complete
embedded closure from Deno's MIT root notice, Mach-O load commands, or the
application's unrelated Cargo graph. No helper was launched, archive fetched,
build reproduced or redistribution approval claimed in this follow-up.

A [bounded offline first-stage collector](deno-source-inventory.md) was run after
the hold against the exact checksum-verified release archive. It records 1,128
unselected lock candidates and 66 notice-named files, including 61 test-tree
files and one copyright-checking program. Its report preserves original bytes
without treating every filename match as an applicable grant. The selected
feature graph, embedded component closure and binary/source association remain
unverified; the completed first-stage inventory does not clear distribution.
