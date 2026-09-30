#!/usr/bin/env bash
# CI/packaging helper: fetch one pinned, hash-verified libmpv development
# archive for x86_64-pc-windows-msvc builds and extract it into a new directory.
# Point MPV_DIR at the result. Nothing is installed system-wide.
#
# Source: shinchiro/mpv-winbuild-cmake (self-contained libmpv-2.dll with a
# statically linked FFmpeg, libmpv.dll.a import library and include/mpv).
# That project prunes old releases (about 30 retained), so this pin must be
# refreshed deliberately: update tag, file name and SHA-256 together and record
# the change in docs/build-inputs.md. Never replace the hash without review.
set -euo pipefail

tag=20260928
name=mpv-dev-x86_64-20260928-git-e470f8986e.7z
sha=81795d759e01016f1550fd71651a1a5d59ab5c28ef31c0b6793224e9cff39459

if [[ $# != 1 ]]; then
  echo 'Usage: install-mpv-windows.sh NEW_DIRECTORY' >&2
  exit 2
fi
destination=$1
if [[ -e $destination || -L $destination ]]; then
  echo "Refusing to reuse existing $destination" >&2
  exit 2
fi
for tool in curl sha256sum 7z; do
  command -v "$tool" >/dev/null
done

work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/oxplay-mpv-dev.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
archive="$work/$name"
if [[ -n ${OXPLAY_MPV_DEV_ARCHIVE:-} ]]; then
  # A previously downloaded (e.g. cached) copy; it is verified below as well.
  cp -- "$OXPLAY_MPV_DEV_ARCHIVE" "$archive"
else
  curl --fail --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 300 --max-filesize 134217728 \
    --output "$archive" \
    "https://github.com/shinchiro/mpv-winbuild-cmake/releases/download/$tag/$name"
fi
printf '%s  %s\n' "$sha" "$archive" | sha256sum --check --strict
# Extract only after authenticating the complete, pinned archive bytes.
7z x -y -bd -o"$work/extract" "$archive" >/dev/null
for required in libmpv-2.dll libmpv.dll.a include/mpv/client.h include/mpv/render_gl.h; do
  if [[ ! -f $work/extract/$required ]]; then
    echo "Pinned archive is missing $required" >&2
    exit 1
  fi
done
# crates/media/build.rs checks the libmpv client API version (>= 2.5).
mv -- "$work/extract" "$destination"
printf '%s SHA256 %s\n' "$name" "$sha" > "$destination/ci-source.txt"
sha256sum "$destination/libmpv-2.dll" "$destination/libmpv.dll.a"
