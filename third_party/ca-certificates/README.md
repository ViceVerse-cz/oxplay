# Mozilla CA source and license evidence

The `2026-09-25` directory preserves unmodified primary-source files associated
with the immutable public Mozilla CA bundle installed by Homebrew. It contains
no host Keychain material. `manifest.json` records the source URLs and hashes.

The installed formula pins the PEM bundle to SHA-256
`a41b5d356aea97a529fe27e0f7316d2f9d946d75927476cf9cf1b90637d00505` and
declares MPL-2.0. Its original header identifies the input `certdata.txt` with
SHA-256 `beb7e6dfe6499926e52c075c27bcfbe4c957f8609c575b3860273ae2806f63eb`.
The retained Mozilla file matches that hash exactly, including its original MPL
notice and certificate/trust data. It was fetched from Firefox revision
`9afcd4459e1f26aab45882f7d9df0f0c1e1b23f4`, resolved from the official `release`
branch on 2026-09-29. The MPL-2.0 text is the unmodified Mozilla-published text,
not a reconstructed notice or an invented copyright attribution.

The offline packager checks the installed bundle hash, the bundle's source-hash
header and both retained file hashes before copying this evidence into its
native dependency notices. A newer CA bundle needs its own reviewed evidence;
these files cannot silently satisfy an unrelated bundle version.

This closes the missing-text/source-evidence gap for this particular CA input.
The PEM conversion has not been reproduced, and these files do not establish
complete corresponding source or legal clearance for the entire application.
