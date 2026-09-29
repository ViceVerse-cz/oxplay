# macOS developer packaging

The available target is Apple Silicon on macOS 27. This workflow creates a
reviewable **development bundle**, not a portable or release-qualified product.
No Developer ID identity, account credential, notarization service, publication
endpoint or automatic updater is used. Windows, Linux/X11 and native Wayland
packaging have not been validated.

## Run the offline tooling

Install the documented native prerequisites and build with the committed lock:

```sh
cargo build --release --locked -p serein -p serein-network --bins
python3 scripts/package_macos.py inspect --output artifacts/package-evidence
python3 scripts/package_macos.py bundle --output artifacts/Serein-Development.app
python3 scripts/package_macos.py bundle --bundle-helpers --output artifacts/Serein-WithHelpers.app
```

Both commands require a fresh output path and refuse overwrites. `inspect`
creates dependency evidence without changing an application executable. `bundle`
adds a standard `.app` layout, copies and relocates the non-system Mach-O load
closure into `Contents/Frameworks`, signs only those owned copies ad hoc, and
verifies the resulting signature. The original application and Homebrew files
are never rewritten. It does not download anything or silently resolve a new
Cargo lock. Offline Cargo metadata must already be available.

Use `--build` to make the tool build both `serein` and `serein-dns` in one
locked release invocation and select their executable paths from Cargo artifacts.
The first-party DNS helper is always included at `Contents/Helpers/serein-dns`,
independently of `--bundle-helpers`. Prebuilt mode requires both binaries;
`--dns-helper` selects an explicit DNS executable. Without `--build`, the manifest explicitly marks the association between the prebuilt binary
and the current source snapshot **unverified**. The binary hash, Git commit,
dirty-tree flag, source-tree fingerprint and deterministic source archive are
recorded separately. A source change during inventory or compilation aborts;
an input binary changing during copying also aborts. A newer dirty source
snapshot is never claimed to reproduce an older executable.

`SOURCE_DATE_EPOCH` can set the source-archive/SBOM timestamp; otherwise the
recorded Git commit timestamp is used. Sorting, source archive ownership and
archive timestamps are normalized. This supports repeatable inventory from
identical inputs; it is not yet evidence of independent bit-reproducible Rust,
Homebrew or signed-bundle builds. Record the toolchain and compare complete
output inventories before making such a claim.

## Transient DNS helper packaging

Current packaging includes the first-party `serein-dns` executable from
`serein-network`. Both binaries use the same source snapshot and lockfile. The
helper's actual Mach-O dependency graph contributes to the copied closure and
minimum macOS version; the packager does not assume it links only system
libraries. Relocation, individual ad-hoc signing, closure checks, final bundle
signature verification and file inventory cover this executable too. The
manifest binds its original native hash to its input path and identifies its
GPL-3.0-or-later workspace source; final signed bytes are covered separately by
the final inventory. The Cargo/license inventory includes both build roots.

A mandatory offline probe runs the bundled helper with empty stdin in a clean
environment and private temporary home. It requires exit code 2 and empty stdout
and stderr with a configured five-second subprocess timeout. Process creation,
OS scheduling, termination and reaping remain OS contracts rather than a proven
hard real-time bound. This exercises loading and rejection of malformed input
before a DNS request. It does **not** prove real system resolution,
cancellation, VPN/scoped resolver behavior, or clean-machine portability. Those
are separate network tests. No persistent resolver service is added.

Synthetic tests verify selection of exactly both Cargo binary artifacts,
missing/duplicate/wrong-kind rejection, the offline probe's bounded clean
environment, and first-party input/final-file inventory consistency. The frozen
`b1f4074` bundle below predates this helper and has not been rebuilt or relabeled;
the newer checkpoint has separately recorded offline and native functional
evidence below.

## Checkpoint `bc2e53f`: DNS helper and scoped playback bundle validation

The detached, clean checkout at `bc2e53f6a3eb620dad52d4934a43869aadba8a5a`
produced the new `artifacts/Serein-bc2e53f.app` using `--build --bundle-helpers`
and a shared Cargo target directory. Both executable paths came from the locked
build's Cargo artifacts. The app input SHA-256 was
`6791c634802d1649b5d4ff26a7c5cfd6d3e1c334527a527760fe26233edcf874`;
the DNS helper input was
`2b6215e3854806298e5be4f835354b034eae423e6d4a0b65eb8e44d0963c3cae`.
Both matched the main-checkout checkpoint build. This is a verified source/build
association and a matching rebuild on this host, not independent reproducibility.

The package contains 175 original Mach-O inputs plus the generated yt-dlp
launcher, 5,149 inventoried files totaling 324,873,582 bytes, and 226 archived
application source files. Its load-command minimum remains macOS 27.0. Actual
`otool -L` on bundled `serein-dns` reports only `libSystem.B.dylib` and
`libiconv.2.dylib` under `/usr/lib`. Python import/trust inspection, yt-dlp
version, Deno version, Deno script, and the empty-input DNS helper probe all
passed. Python reported 121 CA certificates and only the selected bundled
runtime paths. Deep strict ad-hoc signature verification and the read-only
whole-tree/source verifier passed. The inventory SHA-256 is
`534d10c28c011f82375c93ebd79429fc16003e3215ffbf5639ccfec9c7e6359e`.

[Sanitized evidence](evidence/2026-09-29-packaged-dns-helper-validation.json)
records original and signed binary hashes, probes, source association and limits.
A subsequent finite 40-second launch of this exact bundle used `--scoped-media`,
a fresh 0700 profile, an isolated home, `PATH=/usr/bin:/bin`, no helper overrides
and no credentials. Public Big Buck Bunny playback advanced to 37.23 seconds,
with VideoToolbox H.264 at 1920×1080/60 fps and Opus 48 kHz through avfoundation.
The [inspected screenshot](evidence/2026-09-29-packaged-scoped-media.png) shows
video composed inside the shared light-theme UI. Audio output was reported by
the engine; audible quality and A/V synchronization were not assessed.

Both scoped HTTP sources were released: 21 validated ranges totaling 22,020,096
bytes, with zero transport errors. Process sampling observed the bundled Python
runtime and an app-owned `(serein-dns)` child; the DNS child exited before its
full path was sampled. The fixed validated bundle path, absence of overrides,
and successful scoped requests jointly support helper discovery. This is not
whole-process egress verification. There were no surviving tracked processes
or caption files after exit. Loaded paths contained 49 bundle paths plus system
libraries and no external non-system library. Whole-bundle integrity verification
passed again unchanged after the native run.

[Sanitized diagnostics](evidence/2026-09-29-packaged-scoped-media.log) record zero
catalog changes/resets, seven presentation drops and zero decoder drops. This
functional run used timing diagnostics, frequent process sampling and a bounded
display-wake assertion; it is not a resource or frame-pacing qualification.
The older bundle's caption lifecycle is not automatically attributed to this
artifact. Dispatch's missing original notice and the ten objc2-related review
items remain open. No release/redistribution approval or clean-machine
portability is asserted. The older `b1f4074` artifact remains unchanged.

## What is captured

`Contents/Resources/BuildInfo` contains:

- The original application source archive and full application GPLv3 text.
- `build-manifest.json`, with original native file hashes, architectures,
  minimum-OS load commands, helper paths/hashes, source/build identity, missing
  notices and explicit limitations.
- `cargo-graph.json`, using the selected application's normal/build dependency
  graph. `cargo metadata` alone over-approximates inactive optional renderers;
  the tool intersects it with `cargo tree` for the actual macOS target. Build
  dependencies are distinguished in edges, and are not asserted to be shipped.
- An SPDX 2.3 development SBOM, inspected Rust license files, and Slint's license
  texts from the exact Cargo Git source. Slint's GPLv3 route remains selected;
  preserving upstream alternative text does not select those alternatives.
- Homebrew installed receipts, formula source including hashes/build options,
  upstream Homebrew SBOMs and available license notices for the actual dylib and
  helper kegs. Receipts redact the home-directory prefix; their original hashes
  identify the inspected originals separately from the sanitized copies.
- External Python distribution metadata and installed-file hashes, inspected
  with user-site imports disabled. The installed extractor exposes some global
  Homebrew Python packages; inventory inclusion means reachable from its Python
  environment, not proof each package executes during playback.

A neighboring `.inventory.json` records every finished output file's SHA-256,
including the final ad-hoc signature. Native SBOM input checksums identify
**pre-relocation** files, since changing load commands and signing changes bytes.
The final inventory records the transformed bytes. Neither document invents an
upstream source revision from a binary hash.

## Read-only artifact verification

`scripts/verify_package.py` verifies the current schema-1 development bundle and
its neighboring inventory without executing native tools, loading the app or
extracting archives:

```sh
python3 -B scripts/verify_package.py /path/to/Serein.app
python3 -B scripts/test_verify_package.py
```

An explicit `--inventory PATH` permits verification after relocating or renaming
the bundle. `--inventory-sha256 HEX` additionally checks an independently
obtained inventory digest. A digest shipped beside an attacker-modified bundle
is not publisher authentication; the tool always reports that authentication,
code-signature verification and portability qualification have not been done.
It does not replace Developer ID/notarization or the packager's native signature
checks, and it is not an installer or updater.

The verifier checks the exact regular-file set, sizes and final SHA-256 values,
manifest/inventory fingerprints, referenced evidence, the media trust resource
and unchanged non-native helper resources. Original native helper hashes remain
distinct from relocated/signed file hashes. It also reads the source tar without
extracting it, rejects unsafe/duplicate/link/special members, and recomputes the
application source fingerprint and archived `Cargo.lock` hash. JSON, path depth,
entry counts, file/tree sizes and archive sizes have explicit limits. Ambiguous
case/Unicode paths, duplicate JSON keys and nonfinite numbers are rejected.

Schema 1 records regular files, not link targets or permissions. Consequently
the verifier rejects **all symlinks and hard-linked files**, including internal
links, instead of following unrecorded targets. Descriptor-relative directory
traversal uses `O_NOFOLLOW`; FIFOs/devices are rejected before reads. Extra empty
directories and historical permission changes are not authenticated by this
schema. A second file metadata snapshot detects ordinary concurrent changes,
but verification expects a quiescent bundle and is not an atomic installation
defense against an adversary concurrently replacing the tree.

Thirteen synthetic tests passed, covering same-size tampering, missing/extra
files, supplied-digest mismatch, manifest/evidence/source mismatch, unsafe paths,
duplicate metadata, special files, internal/external/broken/directory symlinks,
hard links, source archive traversal/link entries, modified helper resources and
a file changed during verification. The eleven packaging tests also
passed. The existing `Serein-helper-offline-gate.app` passed the read-only audit:
5,144 files, 317,758,059 bytes, and 170 archived application source files. Its
inventory SHA-256 is
`f9e9744f12923a49f96d2005ae6f9c82b51d8c2a20aba09768b93ed1af508695`.
This result verifies that older artifact's recorded contents; its application
binary still predates the media TLS fix and remains unsuitable as evidence of
secure network playback. No native code or account operation ran during this
verification.

## Declared external dependencies and limits

An unpackaged development executable defaults to `/opt/homebrew/bin/yt-dlp` and
`/opt/homebrew/bin/deno` on macOS. An executable under an owning
`.app/Contents/MacOS` instead selects `Contents/Helpers/yt-dlp` and `deno`.
Missing bundled paths never silently fall back to Homebrew. Explicit absolute
`--yt-dlp` and `--deno` paths override the individual selections.
Media TLS uses `Contents/Resources/Certificates/mozilla.pem` in a bundle and
the immutable `ca-certificates` keg source in an unpackaged macOS build.
The expected CA resource is checked before UI startup: it must be a nonempty
regular file at an absolute UTF-8 path, and a bundled resource must resolve
inside that application's resource directory. Missing resources fail clearly;
they never silently select ambient host trust. The packager includes this
shared media CA resource even when Python/Deno helpers are left external.
The installed Homebrew yt-dlp launcher names its exact installed
Homebrew virtual-environment Python interpreter in an absolute shebang. That
venv enables global Homebrew site packages and contains `.pth` references to
other kegs. Copying the launcher or venv alone would not make it portable.
Without `--bundle-helpers`, the tool inventories those external inputs but does
not copy them; use explicit CLI paths to exercise that development bundle's
external helpers. With the flag, the tool copies the reviewed interpreter,
standard library, Python packages, Deno and native dependency closure described
below. It never downloads a new helper at runtime or during packaging.

## Bundled helper implementation

The selected route preserves installed Python 3.14.7, yt-dlp 2026.8.19,
yt-dlp-ejs 0.8.0 and Deno 2.9.7. The same-version
[official yt-dlp macOS release](https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19)
was evaluated, but its PyInstaller dependency build was not substituted for
these inspected inputs.

The app-owned, signed native launcher executes the bundled interpreter with
`-I -S -B`. It uses no shell or PATH lookup. Isolated mode excludes environment
and user import paths; `-S` suppresses site initialization and `.pth` execution;
`-B` prevents bytecode writes to the signed bundle. The bootstrap adds exactly
one package directory under `Contents/Resources/HelperRuntime`.
See the [Python 3.14 options](https://docs.python.org/3.14/using/cmdline.html).
The source and compiler identity of this small launcher are recorded separately.

Actual inspection found that Homebrew's `bin/python3.14` is a trampoline to
`Python.app`; the tool copies the real interpreter behind it. Homebrew also
removes wheel `RECORD` files. Empty metadata file lists are therefore not accepted
as runtime inventories: the tool enumerates bounded, explicitly reviewed package
roots and their distribution metadata, and records that inventory method.
It includes 14 distributions and excludes unrelated global pip, wheel and
VapourSynth installations. The earlier external-only inventory could contain
empty file lists; those artifacts do not establish helper file completeness.

Certifi's installed certificate symlink targets a host-generated bundle that
merges system Keychain trust. Packaging deliberately excludes that host trust
state. It copies the immutable Mozilla CA bundle from the installed
`ca-certificates` keg and checks its SHA-256 against the installed formula.
This means custom host/enterprise CA additions are not inherited by the bundled
Python helper. No credential or Keychain contents are scanned or exported.
The certificate input and this trust policy appear explicitly in the manifest.
The packager now includes supplemental original Mozilla `certdata.txt` and
MPL-2.0 text for the exact installed CA bundle. It verifies the bundle hash,
source hash in its PEM header, and both vendored file hashes before inclusion;
an unknown CA version remains an explicit notice gap. See the
[CA evidence](../third_party/ca-certificates/README.md). The actual-input notice
copy and synthetic mismatched-bundle, wrong-source-header and tampered-license
tests passed. Existing package artifacts predate this notice addition and remain
unchanged; their historical notice-gap records have not been relabeled.

Every stdlib extension, package native module, interpreter and Deno executable
gets a recursively inspected and relocated native load closure. Resource and
source-input hashes, distribution notices, the relevant Homebrew receipts and
source formulae are retained. The packager runs isolated offline imports,
certificate loading, helper-version and Deno JavaScript probes before publishing
the output directory. It rejects external Python paths and non-system dynamic
loads. `DENO_NO_UPDATE_CHECK=1` suppresses Deno's background update check; the
application supervisor enforces it for normal resolver children too.

The helper closure prototype was executed on the development host with a clean
`PATH=/usr/bin:/bin`, isolated temporary HOME/cache, and intentionally invalid
`PYTHONHOME`/`PYTHONPATH`. Python reported 3.14.7 and imported the exercised stdlib,
cryptography, compression, networking and extractor modules from bundle paths;
its complete reported import search path stayed within the bundle. yt-dlp
reported 2026.08.19. Deno reported 2.9.7 and executed an offline JavaScript probe.
Dynamic loader traces contained only bundle and system paths: 608 for the Python
probe and 549 for Deno. This fixes the earlier trampoline and missing package
inventory failures; failed prototype artifacts are not accepted bundles.

An explicitly guest-only extraction using the bundled helpers resolved
`aqz-KE-bpKQ`, the Blender Foundation Big Buck Bunny film, with genuine 635-second
metadata and HTTPS formats 299 (H.264 1920×1080 at 60 fps) and 251 (Opus).
No credential file was supplied, no account mutation occurred, and no signed
stream URLs or raw provider JSON were retained in the evidence. An older test
ID (`BaW_jenozKc`) correctly returned “video unavailable”; that failure was not
reported as a successful extraction. Evidence is under
`artifacts/helper-closure-validation/`. The helper prototype uses an older
preserved application binary. A later `Serein-helper-offline-gate.app`, generated
from frozen `6ccd0e8` production sources plus updated packaging scripts, passed
the mandatory offline packager probes, including CA parsing (121 roots), all
14 selected distributions and bundle-only import/native paths. The first gated
attempt correctly rejected an unsupported Deno `eval` option; the probe now uses
the inspected `run --ext=js ... --no-code-cache -` interface. That artifact's
helper probes were also repeated after moving the app to a different directory
and a filename containing spaces. The normalized results were identical and
strict ad-hoc signature verification still passed; the app was then restored
to its original artifact path. No public network was invoked by that relocation
test (`artifacts/helper-relocation-test/result.json`). That artifact's
application binary predates the explicit media TLS-verification fix and must
not be used as evidence of secure network playback. The later `b1f4074` bundle
run below exercised automatic helper discovery and playback using the corrected
TLS setting and packaged CA resource. These results do not establish
clean-machine qualification or end-to-end account behavior.

The installed libmpv `LC_BUILD_VERSION` requires macOS **27.0**, with SDK 27.0.
The tool derives the bundle minimum from the greatest minimum across all copied
Mach-O inputs; it never guesses an older supported OS. That declaration is a
loader constraint, not a successful runtime test on every eligible system.

A complete `otool -L` closure does not prove closure over `dlopen` plugins,
Vulkan ICDs, font configuration, locale data, certificates, FFmpeg model files,
or other resources. The selected application disables uncontrolled mpv scripts
and configuration, but native libraries still expose capabilities outside the
tested path. Local video, audio, subtitles, seek/resize, hardware-decoder state
and guest extraction must be tested in the bundle. Testing on this development
host still does not establish operation on a clean machine without Homebrew.

Some published Rust packages omit original notice files. The explicit
`scripts/fetch_notice_evidence.py` maintainer audit retrieves only text from the
exact official repository revisions declared by the installed crates; its
reviewable results live in `third_party/notices`. The offline packager verifies
that evidence against the selected crate metadata and file hashes. The audit
recovered upstream files for 25 of 26 gaps, while distinguishing ten policy-only
objc2-family documents from full grant texts; `dispatch 0.2.0` remains missing.
The tool reports missing files and unresolved policy reviews instead of
fabricating copyright notices.
The original application source archive is **not** the complete corresponding
source for Slint, FFmpeg, codecs or other dependencies. The package-specific
source/notice/legal audit, binary integrity trust root, signed update manifest,
rollback policy, Developer ID signing and notarization remain release work.
No distribution clearance follows from a successful developer bundle build.

## Evidence

The `b1f4074defcb36371c7bc8cd25966474d8e6d0d5` checkpoint was packaged with
`--build --bundle-helpers` from a clean, frozen source tree. The locked release
build succeeded inside the packager, so `source_build_performed` and
`prebuilt_source_association_verified` are both true. The original executable
SHA-256 is `62e90c91af4d382b1d6e3cf942660977e627f1f9a52d6274b0a0a595b71a6c8a`;
the relocated/ad-hoc-signed bundle executable is
`2f5ad3cd7ddae04755ee5577c207299b091b580627afa6a96f64ae79892e9386`.

This bundle passed all four mandatory offline helper probes and the packager's
strict ad-hoc signature verification. It includes 175 relocated native objects,
the exact CA source/MPL evidence, and no remaining native notice-file gaps.
The dispatch notice gap and ten objc2 policy reviews remain. The read-only
verifier passed for 5,148 files / 322,754,836 bytes and 212 archived application
source files. Inventory SHA-256:
`596c6b480142074cb5c9a358d6edb8dafcee46aafc75a170a555e7f9d05b4b1e`.
Evidence is in `artifacts/package-b1f4074-validation/bundle-result.json`.

The actual packaged app then passed its 70-second guest caption diagnostic with
exit 0, no helper-path overrides, `PATH=/usr/bin:/bin`, isolated HOME and a fresh
private data directory. It observed English selection, Off, cached reselection
and reattachment after a paused quality replacement to 720p. Initial playback
reported VideoToolbox H.264 1080p59.94 with Opus/avfoundation; the final paused
replacement reported 720p59.94, selected sid 1, two file loads and no media error.
Unrelated catalog changes and resets were both zero. The inspected screenshot
shows the caption selector and native confirmation without overflow; it does
not qualify subtitle glyph timing or composition.

No tracked child process, caption content file or instance directory remained
after shutdown. The expected empty 0600 `.registry-lock` remains in the private
cache root. The first harness incorrectly counted that lock as content and
expected DYLD diagnostics through the system `caffeinate` launcher, which
scrubbed those environment variables. Its failure and the corrected review are
retained separately. A direct-app loader probe initially failed display-clock
creation while the display was asleep (-6661); that run is also retained.
Repeating the 18-second probe with a bounded wake and a power assertion attached
to the app PID passed: 1,234 unique loaded paths, including 49 bundled native
files, with no external non-system loads and no remaining bundle processes.
The helper runtimes have their separate mandatory offline loader probes.

A final inventory verification confirmed the bundle remained unchanged after
these executions. This establishes functional development-host evidence for
the corrected direct TLS/CA path, helper discovery and caption lifecycle; it
does not establish clean-machine installation, visual subtitle timing, resource
budgets, human account behavior or whole-process egress protection. Negative
certificate tests are documented separately in [media TLS](media-tls.md).
See the [sanitized result](evidence/2026-09-29-packaged-caption-validation.json)
and `artifacts/package-b1f4074-validation/result.json` for the complete scope.

The first local `inspect` run on 2026-09-29 identified 49 Mach-O inputs totaling
about 82 MiB, with a greatest minimum macOS version of 27.0. It requested no downloads or account credentials. That first audit exposed inactive
optional renderer entries in Cargo metadata; the tool was corrected to use
Cargo tree before treating its graph as selected. Two subsequent developer bundle runs completed relocation, ad-hoc signing and
`codesign --verify --deep --strict`; `plutil -lint` also passed. The first bundle
contained 792 files totaling approximately 87.6 MiB, including dependency and
source evidence. Reinspection resolved every non-system Mach-O load inside the
bundle. The corrected Cargo graph contained 333 packages and excluded the
unselected Skia/Slint software renderers. Current source changes between those
two runs produced differing input fingerprints, so the runs are not a
same-input bit-reproducibility test. Native GUI execution and clean-machine
portability remain untested for those artifacts.

`python3 scripts/test_package_macos.py` passed six tests covering overwrite
refusal, dependency parsing/rpath failure, normalized system-path boundaries,
receipt redaction, and rejection of mismatched/tampered supplemental notices. Later collector changes preserve alternate native
license filenames and the vendored icon provenance explicitly; regenerate a
bundle from a stable checkpoint before distributing it for review.

### Frozen-checkpoint functional validation

A later package was produced from a clean, detached
`c77ebe91e8a268ec0a4d26150a76c3ace93f47b7` worktree, using the separately
preserved release executable with SHA-256
`6b3c97f2a2a610aab267d0bb6dce4b806b714b195efcc4c92e8627cd30637ede`.
The parent build recorded that source checkpoint; the packaging tool did not
rebuild it, so `prebuilt_source_association_verified` remains `false` rather
than treating a matching file hash as proof of source correspondence.
`artifacts/Serein-c77ebe91.app` contains 863 files totaling approximately
88.6 MiB, including the supplemental notices. Its inventory retains one
missing Rust notice (`dispatch 0.2.0`), ten upstream policy review items,
and no missing top-level native formula notice files. Presence of those
files does not resolve the outstanding redistribution review.

The relocated application ran from `/` with `PATH=/usr/bin:/bin`, an explicit
isolated data root, and cleared inherited `DYLD_*` settings. The local
22-second smoke exercise used the generated 1080p60 H.264/AAC clip, explicit
subtitles and `--no-ui-cache`. It exited successfully, including the paused
seek-to-20-seconds assertion and resize/fullscreen/minimize/restore sequence.
The engine reported `videotoolbox`, AAC through `avfoundation`, and no engine
error. Its dynamic loader trace contained 1,234 paths: 49 inside the app and
the remainder under system library/framework directories. No external
Homebrew dylib was loaded for this exercised path. The functional run had
77 reported dropped frames during deliberate transitions and a concurrent
build; it is not a performance measurement or an optimized-gate result.

A separate 17-second launch supplied an intentionally absent absolute
`--yt-dlp` path and a public video URL. The screenshot visibly displayed
“The configured yt-dlp helper is unavailable.” and the app exited successfully.
The failed executable spawn occurs before helper network activity. No live
account or credential import was involved. Logs, screenshot and a compact
result record are under `artifacts/package-c77ebe91-validation/`.
`codesign --verify --deep --strict`, `plutil -lint`, and the six packaging
regression tests passed. These checks establish this developer-host slice;
clean-machine operation, non-library resources, successful packaged extraction,
and portable helper distribution remain unqualified.

A second package from those same frozen inputs, placed under
`artifacts/package-repeat/Serein-c77ebe91.app`, produced the same input
fingerprint and identical SHA-256/size records for all 863 files, including
the ad-hoc signatures. `reproducibility.json` records the comparison. This
demonstrates repeatable packaging of these already-built inputs on this host;
it does not establish bit-reproducible compilation from dependency sources.
