#!/bin/bash
# next_step_ab.sh APP NEW_OUTDIR WHAT...: launch Intcad in a private headless GNOME session.
# Set NSM_HOME to a dedicated profile to keep sign-ins between runs. WHAT is `serve` or DESIGN MODEL.
# NSM_AGENT=claude uses your existing Claude login. The default `builtin` needs SOLIDCRAFT_MASTRA_AGENT.
set -eu
umask 077
APP=${1:?app required}
HERE=$(cd -- "$(dirname -- "$0")" && pwd)
source "$HERE/next_step_harness.sh"
OUT=$(fresh_output "${2:?new output directory required}")
shift 2
NSM_AGENT=${NSM_AGENT:-builtin}
NSM_SCRIPT=${NSM_SCRIPT:-next_step_ab.py}
[[ $NSM_SCRIPT == *.py && $NSM_SCRIPT != */* ]] || exit 2
[[ ${1:-} == serve || $# -ge 2 ]] || exit 2
RUNDIR=$(mktemp -d "${TMPDIR:-/tmp}/next-step-ab.XXXXXXXX")
trap 'rm -r -- "$RUNDIR"' EXIT
H=$RUNDIR/profile
if [[ -n ${NSM_HOME:-} ]]; then
  H=$NSM_HOME
  if [[ -L $H ]]; then
    printf 'NSM_HOME must not be a symlink\n' >&2
    exit 2
  fi
  if mkdir -- "$H" 2>/dev/null; then
    touch -- "$H/.intcad-next-step-profile"
  elif [[ ! -f $H/.intcad-next-step-profile || -L $H/.intcad-next-step-profile ]]; then
    printf 'NSM_HOME must be a new directory or a profile created by this harness\n' >&2
    exit 2
  fi
else
  mkdir -- "$H"
fi
H=$(cd -P -- "$H" && pwd)
if [[ $NSM_AGENT == claude ]]; then
  [[ ! -e $HOME/.claude || -e $H/.claude || -L $H/.claude ]] || ln -s -- "$HOME/.claude" "$H/.claude"
  [[ ! -f $HOME/.claude.json || -e $H/.claude.json || -L $H/.claude.json ]] || cp -- "$HOME/.claude.json" "$H/.claude.json"
fi
mkdir -- "$RUNDIR/bin" "$RUNDIR/keyring-ctl"
cat > "$RUNDIR/bin/xdg-open" <<'OPEN'
#!/bin/sh
umask 077
printf '%s\n' "$1" > "$NSM_OUTPUT/signin-url"
OPEN
chmod +x "$RUNDIR/bin/xdg-open"
if [[ ! -f $H/.keyring-pass ]]; then
  (set -o noclobber; openssl rand -hex 16 > "$H/.keyring-pass")
fi
export PATH="$RUNDIR/bin:$HOME/.npm-global/bin:$HOME/.local/bin:$PATH"
export XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export NSM_AGENT NSM_OUTPUT="$OUT"
TOKEN=$(openssl rand -hex 32)
printf '%s\n' "$TOKEN" > "$OUT/token"

dbus-run-session -- bash -s -- "$HERE/next_step_harness.sh" "$APP" "$OUT" "$H" "$RUNDIR" "${NSM_PORT:-9493}" "$TOKEN" "$HERE/$NSM_SCRIPT" "$@" <<'BASH'
set -eu
source "$1"
APP=$2 OUT=$3 H=$4 RUNDIR=$5 PORT=$6 TOKEN=$7 SCRIPT=$8
shift 8
SOCK=nsm-$$
pids=()
trap 'stop_demo_children "${pids[@]}"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
env -u GNOME_KEYRING_CONTROL -u SSH_AUTH_SOCK HOME="$H" XDG_DATA_HOME="$H/.local/share" \
  gnome-keyring-daemon --foreground --unlock --components=secrets --control-directory="$RUNDIR/keyring-ctl" \
  < "$H/.keyring-pass" > "$OUT/keyring.log" 2>&1 &
pids+=("$!")
gnome-shell --headless --wayland --no-x11 --virtual-monitor 1600x1000 --wayland-display="$SOCK" > "$OUT/shell.log" 2>&1 &
pids+=("$!")
wait_for "$!" 60 test -S "$XDG_RUNTIME_DIR/$SOCK"
env WAYLAND_DISPLAY="$SOCK" HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" \
  SOLIDCRAFT_SETTINGS_DIR="$H/settings" SOLIDCRAFT_CONTROL_TOKEN="$TOKEN" RUST_LOG=info \
  "$APP" --control "$PORT" > "$OUT/app.log" 2>&1 &
APP_PID=$!
pids+=("$APP_PID")
wait_for_control "$APP_PID" "$PORT" "$TOKEN"
if [[ $1 == serve ]]; then
  printf 'ready\n' > "$OUT/serving"
  wait "$APP_PID"
else
  rc=0
  TAB=${TAB:-} python3 "$SCRIPT" "$PORT" "$TOKEN" "$OUT" "$@" > "$OUT/driver.log" 2>&1 || rc=$?
  printf 'driver-exit %s\n' "$rc" >> "$OUT/driver.log"
  exit "$rc"
fi
BASH
