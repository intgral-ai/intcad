#!/bin/bash
# probe_hand.sh APP NEW_OUTDIR: a throwaway headless session running probe_hand.py.
set -eu
umask 077
HERE=$(cd -- "$(dirname -- "$0")" && pwd)
source "$HERE/next_step_harness.sh"
OUT=$(fresh_output "${2:?new output directory required}")
H=$(mktemp -d "${TMPDIR:-/tmp}/sc-probe-home.XXXXXXXX")
trap 'rm -r -- "$H"' EXIT
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
dbus-run-session -- bash -c 'source "$1"; shift; run_recording "$@"' probe \
  "$HERE/next_step_harness.sh" "${1:?app required}" "$OUT" "$H" "${NSM_PORT:-9495}" "$HERE/probe_hand.py" 400
