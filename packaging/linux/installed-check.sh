#!/usr/bin/env bash
# Install one Linux package on a bare system, as a user would, and open the Assistant there.
#
# Usage: packaging/linux/installed-check.sh IMAGE PACKAGE [x11|wayland]
#
#   IMAGE    a distribution's container image with nothing of ours in it (ubuntu:24.04, fedora:44)
#   PACKAGE  a .deb, .rpm or .AppImage from package.sh
#   x11      a virtual X display (the default)
#   wayland  a Wayland session (sway on a headless output, with XWayland), where the app opens
#            an X11 window through XWayland by itself
#
# Needs docker. The log and the pictures go to $EVIDENCE/<name> (default: target/installed-check).
# A build tree, or the machine that built the package, proves nothing here: it has libraries a
# user's machine may lack.
#
# Two containers, because a package check never starts a release build of the desktop app with
# the network open (docs/releasing.md): the first installs the package and the test's tools and
# is saved as an image; the second is that image with no network at all, and only there is the
# app started. What runs inside is installed-check-inside.sh.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ $# -lt 2 ]; then sed -n '2,19p' "$0"; exit 2; fi
IMAGE="$1"
PACKAGE="$2"
SESSION="${3:-x11}"
[ -f "$PACKAGE" ] || { echo "error: $PACKAGE not found" >&2; exit 1; }
case "$SESSION" in
  x11 | wayland) ;;
  *) echo "unknown session: $SESSION" >&2; exit 2 ;;
esac
command -v docker >/dev/null || { echo "error: docker not found" >&2; exit 1; }

EVIDENCE="${EVIDENCE:-$HERE/../../target/installed-check}"
NAME="${PACKAGE##*.}-$(printf '%s' "$IMAGE" | tr -c 'A-Za-z0-9.' '-')-$SESSION"
mkdir -p "$EVIDENCE/$NAME"
EVIDENCE_DIR="$(cd "$EVIDENCE/$NAME" && pwd)"
DIST_DIR="$(cd "$(dirname "$PACKAGE")" && pwd)"
PACKAGING_DIR="$(cd "$HERE/.." && pwd)"
BUILDER="installed-check-$$"
INSTALLED="installed-check:$$"
cleanup() {
  docker rm -f "$BUILDER" >/dev/null 2>&1 || true
  docker rmi -f "$INSTALLED" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> $(basename "$PACKAGE") on $IMAGE ($SESSION): installing"
# DOCKER_RUN_ARGS: extra `docker run` arguments for the installing container, for a machine
# that needs them (a DNS server, say).
# shellcheck disable=SC2086
docker run --name "$BUILDER" ${DOCKER_RUN_ARGS:-} \
  -v "$DIST_DIR:/dist:ro" -v "$PACKAGING_DIR:/packaging:ro" \
  "$IMAGE" bash /packaging/linux/installed-check-inside.sh install "/dist/$(basename "$PACKAGE")" "$SESSION"
docker commit "$BUILDER" "$INSTALLED" >/dev/null
docker rm "$BUILDER" >/dev/null

echo "==> $(basename "$PACKAGE") on $IMAGE ($SESSION): starting the installed app, with no network"
docker run --rm --network none \
  -v "$PACKAGING_DIR:/packaging:ro" -v "$EVIDENCE_DIR:/evidence" \
  "$INSTALLED" bash /packaging/linux/installed-check-inside.sh run "$SESSION"
echo "==> passed: $(basename "$PACKAGE") on $IMAGE ($SESSION); evidence in $EVIDENCE_DIR"
