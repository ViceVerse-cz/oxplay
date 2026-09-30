# Upstream icon provenance

All SVGs in this directory are unchanged files from the official [Lucide repository](https://github.com/lucide-icons/lucide), revision [`66d8f9fc394b8530377e5f6112f0b8908ba01280`](https://github.com/lucide-icons/lucide/tree/66d8f9fc394b8530377e5f6112f0b8908ba01280/icons). The repository's default branch was verified as `main` with `git ls-remote --symref`; the shallow checkout resolved the same exact revision.

Source path for each SVG: `icons/<same filename>` at that revision. No paths, strokes, geometry, or metadata were redrawn or modified. Slint applies runtime size and color only. The earlier project-created SVGs have been removed. The application badge uses the upstream `clapperboard` icon; it is not an original Serein icon or the Lucide project logo.

`LICENSE-LUCIDE` is the complete, unchanged upstream license: ISC copyright Lucide Icons and Contributors, with the retained Feather-derived MIT license and Cole Bemis attribution. Both notices must accompany redistributed assets. [Official license information](https://lucide.dev/license).

`SHA256SUMS` records every vendored SVG and the upstream license. Verify with `shasum -a 256 -c SHA256SUMS` from this directory. Deliberate updates must resolve a new official revision, recopy unchanged assets/license, regenerate the hashes, and validate the Slint build and visible icon rendering.

The mute-state action includes the unchanged `icons/volume-x.svg` from this same pinned checkout, alongside `volume-2.svg`. Its state reflects the observed native player property; it does not imply a persisted mute preference.

The picture-in-picture control uses the unchanged `icons/picture-in-picture-2.svg` from this same revision. Its checksum is included alongside the existing icons.

The retained `minus.svg`, `square.svg`, `copy.svg`, and `x.svg` also originate from this revision. Normal windows now use native system decorations. Theatre mode uses the unchanged `rectangle-horizontal.svg` from the same revision.

Comment refresh uses unchanged `refresh-cw.svg` from the same pinned official Lucide revision, with its checksum and license retained.

The guide's Explore shortcuts use unchanged `music.svg`, `gamepad-2.svg`, `newspaper.svg`, `trophy.svg` and `graduation-cap.svg`, and drop-down fields use unchanged `chevron-down.svg`, all copied byte-for-byte from `icons/` at this same pinned revision, with checksums added to `SHA256SUMS`.
