# Bundled helper notices

Unmodified license texts for the helpers that Windows and Linux release packages
bundle at the versions pinned in `scripts/fetch_pinned.py`. Update these files
in the same commit as a pin change.

| File | Upstream source | SHA-256 |
| --- | --- | --- |
| `deno-LICENSE.md` | <https://github.com/denoland/deno/blob/v2.9.7/LICENSE.md> | `f62497fffecc0852960c8d3e6934b9db86d16396e9b604072e923892cae3a588` |
| `yt-dlp-LICENSE` | <https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/LICENSE> | `7e12e5df4bae12cb21581ba157ced20e1986a0508dd10d0e8a4ab9a4cf94e85c` |
| `yt-dlp-THIRD_PARTY_LICENSES.txt` | <https://github.com/yt-dlp/yt-dlp/blob/2026.08.19/THIRD_PARTY_LICENSES.txt> | `472aefe951c7db35e1657c1d13fd337140511ed6f2b329205105ad441c5a02b7` |

The last file covers the components inside yt-dlp's PyInstaller executables.
libmpv notices depend on the build: the Linux packages record the pinned mpv
source they compiled, and the Windows package names the pinned
[shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake)
archive. Complete corresponding-source coverage for those native builds is
still open; see `docs/licensing.md`.
