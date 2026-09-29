# Original upstream notice evidence

These files retain their upstream licenses and copyright notices. The project
license does not replace their terms. They are copies for review and packaging,
not newly authored or generic substitute licenses.

`manifest.json` associates each selected crate version with the exact revision
in its published `.cargo_vcs_info.json`, its declared repository and license,
the original file path/URL, the inspected Git tree/blob IDs, and SHA-256 of each
copied file. Source Git symlinks were resolved inside the same repository and
revision. Downloaded bytes were checked against the Git blob identifier before
storage. No revision was inferred from a version number or moving branch.

The explicit developer command `python3 scripts/fetch_notice_evidence.py`
created this evidence using read-only public upstream requests. It refuses to
overwrite existing evidence; deliberate refresh requires a new output directory
and a reviewed diff. Normal application execution and the offline bundle tool
never fetch these files. The bundle tool matches the evidence to current crate
VCS metadata and verifies every file hash before including it.

The initial audit covered 26 packages at 13 exact upstream revisions. It found
41 original files for 25 packages. Fifteen packages have original full license
text/attribution files; ten newer objc2-family packages instead provide an
upstream policy document linking to license terms and raising Apple SDK-derived
binding questions. Those ten are explicitly marked `review_required`; collecting
the document is not a legal conclusion or completion of attribution review.

`dispatch 0.2.0` remains unresolved: the inspected exact source tree has a MIT
license declaration in Cargo.toml but no original license/notice file. Its
manifest entry records the original tree URL and the gap. No copyright notice
has been invented or copied from a different version/project to hide it.
The 2026-09-29 follow-up also inspected current upstream master
`f540a2d8ccaebf0e87f5805033b9e287e8d01ba5`, which still has no original notice.
The upstream [missing MIT text issue](https://github.com/SSheldon/rust-dispatch/issues/18)
has no comments supplying it. This is evidence of the remaining gap, not a
substitute license grant. No maintainer contact was made.

This collection is not complete dependency corresponding source, not a patent
license inventory, and not permission to publish an application binary. The
selected-build SBOM and remaining package review belong in docs/licensing.md
and the developer bundle's build manifest.
