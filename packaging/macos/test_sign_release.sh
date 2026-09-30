#!/bin/bash
# The stubs read variables (keychain, scenario) set by the sourced script and loop.
# shellcheck disable=SC2154,SC1090
# SPDX-License-Identifier: GPL-3.0-or-later
# Offline regression: every Apple/keychain command below is a synthetic stub.
set -euo pipefail

script=${1:-packaging/macos/sign-release.sh}
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
export RUNNER_TEMP="$fixture/runner temp"
export MACOS_CERTIFICATE_BASE64=c3ludGhldGlj MACOS_CERTIFICATE_PASSWORD=synthetic-password
export MACOS_SIGNING_IDENTITY='Developer ID Application: Synthetic Owner (TESTTEAM01)'
export APPLE_ID=synthetic@example.invalid APPLE_TEAM_ID=TESTTEAM01
export APPLE_APP_SPECIFIC_PASSWORD=synthetic-apple-password
expected_fingerprint=0123456789ABCDEF0123456789ABCDEF01234567
app="$fixture/Oxplay.app"
mkdir -p "$RUNNER_TEMP" "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Helpers" \
  "$app/Contents/Resources/HelperRuntime/Python/bin" "$app/Contents/Resources/HelperRuntime/Python/lib/python3.14/lib-dynload"
for path in MacOS/oxplay Frameworks/libmpv.2.dylib Helpers/deno Helpers/yt-dlp Helpers/oxplay-dns \
  Resources/HelperRuntime/Python/bin/python3.14 Resources/HelperRuntime/Python/lib/python3.14/lib-dynload/_ssl.so \
  Resources/HelperRuntime/Python/lib/python3.14/os.py Info.plist; do
  printf 'synthetic\n' > "$app/Contents/$path"
done

security() {
  case "$1" in
    list-keychains)
      if [[ "${4:-}" == -s ]]; then
        : > "$fixture/search-list"
        for entry in "${@:5}"; do printf '%s\n' "$entry" >> "$fixture/search-list"; done
      else
        sed 's/^/    "/; s/$/"/' "$fixture/search-list"
      fi ;;
    create-keychain) touch "$keychain" ;;
    set-keychain-settings|unlock-keychain|set-key-partition-list) ;;
    import) [[ "$scenario" != import-failure ]] ;;
    find-identity)
      [[ "$*" == "find-identity -v -p codesigning $keychain" ]] || return 1
      grep -Fxq "$keychain" "$fixture/search-list" || return 1
      printf '%s\n' preflight >> "$fixture/events"
      case "$scenario" in
        missing) echo '     0 valid identities found' ;;
        mismatch) printf '  1) %s "Developer ID Application: Another Owner (TESTTEAM01)"\n' "$expected_fingerprint" ;;
        ambiguous)
          printf '  1) %s "%s"\n' "$expected_fingerprint" "$MACOS_SIGNING_IDENTITY"
          printf '  2) %s "%s"\n' 89ABCDEF0123456789ABCDEF0123456789ABCDEF "$MACOS_SIGNING_IDENTITY" ;;
        *) printf '  1) %s "%s"\n     1 valid identities found\n' "$expected_fingerprint" "$MACOS_SIGNING_IDENTITY" ;;
      esac ;;
    delete-keychain) printf '%s\n' cleanup >> "$fixture/events" ;;
    *) echo "Unexpected security command: $1" >&2; return 99 ;;
  esac
}
openssl() { printf '%s\n' synthetic-keychain-password; }
file() {
  case "$2" in
    *.py|*.plist) echo 'ASCII text' ;;
    *.dylib) echo 'Mach-O 64-bit dynamically linked shared library arm64' ;;
    *.so) echo 'Mach-O 64-bit bundle arm64' ;;
    *) echo 'Mach-O 64-bit executable arm64' ;;
  esac
}
codesign() {
  if [[ "$1" == --force ]]; then
    grep -Fxq preflight "$fixture/events" || return 1
    grep -Fxq "$keychain" "$fixture/search-list" || return 1
    [[ " $* " == *" --sign $expected_fingerprint "* && " $* " == *" --keychain $keychain "* ]] || return 1
    [[ " $* " == *" --options runtime "* && " $* " == *" --timestamp "* ]] || return 1
    local entitlement=none
    [[ " $* " == *"helpers.entitlements"* ]] && entitlement=helpers
    [[ " $* " == *"app.entitlements"* ]] && entitlement=app
    printf 'sign %s %s\n' "${*: -1}" "$entitlement" >> "$fixture/events"
    [[ "$scenario" != signing-failure ]]
  else
    printf '%s\n' verify >> "$fixture/events"
  fi
}
ditto() { :; }
# plutil -extract KEY raw -o - FILE
plutil() { sed -n "s/.*\"$2\": *\"\\([^\"]*\\)\".*/\\1/p" "$6"; }
xcrun() {
  printf '%s\n' "$1 $2" >> "$fixture/events"
  if [[ "$1 $2" == 'notarytool submit' ]]; then
    if [[ "$scenario" == notarization-failure ]]; then
      echo '{"id": "synthetic-id", "status": "Invalid"}'
    else
      echo '{"id": "synthetic-id", "status": "Accepted"}'
    fi
  fi
}
spctl() { printf '%s\n' verified >> "$fixture/events"; }

for scenario in success empty-search-list missing mismatch ambiguous import-failure signing-failure notarization-failure; do
  : > "$fixture/original"
  if [[ "$scenario" != empty-search-list ]]; then
    printf '%s\n' '/tmp/login keychain-db' '/tmp/other.keychain-db' > "$fixture/original"
  fi
  cp "$fixture/original" "$fixture/search-list"
  : > "$fixture/events"
  # Launch a subshell normally: an `if source ...` would disable the script's errexit.
  set +e
  ( source "$script" "$app" ) > "$fixture/output" 2>&1
  result=$?
  set -e
  cmp "$fixture/original" "$fixture/search-list"
  grep -Fxq cleanup "$fixture/events"
  [[ -z "$(ls -A "$RUNNER_TEMP")" ]]
  ! grep -Fq "$MACOS_CERTIFICATE_PASSWORD" "$fixture/output" || exit 1
  ! grep -Fq "$APPLE_APP_SPECIFIC_PASSWORD" "$fixture/output" || exit 1
  ! grep -Fq "$MACOS_CERTIFICATE_BASE64" "$fixture/output" || exit 1
  if [[ "$scenario" == success || "$scenario" == empty-search-list ]]; then
    [[ "$result" == 0 ]] || { cat "$fixture/output"; echo "Synthetic signing failed: $scenario" >&2; exit 1; }
    grep -Fxq verified "$fixture/events"
    signed=$(grep '^sign ' "$fixture/events")
    # Nested code first (deepest first), the bundle last with the app entitlements.
    [[ "$(tail -n 1 <<< "$signed")" == "sign $app app" ]]
    [[ "$(head -n 1 <<< "$signed")" == "sign $app/Contents/Resources/HelperRuntime/Python/lib/python3.14/lib-dynload/_ssl.so none" ]]
    grep -Fxq "sign $app/Contents/Helpers/deno helpers" <<< "$signed"
    grep -Fxq "sign $app/Contents/Resources/HelperRuntime/Python/bin/python3.14 helpers" <<< "$signed"
    grep -Fxq "sign $app/Contents/Frameworks/libmpv.2.dylib none" <<< "$signed"
    grep -Fxq "sign $app/Contents/Helpers/yt-dlp none" <<< "$signed"
    grep -Fxq "sign $app/Contents/Helpers/oxplay-dns none" <<< "$signed"
    ! grep -Fq 'os.py' <<< "$signed" || exit 1
    ! grep -Fq 'Info.plist' <<< "$signed" || exit 1
    ! grep -Fq "sign $app/Contents/MacOS/oxplay" <<< "$signed" || exit 1
    [[ "$(wc -l <<< "$signed")" -eq 7 ]]
    grep -Fxq 'stapler staple' "$fixture/events"
  else
    [[ "$result" != 0 ]]
    ! grep -Fxq verified "$fixture/events" || exit 1
    ! grep -Fxq 'stapler staple' "$fixture/events" || exit 1
    case "$scenario" in
      missing|mismatch|ambiguous)
        grep -Fq 'MACOS_SIGNING_IDENTITY' "$fixture/output"
        ! grep -q '^sign ' "$fixture/events" || exit 1
        ! grep -Fq notarytool "$fixture/events" || exit 1 ;;
      notarization-failure)
        grep -Fxq 'notarytool log' "$fixture/events"
        grep -Fq 'Notarization status: Invalid' "$fixture/output" ;;
    esac
  fi
done
echo 'Mac signing regression passed: exact identity, inside-out nested signing, entitlements, notarization gate and keychain cleanup (synthetic).'
