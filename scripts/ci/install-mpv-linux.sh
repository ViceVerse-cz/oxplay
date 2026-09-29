#!/usr/bin/env bash
# CI-only, unmodified official libmpv source. Does not install into /usr or /usr/local.
set -euo pipefail
umask 022

if [[ $(uname -s) != Linux || $# != 1 || $1 != /* || -e $1 || -L $1 ]]; then
  echo 'Expected Linux and a new absolute install prefix.' >&2
  exit 2
fi
prefix=$1
for tool in curl sha256sum tar meson ninja timeout pkg-config; do
  command -v "$tool" >/dev/null
done

work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/serein-mpv-build.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT
archive="$work/mpv.tar.gz"
source_sha=ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209
curl --fail --location --proto '=https' --proto-redir '=https' \
  --connect-timeout 15 --max-time 180 --max-filesize 33554432 \
  --output "$archive" https://github.com/mpv-player/mpv/archive/refs/tags/v0.41.0.tar.gz
printf '%s  %s\n' "$source_sha" "$archive" | sha256sum --check --strict
mkdir "$work/source"
# Extract only after authenticating the complete, pinned upstream archive bytes.
tar --extract --gzip --file "$archive" --directory "$work/source" \
  --strip-components=1 --no-same-owner --no-same-permissions

# mpv only exposes its script-disable options when built with a script engine.
# Preserve that capability so production can explicitly disable every built-in
# and inherited script; compiling Lua out makes those required options unknown.
timeout --kill-after=10s 120s meson setup "$work/build" "$work/source" \
  --prefix="$prefix" --libdir=lib --buildtype=release --wrap-mode=nofallback \
  -Dauto_features=disabled -Dlibmpv=true -Dcplayer=false -Dbuild-date=false \
  -Dtests=false -Dgl=enabled -Dplain-gl=enabled -Degl=enabled \
  -Dx11=enabled -Dwayland=enabled -Degl-x11=enabled -Degl-wayland=enabled \
  -Dalsa=enabled -Dpulse=enabled -Dlua=luajit -Dmanpage-build=disabled
timeout --kill-after=10s 900s meson compile --ninja-args=-j2 -C "$work/build"
timeout --kill-after=10s 60s meson install -C "$work/build" --no-rebuild
PKG_CONFIG_PATH="$prefix/lib/pkgconfig" pkg-config --atleast-version=2.5 mpv
printf 'mpv 0.41.0 source SHA256 %s\n' "$source_sha" > "$prefix/ci-source.txt"
cp "$work/build/meson-info/intro-buildoptions.json" "$prefix/ci-build-options.json"
cp "$work/build/meson-info/intro-dependencies.json" "$prefix/ci-build-dependencies.json"
sha256sum "$prefix/lib/libmpv.so.2"
