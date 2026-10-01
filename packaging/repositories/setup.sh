#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
# Oxplay package repository setup (Ubuntu 24.04, Fedora 44, Arch Linux; x86_64).
# Detects the distribution, verifies the signing key fingerprint and configures
# the signed repository. The publishing workflow substitutes the fingerprint.
set -eu

BASE_URL="${OXPLAY_REPO_BASE_URL:-https://viceverse-cz.github.io/oxplay}"
CHANNEL="${OXPLAY_CHANNEL:-production}"
EXPECTED_FINGERPRINT="${OXPLAY_FINGERPRINT:-@FINGERPRINT@}"

if [ "$(id -u)" -ne 0 ]; then
    SUDO="sudo"
else
    SUDO=""
fi

if [ -t 1 ]; then
    BOLD=$(printf '\033[1m'); DIM=$(printf '\033[2m'); BLUE=$(printf '\033[1;34m')
    CYAN=$(printf '\033[1;36m'); GREEN=$(printf '\033[1;32m'); PURPLE=$(printf '\033[1;35m')
    RED=$(printf '\033[1;31m'); NC=$(printf '\033[0m')
else
    BOLD=""; DIM=""; BLUE=""; CYAN=""; GREEN=""; PURPLE=""; RED=""; NC=""
fi

log() { printf " %b::%b %s\n" "${BLUE}" "${NC}" "$1"; }
success() { printf " %bok%b %s\n" "${GREEN}" "${NC}" "$1"; }
error() { printf " %berror:%b %s\n" "${RED}" "${NC}" "$1" >&2; exit 1; }

case "$CHANNEL" in
    nightly|production) ;;
    *) error "OXPLAY_CHANNEL must be nightly or production." ;;
esac
case "$EXPECTED_FINGERPRINT" in
    *[!0-9A-Fa-f]*|"") error "No signing key fingerprint is configured; set OXPLAY_FINGERPRINT to the published fingerprint." ;;
esac

if command -v curl >/dev/null 2>&1; then
    download() { curl --fail --silent --show-error --location --proto '=https' "$1" -o "$2"; }
elif command -v wget >/dev/null 2>&1; then
    download() { wget --quiet --https-only -O "$2" "$1"; }
else
    error "Either curl or wget is required to download repository configuration."
fi

verify_key() {
    command -v gpg >/dev/null 2>&1 || error "gpg is required to verify the signing key."
    fingerprint=$(gpg --batch --show-keys --with-colons "$1" 2>/dev/null | awk -F: '$1 == "fpr" {print $10; exit}')
    [ -n "$fingerprint" ] || error "Could not extract a fingerprint from the downloaded signing key."
    fingerprint=$(printf '%s' "$fingerprint" | tr '[:lower:]' '[:upper:]')
    expected=$(printf '%s' "$EXPECTED_FINGERPRINT" | tr '[:lower:]' '[:upper:]')
    [ "$fingerprint" = "$expected" ] || error "GPG fingerprint mismatch (expected $expected, got $fingerprint); refusing an untrusted key."
    success "Signing key verified (${DIM}$fingerprint${NC})"
}

[ -f /etc/os-release ] || error "/etc/os-release not found. Unsupported Linux distribution."
. /etc/os-release
DISTRO_ID="$ID"
case " ${ID_LIKE:-} " in
    *" arch "*) DISTRO_ID="arch" ;;
esac
ARCH=$(uname -m)
[ "$ARCH" = x86_64 ] || error "Architecture $ARCH has no Oxplay package repository; use the AppImage or build from source."

# Native packages must match the distribution whose libraries they were built against.
case "$DISTRO_ID:${VERSION_ID:-}" in
    ubuntu:24.04) REPO_PATH="ubuntu-24.04/amd64/apt" ;;
    fedora:44) REPO_PATH="fedora-44/$ARCH/rpm" ;;
    arch:*) REPO_PATH="arch/$ARCH/arch" ;;
    *) error "No native repository for $ID ${VERSION_ID:-rolling}. Use the AppImage from the GitHub release." ;;
esac
REPO_URL="$BASE_URL/$CHANNEL/$REPO_PATH"
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT HUP INT TERM
KEY_FILE="$TEMP_DIR/oxplay.asc"

log "Configuring the Oxplay ${BOLD}${CHANNEL}${NC} repository for ${BOLD}${PRETTY_NAME:-$ID}${NC} (${ARCH})..."
download "$REPO_URL/oxplay.asc" "$KEY_FILE" || error "The signing key for this distribution is not published yet."
verify_key "$KEY_FILE"

case "$DISTRO_ID" in
    ubuntu)
        $SUDO install -Dm644 "$KEY_FILE" /etc/apt/keyrings/oxplay.asc
        printf 'deb [arch=amd64 signed-by=/etc/apt/keyrings/oxplay.asc] %s ./\n' "$REPO_URL" | \
            $SUDO tee /etc/apt/sources.list.d/oxplay.list >/dev/null
        $SUDO apt-get update -o Dir::Etc::sourcelist="sources.list.d/oxplay.list" \
            -o Dir::Etc::sourceparts="-" -o APT::Get::List-Cleanup="0" >/dev/null
        INSTALL_CMD="$SUDO apt install oxplay"
        ;;
    fedora)
        download "$REPO_URL/oxplay.repo" "$TEMP_DIR/oxplay.repo"
        $SUDO rpm --import "$KEY_FILE"
        $SUDO install -m644 "$TEMP_DIR/oxplay.repo" /etc/yum.repos.d/oxplay.repo
        INSTALL_CMD="$SUDO dnf install oxplay"
        ;;
    arch)
        $SUDO pacman-key --add "$KEY_FILE" >/dev/null 2>&1
        $SUDO pacman-key --lsign-key "$EXPECTED_FINGERPRINT" >/dev/null 2>&1
        if grep -q '^\[oxplay\]' /etc/pacman.conf; then
            log "Repository [oxplay] is already present in /etc/pacman.conf."
        else
            printf '\n[oxplay]\nSigLevel = Required\nServer = %s\n' "$REPO_URL" | $SUDO tee -a /etc/pacman.conf >/dev/null
        fi
        INSTALL_CMD="$SUDO pacman -Syu oxplay"
        ;;
esac

success "Repository configuration complete."
DO_INSTALL=false
if [ -t 0 ] || ( : </dev/tty ) 2>/dev/null; then
    printf "%b?%b Install %bOxplay%b now? [Y/n]: " "${PURPLE}" "${NC}" "${BOLD}" "${NC}"
    if [ -t 0 ]; then read -r answer || answer=n; else read -r answer </dev/tty || answer=n; fi
    case "$answer" in
        [nN]|[nN][oO]) ;;
        *) DO_INSTALL=true ;;
    esac
fi
if [ "$DO_INSTALL" = true ]; then
    log "Installing Oxplay (${INSTALL_CMD})..."
    if [ -t 0 ]; then $INSTALL_CMD; else $INSTALL_CMD </dev/tty; fi
    success "Oxplay installed. Launch it from your applications menu or run ${BOLD}oxplay${NC}."
else
    log "To install Oxplay later, run:"
    printf '\n    %b%s%b\n\n' "${CYAN}" "$INSTALL_CMD" "${NC}"
fi
