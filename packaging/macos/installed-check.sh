#!/usr/bin/env bash
# Install check for the macOS disk image: copy SolidCraft.app out of the .dmg, verify the
# Assistant page and its records inside that copy, then open the Assistant in it with the network
# taken away. The build tree is never treated as the installed app.
#
#   packaging/macos/installed-check.sh [path/to.dmg] [evidence-dir]
#
# Defaults: the one .dmg in $DIST, and $RUNNER_TEMP/installed-check-macos when RUNNER_TEMP is set.
# Set SOLIDCRAFT_MACOS_SCREEN=1 to keep a picture of the whole screen as well.
#
# The app is started under packaging/macos/no-network.sb. The script first shows that the profile
# stops /usr/bin/curl, and refuses to start the app if it does not. It does not turn the WebKit
# sandbox off, broaden permissions, or hide errors. Signing and notarization are reported as they
# are: without Apple credentials these packages are ad-hoc signed, and Gatekeeper is not expected
# to accept them on a user's Mac.
set -euo pipefail
# shellcheck source=../env.sh
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"

DMG="${1:-}"
if [ -z "$DMG" ]; then
  DMG="$(find "$DIST" -maxdepth 1 -name '*.dmg' -print -quit)"
fi
if [ -z "$DMG" ] || [ ! -f "$DMG" ]; then
  echo "error: no disk image found; pass one: packaging/macos/installed-check.sh path/to/SolidCraft.dmg" >&2
  exit 2
fi
EVIDENCE="${2:-${RUNNER_TEMP:-$PWD}/installed-check-macos}"
mkdir -p "$EVIDENCE"

MNT="$(mktemp -d "${TMPDIR:-/tmp}/solidcraft-dmg.XXXXXX")"
APPS="$(mktemp -d "${TMPDIR:-/tmp}/solidcraft-apps.XXXXXX")"
MOUNTED=0
cleanup() {
  if [ "$MOUNTED" = 1 ]; then
    hdiutil detach "$MNT" -quiet || hdiutil detach "$MNT" -force || true
  fi
  rm -rf "$MNT" "$APPS"
}
trap cleanup EXIT INT TERM

echo "==> mounting $DMG"
hdiutil attach -readonly -nobrowse -mountpoint "$MNT" "$DMG"
MOUNTED=1
if [ ! -d "$MNT/SolidCraft.app" ]; then
  echo "error: the disk image has no SolidCraft.app" >&2
  exit 1
fi
echo "==> copying SolidCraft.app out of the disk image"
cp -R "$MNT/SolidCraft.app" "$APPS/"
hdiutil detach "$MNT" -quiet
MOUNTED=0
APP="$APPS/SolidCraft.app"
EXE="$APP/Contents/MacOS/SolidCraft"
case "$APP" in
  "$MNT"/*) echo "error: the app was not copied out of the disk image" >&2; exit 1 ;;
esac
test -x "$EXE"

echo "==> the Assistant page and its records inside the installed copy"
python3 "$ROOT/packaging/macos/verify-assistant-bundle.py" --app "$APP" --source "$ROOT/packages/assistant-web"

echo "==> the bundle's local signature"
codesign --verify --deep --verbose=2 "$APP"
if spctl --assess --type execute --verbose=4 "$APP"; then
  echo "ok: Gatekeeper accepts this copy"
else
  echo "note: Gatekeeper does not accept this copy. The package is ad-hoc signed unless Apple signing/notarization credentials were provided; that work stays configurable and pending."
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "macOS installed check: Gatekeeper rejected the ad-hoc/unnotarized copy; signing and notarization remain pending." >>"$GITHUB_STEP_SUMMARY"
  fi
fi

SANDBOX="$ROOT/packaging/macos/no-network.sb"
if ! command -v sandbox-exec >/dev/null 2>&1; then
  echo "error: sandbox-exec is not available; refusing to start the app with the network open" >&2
  exit 1
fi
echo "==> showing that the sandbox takes the network away"
if sandbox-exec -f "$SANDBOX" /usr/bin/curl -s -m 5 -o /dev/null https://api.github.com/; then
  echo "error: the sandbox profile does not take the network away on this machine; the app is not started" >&2
  exit 1
fi
echo "ok: curl under packaging/macos/no-network.sb cannot reach the network"

OFFLINE="sandbox-exec -f $SANDBOX"

# macOS ships bash 3.2: do not expand an empty array under `set -u`.
run_assistant_check() {
  local evidence="$1" prefix="$2"
  if [ "${SOLIDCRAFT_MACOS_SCREEN:-0}" = 1 ]; then
    python3 "$ROOT/packaging/assistant-check.py" --run-with "$prefix" --exe "$EXE" \
      --evidence "$evidence" --screen
  else
    python3 "$ROOT/packaging/assistant-check.py" --run-with "$prefix" --exe "$EXE" \
      --evidence "$evidence"
  fi
}

echo "==> the Assistant in the installed app"
run_assistant_check "$EVIDENCE/native" "$OFFLINE"

# The other half of a universal file, where this machine can run it.
if arch -x86_64 /usr/bin/true 2>/dev/null; then
  echo "==> the Intel half of the universal app"
  run_assistant_check "$EVIDENCE/x86_64" "$OFFLINE arch -x86_64"
else
  warn "no Rosetta on this machine, so the Intel half of the app was not started"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "macOS installed check: no Rosetta, so the Intel half of the universal app was not started; the native half and both architectures' embedded page files were checked." >>"$GITHUB_STEP_SUMMARY"
  fi
fi

echo "==> done"
