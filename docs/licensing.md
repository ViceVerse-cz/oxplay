# License route and distribution audit

Recorded 2026-09-29. New application code uses **GPL-3.0-or-later**, following
the implementation request and SPEC's proposed open-source route. [LICENSE](../LICENSE)
contains the complete GPL version 3 text, copied without modification from the
reviewed Slint checkout's `LICENSES/GPL-3.0-only.txt`. The application grant is
version 3 or later; the license text itself is the standard version 3 text.
Existing specification ownership and third-party files retain their own terms.
This record does not relicense Slint or any other dependency.

## Slint decision

The selected framework option is **GPL-3.0-only** at upstream revision
`cf3b07d4917e6759a63b0c03913a2594ec653414`. The fetched `LICENSE.md`, framework
manifest SPDX header, and `LICENSES/GPL-3.0-only.txt` were inspected. Framework
source offers `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR
LicenseRef-Slint-Software-3.0`. Neither alternate route is selected here.
Examples and documentation use MIT; that does not make the linked framework
MIT. File-level headers and REUSE metadata remain authoritative. See the
[exact upstream license declaration](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/LICENSE.md)
and [runtime manifest](https://github.com/slint-ui/slint/blob/cf3b07d4917e6759a63b0c03913a2594ec653414/api/rs/slint/Cargo.toml).

The intended combined-distribution baseline is GPL version 3, respecting the
Slint GPL-3.0-only component. Marking original application files “or later” does
not grant later-version rights to Slint or establish those rights for a combined
binary. Final distribution review must resolve the actual linked dependency
graph, preserve applicable notices, and supply corresponding source/build
material as required. No binary package is cleared for publication by this
document alone.

The [manual source-preview workflow](ci-release.md) archives this application's
tracked source and existing notices only. It publishes no compiled application,
media or helper binary and makes no complete dependency-source coverage claim.
The developer bundle and its outstanding audit findings remain outside that
workflow's release assets.

The local storage crate adds `rusqlite 0.40.2` and `libsqlite3-sys 0.38.2`, both
MIT according to their fetched package manifests. The explicitly selected
`bundled` feature compiles SQLite `3.53.2` (observed in the bundled
`sqlite3.c`/`sqlite3.h`). SQLite's own source notice applies separately from the
Rust wrapper; preserve its public-domain declaration and review the complete
bundle. The `backup` feature is enabled; extension loading is not requested.
The protected-session module adds `chacha20poly1305 0.10.1`, `zeroize 1.9.0`,
`getrandom 0.4.3`, and macOS `security-framework 3.7.0` /
`security-framework-sys 2.17.0`. Their inspected manifests declare
`Apache-2.0 OR MIT` or the equivalent reversed expression. Preserve their license
texts and audit transitive cryptography/CoreFoundation dependencies in the SBOM.
The shared account UI uses `rfd 0.17.2` (MIT) for native file selection and
`webbrowser 1.2.4` (MIT OR Apache-2.0) to open the system browser. The account
transport uses `httpdate 1.0.3` (MIT OR Apache-2.0) for HTTP Retry-After dates.
These expressions were checked in the fetched package manifests. File dialogs
and browser launching do not introduce an embedded browser/application UI;
platform-specific transitive crates still belong in the distribution inventory.
The application integration status belongs in progress.md; a working storage
crate alone does not prove the shared local-library or account acceptance gates.

## Actual development media/helper build

These are locally installed Homebrew development dependencies, not a vetted
redistributable app bundle. Observed with `brew list --versions`,
`brew info --json=v2`, installed formulae/metadata, `mpv --version`,
`ffmpeg -buildconf`, and `otool -L` on libmpv.

| Component | Observed build | License/build evidence |
|---|---|---|
| mpv/libmpv | Homebrew `0.41.0_10`, runtime `v0.41.0` | Installed Copyright: GPL-2.0-or-later default; mixed LGPL-2.1-or-later sources. Installed formula does not request `-Dgpl=false` |
| FFmpeg | `9.0.2`, libavcodec `63.1.102` | Actual configuration includes `--enable-gpl --enable-version3`; Homebrew metadata reports GPL-3.0-or-later |
| yt-dlp | Homebrew `2026.8.19_1` | Source distribution Unlicense; Python virtualenv includes additional separately licensed packages |
| yt-dlp-ejs | Installed Python package `0.8.0` | Installed metadata: `Unlicense AND MIT AND ISC` |
| Deno | Homebrew `2.9.7` | Formula license MIT; embedded/runtime dependencies require their own inventory |
| Python | Homebrew `3.14.7` | Formula license Python-2.0; bundled third-party notices must be retained |

The build requires libmpv client API 2.5 or newer (`mpv.pc` reports 2.5.0 for the
observed mpv 0.41.0 package). That build requirement is separate from the
dependency's licensing and does not select an LGPL variant.

The installed mpv formula enables libmpv, JavaScript, LuaJIT, libarchive, uchardet
and Vulkan. The installed formula applies two upstream patches: Vapoursynth 74+
support (`75b2ccfeb1ce4ed5a40ac9860fa74f3d1265e13f`) and a macOS audio-device-change
use-after-free fix (`c5d391adba7bd024954d0df1e0405f5749f4d4ca`). These belong in
corresponding-source materials if that build is redistributed.

libmpv's observed non-system dynamic dependencies include FFmpeg, libass,
libplacebo, mujs, little-cms2, libarchive, libbluray, LuaJIT, rubberband, uchardet,
zimg, jpeg-turbo and vulkan-loader. The Homebrew formula also depends on
MoltenVK and Vapoursynth. A formula dependency is not by itself proof a library
is in the final linked bundle. Audit transitive dylibs and packaged helpers,
including disabled runtime features that remain linked.

FFmpeg's actual build additionally enables shared libraries, pthreads, ffplay,
SVT-AV1, Opus, x264, LAME, dav1d, VMAF, VPX, x265, OpenSSL, VideoToolbox,
AudioToolbox and NEON. It is not an LGPL-only development build. The
[FFmpeg legal page](https://ffmpeg.org/legal.html) explains why enabled components
affect the license; the [mpv Copyright file](https://github.com/mpv-player/mpv/blob/v0.41.0/Copyright)
likewise requires reviewing linked libraries even for an LGPL configuration.

The yt-dlp virtualenv includes mutagen `1.48.1` (GPL-2.0-or-later), curl-cffi
`0.16.2` (MIT), requests `2.34.2` (Apache-2.0), urllib3 `2.7.0` (MIT), websockets
`17.1` and idna `3.19` (BSD-3-Clause), brotli `1.2.0` and charset-normalizer
`3.5.1` (MIT), and pycryptodomex `3.23.0` (metadata says BSD/Public Domain;
individual license files still need review). certifi, cffi and pycparser are
external Homebrew dependencies. This is not an assertion that the whole helper
environment is Unlicense. Upstream also distinguishes source distributions
from its bundled executables in its
[release licensing notes](https://github.com/yt-dlp/yt-dlp#licensing).

Source archive SHA-256 values recorded from installed formula metadata:

| Source | SHA-256 |
|---|---|
| mpv v0.41.0 | `ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209` |
| FFmpeg 9.0.2 | `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e` |
| yt-dlp 2026.8.19 | `9e213e48cea35c66b378e4447903f118f6392a5fa380a2b6d7070ec86f4e0af1` |
| yt-dlp-ejs 0.8.0 | `d5fa1639f63b5c4af8d932495f60689d5370f1a095782c944f7f62a303eb104e` |

These identify formula sources; they are not hashes of installed dylibs or
executables. Installed Homebrew kegs contain `sbom.spdx.json` and
`INSTALL_RECEIPT.json`; collect them with binary hashes for a package-specific
inventory. Homebrew SBOMs do not replace a complete application SBOM.

## Before distributing binaries

- Audit the locked Cargo graph and selected features, especially Slint core,
  compiler, Winit, FemtoVG, text/accessibility and native transitive libraries.
  Build tools and runtime dependencies must be distinguished. Record SPDX
  expressions and license-file provenance; do not infer licenses from crate
  names or assume dynamic linking removes obligations.
- Generate a complete package SBOM and third-party notices; include every
  shipped helper/runtime/script, library, font, icon and test asset. The full
  transitive audit and release SBOM are not complete yet.
- Preserve notices and supply the applicable complete license texts and
  corresponding source, patches and build instructions for the distributed
  versions. Review GPL obligations for the particular distribution method,
  including any installation-information requirements that apply.
- Use only verified helper artifacts. Record hashes, provenance and an explicit
  update trust root, compatibility test and rollback process. No application
  auto-download/execute mechanism is approved by this development inventory.
- Review YouTube/platform terms, account risk, trademark/branding, codec/patent
  and signing/notarization obligations for the distribution jurisdictions.
  GPL source availability is not platform approval or a patent assurance.

No proprietary YouTube logo, screenshot, remote font or copied application
chrome is licensed by this project. Use system fonts and retain the applicable third-party asset licenses.
UI icons are unmodified upstream Lucide SVGs, pinned to revision
`66d8f9fc394b8530377e5f6112f0b8908ba01280` from the official repository.
`crates/app/ui/icons/LICENSE-LUCIDE` retains the full ISC and Feather-derived
MIT notices; `SOURCE.md` and `SHA256SUMS` record provenance and exact bytes.
The earlier project-created icons are no longer used. The generated/local test media must have an
explicit provenance record before being included in packages.
The current `scripts/generate-fixture.sh` generates a 90-second 1080p60 test
pattern and sine tone with FFmpeg's lavfi sources, plus original subtitle text;
it downloads no third-party footage. These generated files live in ignored
`artifacts/` and are not a licensed third-party video asset bundled by this tree.

## Developer bundle audit, 2026-09-29

The offline [macOS developer packaging tool](../scripts/package_macos.py)
records the selected Cargo normal/build graph, actual Mach-O load closure,
installed Homebrew source formulae/receipts/SBOMs, helper/Python fingerprints,
license texts, application source snapshot and final artifact hashes. See
[packaging.md](packaging.md) for the exact scope and commands. It performs no
publication, Developer ID signing, notarization or automatic dependency update.

An executed developer bundle contained 49 Mach-O inputs (the app and 48 copied
native libraries), all with arm64 support. Its greatest recorded minimum OS
was macOS 27.0. Relocation closure inspection, ad-hoc signature verification
and Info.plist syntax validation passed. These checks establish neither clean-
machine portability nor functional account/media acceptance. The installed
absolute-path yt-dlp/Python environment and Deno remained external dependencies.

The selected application graph contained 333 Rust packages, including build
inputs, with no selected Skia or Slint software renderer. The initial audit
identified 26 selected Rust packages missing original notices in their Cargo
archives. The explicit source audit recovered 41 original upstream files for
25 packages at 13 exact revisions from `.cargo_vcs_info.json`; see
[the versioned evidence](../third_party/notices/README.md). The offline collector
verifies the crate/revision association and file hashes before using them.

Fifteen packages supply full original grant/attribution texts. Ten newer
objc2-family packages instead supply an explanatory policy document with links
and an explicit Apple SDK-derived bindings review concern. This is recorded as
an unresolved review item, not interpreted as a finding that distribution is
permitted or prohibited. See the
[exact upstream objc2 policy](https://raw.githubusercontent.com/madsmtm/objc2/8852b424193ca41602281b3d7540d7c8ed51e49a/LICENSE.md).
`dispatch 0.2.0` has a
[MIT manifest declaration](https://raw.githubusercontent.com/SSheldon/rust-dispatch/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/Cargo.toml)
but its inspected exact tree contains no original license/notice file. That
gap remains explicit; no copyright name or generic replacement notice was
fabricated. A follow-up on 2026-09-29 found no original notice at verified
upstream master `f540a2d8ccaebf0e87f5805033b9e287e8d01ba5` either. Upstream's
[missing MIT text issue](https://github.com/SSheldon/rust-dispatch/issues/18)
has no comments providing the notice. A maintainer clarification or a reviewed
dependency replacement remains needed; no upstream message was sent.
Complete attribution, dependency corresponding source and precise
distribution review remain necessary. Installed glib and libunibreak notices
use `LGPL-2.1-or-later.txt` and `LICENCE`; the collector preserves those spellings.

The media-network addition was audited separately on 2026-09-29. The current
macOS application normal/build graph has **334** packages versus 333 in the
`6ccd0e8` helper bundle: the sole addition is workspace crate `serein-network`
under the application's GPL-3.0-or-later declaration. No new third-party package
is selected for this macOS artifact; reqwest, Tokio, URL and libc were already
in its graph. This was checked using locked, offline Cargo metadata/tree for
`aarch64-apple-darwin`, not by treating every lockfile entry as shipped code.

The lock update also introduced fourteen registry packages for the initially
evaluated Hickory path and its other-target dependencies. Native macOS DNS now
uses libSystem; these additions are absent from the selected macOS graph.
Their package declarations and original notice-file availability were checked:

| Locked package | Declared license | Original files present |
| --- | --- | --- |
| crossbeam-epoch 0.9.21 | MIT OR Apache-2.0 | LICENSE-MIT, LICENSE-APACHE |
| data-encoding 2.11.1 | MIT | LICENSE |
| hickory-net, hickory-proto, hickory-resolver 0.26.3 | MIT OR Apache-2.0 | LICENSE-MIT, LICENSE-APACHE in each package |
| ipconfig 0.3.4 | MIT/Apache-2.0 | LICENSE-MIT, LICENSE-APACHE |
| moka 0.12.16 | (MIT OR Apache-2.0) AND Apache-2.0 | LICENSE-MIT, LICENSE-APACHE, NOTICE |
| prefix-trie 0.8.4 | MIT OR Apache-2.0 | LICENSE-MIT, LICENSE-APACHE |
| resolv-conf 0.7.6 | MIT OR Apache-2.0 | LICENSE-MIT, LICENSE-APACHE |
| system-configuration 0.7.0, system-configuration-sys 0.6.0 | MIT OR Apache-2.0 | LICENSE-MIT, LICENSE-APACHE in each package |
| tagptr 0.2.0 | MIT/Apache-2.0 | LICENSE-MIT, LICENSE-APACHE |
| widestring 1.2.1 | MIT OR Apache-2.0 | LICENSE-MIT, LICENSE-APACHE |
| windows-registry 0.6.1 | MIT OR Apache-2.0 | license-mit, license-apache-2.0 |

Eleven packages were inspected in the installed Cargo registry sources. The
three Windows-only packages were inspected directly from their official
[ipconfig archive](https://static.crates.io/crates/ipconfig/ipconfig-0.3.4.crate),
[widestring archive](https://static.crates.io/crates/widestring/widestring-1.2.1.crate)
and [windows-registry archive](https://static.crates.io/crates/windows-registry/windows-registry-0.6.1.crate),
each verified against its exact `Cargo.lock` SHA-256 before reading. No code from
these archives was executed. The existing collector recognizes all these
notice filenames, including Moka's additional NOTICE; no supplemental notice
substitution was needed. These checks do not qualify another OS, provide all
corresponding sources, or resolve the earlier dispatch/objc2 review items.

The tool includes the vendored Lucide license, source revision and checksums
outside the source archive as well. It validates every recorded icon/license
hash and includes the icon dependency in the SPDX inventory. Upstream ISC and
retained Feather MIT notices remain applicable to these assets.

The optional `--bundle-helpers` route now includes the inspected Homebrew Python,
yt-dlp/EJS and Deno builds, their native dependency closure, and the selected
Python distributions' original metadata/license files. It preserves original
resource hashes separately from rewritten/signed Mach-O bytes. Homebrew's
stripped wheel RECORD files are handled with explicit package-root inventories;
empty earlier metadata lists are not evidence of complete helper source files.
The exec-only native launcher is original GPL-3.0-or-later application code.

The bundled Python trust file is the original Mozilla CA source from the
installed `ca-certificates 2026-09-25` keg, checked against the formula SHA-256
`a41b5d356aea97a529fe27e0f7316d2f9d946d75927476cf9cf1b90637d00505`.
The formula declares MPL-2.0. The host-generated Keychain-merged certificate
bundle is deliberately excluded. The installed source formula, receipt and
available notices are included for this additional input as well. This does
not resolve the outstanding full corresponding-source, bundled native component
attribution, patent, SDK-binding or distribution review. See
[packaging evidence and limits](packaging.md) for the actual executed slice.
The additional SQLite keg lacks a top-level license file; its original installed
`sqlite3.h`, containing the upstream copyright disclaimer/blessing, is preserved
as source notice evidence.

The CA keg's missing standalone license text now has verified supplemental
evidence in [third_party/ca-certificates](../third_party/ca-certificates/README.md).
The retained original Mozilla `certdata.txt` hashes to
`beb7e6dfe6499926e52c075c27bcfbe4c957f8609c575b3860273ae2806f63eb`, exactly the
source hash recorded in the installed PEM header. It comes from verified Firefox
revision `9afcd4459e1f26aab45882f7d9df0f0c1e1b23f4` and includes the original MPL
notice. The complete, unmodified MPL-2.0 text comes directly from Mozilla.
The offline collector binds both files to the exact CA version, bundle hash and
source-hash header before copying them. Its actual-input copy check passed;
eleven packaging tests, including new mismatch/tampering cases, passed alongside
thirteen verifier tests. Existing older bundle artifacts have not been rewritten.
This closes that specific missing-text/source-evidence item, while PEM conversion
reproduction and the broader distribution review remain outstanding.


The first-party `serein-dns` helper is built from `crates/network` under the
workspace's GPL-3.0-or-later declaration. The package source archive includes its
supervisor, protocol and native adapter. Its native closure and both executable
Cargo build roots are included in the packaging inventory; final signed helper
bytes have their own inventory hash. This adds no new third-party dependency or
framework license route and does not resolve the outstanding distribution review.


## Offline source-availability audit

The report-first [source coverage collector](source-coverage.md) now checks
existing package evidence against local Cargo and Homebrew source caches without
downloads, code execution or source-tree copying. Auditing the unchanged
`bc2e53f` developer artifact found 314 matching registry archives, 14 matching
Git checkout HEADs with unverified worktrees and six archived workspace crates.
The 51 native recipes yielded 70 recognized source/resource/patch inputs whose
matching payloads were absent from the selected cache, including both exact mpv
patches. Those absences are explicit; installed binary bottles were not accepted
as source. See the [hashed report and scope](evidence/2026-09-29-source-coverage-summary.json).

This narrows the local source-collection work but does not complete corresponding
source or the distribution review. In particular, Git worktree content,
conditional/computed native recipe inputs, Deno's embedded components and build
reproduction remain unresolved. The existing dispatch notice gap and objc2/SDK
review items remain unchanged. Future application source snapshots include the
unchanged SPEC and standalone baseline tool; earlier artifacts are preserved and
have not been relabeled as containing those additions.
