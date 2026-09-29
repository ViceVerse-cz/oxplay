# Deno source/notice inventory first stage

`scripts/deno_source_inventory.py` is an offline collector for the exact Deno
2.9.7 source archive specified by the inspected Homebrew recipe. It was authored
during an exclusive native measurement hold. After the hold, its synthetic tests
passed in the centrally run 156-test Python suite, and the real archive scan
completed successfully on 2026-09-29. No source extraction or execution occurred.

The actual invocation used a new private artifacts directory:

```sh
python3 -m unittest discover -s scripts -p 'test_deno_source_inventory.py'
python3 scripts/deno_source_inventory.py \
  --archive artifacts/deno-source-20260929/deno_src.tar.gz \
  --output artifacts/deno-source-20260929/inventory
```

The CLI accepts only source SHA-256
`21069d2f4dd65b6832e3f5c373c24a43a8d35cb3d68d3841e15d0582bed39ea8`,
bound to the official `v2.9.7/deno_src.tar.gz` recipe URL. It does not fetch input,
run Cargo, execute package code or interpret the Ruby formula. It records the
reviewed macOS build request: CLI with default features disabled and
`deno_core/v8,v8/v8`. This recorded request is not evidence that the inspected
lockfile has been resolved under those features.

The streaming scanner checks ordinary tar headers, bounded PAX/GNU name records,
member types, paths, duplicates and the terminator before accepting an inventory.
It retains hashes of Cargo manifests/lockfiles and exact bytes of named original
license/notice files. It never extracts a member to its archive pathname or
follows a link. Notice copies use SHA-256 filenames under a new private output
directory; the report supplies their source paths and sizes. Link members remain
explicitly uncollected. Unsupported sparse/extended archive transformations fail
closed. The exact upstream archive was accepted without a parser change.

The [sanitized result and hash provenance](evidence/2026-09-29-deno-source-inventory-summary.json)
record 34,255,299 compressed bytes, 114,636,800 expanded bytes, 22,005 members,
95 manifests/lockfiles, and nine unfollowed link members. The 1,128 unselected
lock candidates comprise 1,046 registry and 82 workspace/path entries. The full
local `inventory.json` is 413,884 bytes with SHA-256
`981c6d318af849a26f50da011db6c7ecda95fd329b494acddfd1417d85a1f8e7`.

The filename matcher retained 66 candidates with ten distinct byte payloads;
61 paths are under tests. One other match, `tools/copyright_checker.js`, is a
program rather than a standalone license grant. Thus the schema's
`original_notice_files` field is a **named-file candidate inventory**, not an
assertion that every match is an authoritative or applicable notice. The four
non-test license files are the root and WebGPU `LICENSE.md` files, the Node type
definitions' `LICENSE`, and their nested Undici `LICENSE`. Their exact hashes
and source paths are preserved in the summary. The root notice matches the
installed Deno 2.9.7 root notice; that comparison does not associate the binary
with the full source/build closure.

The download used a separate supervised process with a 180-second parent
deadline, a 1 GiB byte cap, and HTTPS redirects limited to GitHub and its release
asset host. It completed in 0.893 seconds and passed the full pinned checksum;
there were no failed fetch attempts. The offline scan returned exit zero.
Independent post-scan checks verified every retained payload's digest and size,
all null package-selection flags, the private 0700 output/0600 notice modes,
and removal of the incomplete marker. No Cargo command or native app ran.

Fixed limits are 1 GiB compressed, 4 GiB expanded, 100,000 headers, 512 MiB per
ordinary member, 64 KiB per extension, 8 MiB per retained member, 64 MiB retained
metadata, and 10,000 lock packages. Cooperative checks share a 120-second
verification/inspection budget; filesystem reads are not a hard real-time
cancellation guarantee. Input must remain quiescent. No source cache is mutated
and an existing output directory is never reused.

Every lock package has `selected_for_homebrew_build: null`; the report explicitly
marks the selected dependency graph absent. A lockfile includes optional, other
target and development packages and cannot establish the actual compiled
feature graph by itself. Root/nested notice bytes also do not establish which
files were compiled or which grant applies to each package. Source checksum
matching does not authenticate the publisher or prove the Homebrew bottle's
build correspondence.

Next stages remain separate: collect a reproducible macOS feature-selected graph
with source associations; inventory
V8, generated snapshots, embedded JavaScript and native build inputs; and bind
the result to the preserved recipe, build evidence and helper binary. No complete
corresponding-source, notice closure, reproducible-build or redistribution
approval claim follows from this first-stage report.
