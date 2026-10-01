#!/bin/sh
# CI/source-build hosts only: native build and packaging tools per distribution.
# Every release builds private patched libmpv/libplacebo/full FFmpeg; no stock
# distribution libmpv can implement the caller-owned native Vulkan ABI.
set -eu
. /etc/os-release
if [ "$(id -u)" -ne 0 ]; then SUDO=sudo; else SUDO=; fi
case "$ID:${VERSION_ID:-rolling}" in
  ubuntu:24.04)
    # Portable artifacts ship part of the distribution closure. Enable the
    # matching signed source indexes so apt retrieves corresponding sources.
    test -f /etc/apt/sources.list.d/ubuntu.sources
    $SUDO sed -i 's/^Types: deb$/Types: deb deb-src/' /etc/apt/sources.list.d/ubuntu.sources
    $SUDO apt-get update
    DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y --no-install-recommends \
      build-essential ca-certificates curl git pkg-config meson ninja-build python3 \
      nasm cmake libgnutls28-dev libass-dev libluajit-5.1-dev \
      libshaderc-dev libvulkan-dev libva-dev libdrm-dev libdisplay-info-dev \
      libegl1-mesa-dev libgl1-mesa-dev libwayland-dev wayland-protocols \
      libxkbcommon-dev libxkbcommon-x11-dev libx11-dev libxss-dev \
      libxext-dev libxpresent-dev libxrandr-dev libxi-dev libxcursor-dev \
      libasound2-dev libpulse-dev libfontconfig1-dev \
      dpkg-dev desktop-file-utils patchelf zsync file apt-utils gnupg
    ;;
  fedora:43|fedora:44)
    dnf install -y --setopt=install_weak_deps=False gcc gcc-c++ make pkgconf-pkg-config \
      curl git ca-certificates tar gzip xz python3 coreutils findutils shadow-utils util-linux \
      meson ninja-build nasm cmake gnutls-devel libass-devel luajit-devel \
      libshaderc-devel vulkan-loader-devel vulkan-headers libva-devel libdrm-devel libdisplay-info-devel \
      alsa-lib-devel pulseaudio-libs-devel patchelf \
      rpm-build cpio desktop-file-utils file glibc-common \
      fontconfig-devel freetype-devel libxkbcommon-devel libxkbcommon-x11-devel \
      wayland-devel libX11-devel libXi-devel libXrandr-devel libXcursor-devel mesa-libEGL-devel
    ;;
  arch:*)
    pacman -Syu --noconfirm --needed base-devel pkgconf curl git ca-certificates tar gzip xz \
      python coreutils desktop-file-utils file meson ninja nasm cmake \
      gnutls libass luajit shaderc vulkan-headers vulkan-icd-loader libva libdrm libdisplay-info \
      alsa-lib libpulse patchelf fontconfig freetype2 libxkbcommon \
      libxkbcommon-x11 wayland libx11 libxi libxrandr libxcursor libglvnd
    ;;
  *)
    printf '%s\n' "Unsupported build host: $ID ${VERSION_ID:-rolling}" >&2
    exit 1
    ;;
esac
