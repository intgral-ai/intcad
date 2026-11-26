#!/bin/bash
# next_step_ab.sh APP OUTDIR WHAT...: Intcad (APP) with the built-in agent on a headless GNOME
# Shell (plain Wayland client, no web view), in a home kept between runs (NSM_HOME, default
# /tmp/nsm-home) so the agent's plan sign-in stays. WHAT: `serve` keeps the app up on PORT for a
# sign-in until killed; otherwise the arguments go to next_step_ab.py (DESIGN MODEL).
# Needs SOLIDCRAFT_MASTRA_AGENT (an executable that starts packages/mastra-runtime/agent/main.mjs).
# NSM_AGENT=claude runs Claude Code (claude-agent-acp from ~/.npm-global/bin) instead, signed in
# through Claude Code's own login: this machine's ~/.claude is linked into the home.
set -u
APP=$1; OUT=$(mkdir -p "$2" && cd "$2" && pwd); shift 2
HERE=$(cd "$(dirname "$0")" && pwd)
H=${NSM_HOME:-/tmp/nsm-home}; mkdir -p $H
NSM_AGENT=${NSM_AGENT:-builtin}
# Names this run's processes: the private bus starts services that outlive it (they hold connections of
# the system bus, which then runs out), and they inherit this.
export NSM_RUN=$$-$(date +%s)
if [ "$NSM_AGENT" = claude ]; then
  [ -e $H/.claude ] || ln -s ~/.claude $H/.claude
  [ -f ~/.claude.json ] && cp ~/.claude.json $H/.claude.json
fi
export XDG_RUNTIME_DIR=/run/user/$(id -u)
SOCK=nsm-$$
TOKEN=${NSM_TOKEN:-$(openssl rand -hex 32)}
PORT=${NSM_PORT:-9493}
echo "$TOKEN" > $OUT/token
# No browser here: the sign-in page's address the agent would open is kept for the person instead.
mkdir -p $OUT/bin
printf '#!/bin/sh\numask 077\nprintf "%%s\\n" "$1" > %s/signin-url\n' "$OUT" > $OUT/bin/xdg-open
chmod +x $OUT/bin/xdg-open
# The app keeps keys in the Secret Service. On this private bus that would be the login session's
# keyring, which may wait for an unlock no one sees: a keyring of this home's own answers instead,
# unlocked with a password made for it (never shown, kept in the home).
mkdir -p $H/keyring-ctl; chmod 700 $H/keyring-ctl
[ -f $H/.keyring-pass ] || (umask 077; openssl rand -hex 16 > $H/.keyring-pass)
dbus-run-session -- bash -c "
  env -u GNOME_KEYRING_CONTROL -u SSH_AUTH_SOCK HOME=$H XDG_DATA_HOME=$H/.local/share \
    gnome-keyring-daemon --unlock --components=secrets --control-directory=$H/keyring-ctl < $H/.keyring-pass > $OUT/keyring.log 2>&1
  gnome-shell --headless --wayland --no-x11 --virtual-monitor 1600x1000 --wayland-display=$SOCK > $OUT/shell.log 2>&1 &
  SHELL_PID=\$!
  for i in \$(seq 1 60); do [ -S $XDG_RUNTIME_DIR/$SOCK ] && break; sleep 0.5; done
  sleep 2
  env WAYLAND_DISPLAY=$SOCK HOME=$H XDG_CONFIG_HOME=$H/.config XDG_DATA_HOME=$H/.local/share \
    PATH=$OUT/bin:$HOME/.npm-global/bin:$HOME/.local/bin:\$PATH NSM_AGENT=$NSM_AGENT \
    SOLIDCRAFT_MASTRA_AGENT=${SOLIDCRAFT_MASTRA_AGENT:-} SOLIDCRAFT_CONTROL_TOKEN=$TOKEN RUST_LOG=info \
    $APP --control $PORT > $OUT/app.log 2>&1 &
  APP_PID=\$!
  for i in \$(seq 1 120); do (echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null && break; sleep 0.5; done
  sleep 3
  if [ \"\$1\" = serve ]; then
    echo ready > $OUT/serving; wait \$APP_PID
  else
    NSM_AGENT=$NSM_AGENT TAB=${TAB:-} python3 $HERE/${NSM_SCRIPT:-next_step_ab.py} $PORT $TOKEN $OUT \"\$@\" > $OUT/driver-$NSM_AGENT-\$1-\${2//\\//_}.log 2>&1
    echo driver-exit \$? >> $OUT/driver-$NSM_AGENT-\$1-\${2//\\//_}.log
  fi
  kill \$APP_PID; sleep 2; kill -9 \$SHELL_PID
  pkill -f -- '--control-directory=$H/keyring-[c]tl'
" nsm "$@"
# What the private bus started and left running.
for e in /proc/[0-9]*/environ; do
  p=${e#/proc/}; p=${p%/environ}
  [ "$p" = $$ ] && continue
  if grep -qz "^NSM_RUN=$NSM_RUN\$" "$e" 2>/dev/null; then kill "$p" 2>/dev/null; fi
done 2>/dev/null
true
