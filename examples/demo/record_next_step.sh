#!/bin/bash
# record_next_step.sh APP NEW_OUTDIR [case|bracket]: record one autocomplete acceptance with
# headless GNOME Shell and a disposable app home. Uses your existing Claude Code login.
set -eu
umask 077
HERE=$(cd -- "$(dirname -- "$0")" && pwd)
source "$HERE/next_step_harness.sh"
APP=${1:?app required} DESIGN=${3:-case}
[[ $# -le 3 && ( $DESIGN == case || $DESIGN == bracket ) ]] || exit 2
OUT=$(fresh_output "${2:?new output directory required}")
H=$(mktemp -d "${TMPDIR:-/tmp}/sc-demo-home.XXXXXXXX")
trap 'rm -r -- "$H"' EXIT
[[ ! -e $HOME/.claude ]] || ln -s -- "$HOME/.claude" "$H/.claude"
[[ ! -f $HOME/.claude.json ]] || cp -- "$HOME/.claude.json" "$H/.claude.json"
export PATH="$HOME/.npm-global/bin:$HOME/.local/bin:$PATH"
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
dbus-run-session -- bash -c 'source "$1"; shift; run_recording "$@"' record \
  "$HERE/next_step_harness.sh" "$APP" "$OUT" "$H" "${NSM_PORT:-9491}" "$HERE/next_step_demo.py" 160 "$DESIGN"
