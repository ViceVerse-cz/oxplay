#!/bin/sh
# CI/source-build hosts only: native build and packaging tools per distribution.
# Ubuntu 24.04 builds a private libmpv 0.41 (its system libmpv is too old);
# Fedora and Arch link the distribution's libmpv (client API >= 2.5 required).
set -eu
. /etc/os-release
if [ "$(id -u)" -ne 0 ]; then SUDO=sudo; else SUDO=; fi
case "$ID:${VERSION_ID:-rolling}" in
  ubuntu:24.04)
    $SUDO apt-get update
    DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl pkg-config meson ninja-build python3 \
      libavcodec-dev libavfilter-dev libavformat-dev libavutil-dev \
      libswresample-dev libswscale-dev libass-dev libplacebo-dev libluajit-5.1-dev \
      libegl1-mesa-dev libgl1-mesa-dev libwayland-dev wayland-protocols \
      libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libxss-dev \
      libxext-dev libxpresent-dev libxrandr-dev libxi-dev libxcursor-dev \
      libasound2-dev libpulse-dev libfontconfig1-dev \
      dpkg-dev desktop-file-utils patchelf zsync file apt-utils gnupg
    ;;
  fedora:43|fedora:44)
    dnf install -y --setopt=install_weak_deps=False gcc gcc-c++ make pkgconf-pkg-config \
      curl ca-certificates tar gzip xz python3 coreutils findutils shadow-utils util-linux \
      rpm-build cpio desktop-file-utils file glibc-common \
      'pkgconfig(mpv)' fontconfig-devel freetype-devel libxkbcommon-devel libxkbcommon-x11-devel \
      wayland-devel libX11-devel libXi-devel libXrandr-devel libXcursor-devel mesa-libEGL-devel
    ;;
  arch:*)
    pacman -Syu --noconfirm --needed base-devel pkgconf curl ca-certificates tar gzip xz \
      python coreutils desktop-file-utils file mpv fontconfig freetype2 libxkbcommon \
      libxkbcommon-x11 wayland libx11 libxi libxrandr libxcursor libglvnd
    ;;
  *)
    printf '%s\n' "Unsupported build host: $ID ${VERSION_ID:-rolling}" >&2
    exit 1
    ;;
esac
if [ "$ID" != ubuntu ]; then
  # The distribution libmpv must expose the client API this application requires.
  pkg-config --atleast-version=2.5 mpv || {
    printf '%s\n' "System libmpv $(pkg-config --modversion mpv 2>/dev/null || echo missing) is older than client API 2.5" >&2
    exit 1
  }
  printf 'System libmpv client API %s\n' "$(pkg-config --modversion mpv)"
fi
