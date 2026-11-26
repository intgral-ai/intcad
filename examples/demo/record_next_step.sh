#!/bin/bash
# record_next_step.sh APP NEW_OUTDIR AGENT [PLAN]: record with a headless GNOME Shell and a
# disposable app home. AGENT: `claude` (uses your existing Claude login) or a scripted agent path.
# DRIVER is a script alongside this launcher (default next_step_demo.py).
set -eu
umask 077
HERE=$(cd -- "$(dirname -- "$0")" && pwd)
source "$HERE/next_step_harness.sh"
APP=${1:?app required} AGENT=${3:?agent required} PLAN=${4:-}
OUT=$(fresh_output "${2:?new output directory required}")
DRIVER=${DRIVER:-next_step_demo.py}
[[ $DRIVER == *.py && $DRIVER != */* ]] || exit 2
H=$(mktemp -d "${TMPDIR:-/tmp}/sc-demo-home.XXXXXXXX")
trap 'rm -r -- "$H"' EXIT
if [[ $AGENT == claude ]]; then
  [[ ! -e $HOME/.claude ]] || ln -s -- "$HOME/.claude" "$H/.claude"
  [[ ! -f $HOME/.claude.json ]] || cp -- "$HOME/.claude.json" "$H/.claude.json"
fi
export PATH="$HOME/.npm-global/bin:$HOME/.local/bin:$PATH"
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
dbus-run-session -- bash -c 'source "$1"; shift; run_recording "$@"' record \
  "$HERE/next_step_harness.sh" "$APP" "$OUT" "$H" "${NSM_PORT:-9491}" "$HERE/$DRIVER" 0 "$AGENT" "$PLAN"
