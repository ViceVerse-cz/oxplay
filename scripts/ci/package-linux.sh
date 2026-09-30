#!/usr/bin/env bash
# CI-only experimental Linux x86_64 tarball: release binary plus the isolated libmpv it was linked against.
# Usage: package-linux.sh <tag> <mpv-prefix> <output-dir>
set -euo pipefail
umask 022

if [[ $(uname -s) != Linux || $(uname -m) != x86_64 || $# != 3 ]]; then
  echo 'Usage: package-linux.sh <tag> <mpv-prefix> <output-dir> (Linux x86_64 only).' >&2
  exit 2
fi
tag=$1 mpv=$2 output=$3
binary=target/release/oxplay
test -x "$binary"
test -e "$mpv/lib/libmpv.so.2"

name="oxplay-$tag-Linux-X64"
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
root="$stage/$name"
mkdir -p "$root/bin" "$root/lib" "$output"
install -m 755 "$binary" "$root/bin/oxplay"
cp -a "$mpv"/lib/libmpv.so* "$root/lib/"
cp LICENSE "$root/"
cp "$mpv/ci-source.txt" "$root/lib/MPV-SOURCE.txt"

cat > "$root/oxplay" <<'LAUNCHER'
#!/usr/bin/env bash
here=$(cd "$(dirname "$(readlink -f "$0")")" && pwd)
export LD_LIBRARY_PATH="$here/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$here/bin/oxplay" "$@"
LAUNCHER
chmod 755 "$root/oxplay"

cat > "$root/README.txt" <<'README'
Oxplay experimental Linux x86_64 build (unsigned, not runtime-qualified).

Run ./oxplay. It uses the bundled libmpv 0.41.0 (see lib/MPV-SOURCE.txt) and needs
system FFmpeg 6.x, libplacebo, LuaJIT, GL/EGL, X11 or Wayland, ALSA/PulseAudio
libraries and yt-dlp on PATH (Ubuntu 24.04 package names). Native X11 and Wayland
playback are unvalidated.
README

tar --create --gzip --file "$output/$name.tar.gz" --directory "$stage" \
  --owner=0 --group=0 --numeric-owner "$name"
ls -l "$output/$name.tar.gz"
