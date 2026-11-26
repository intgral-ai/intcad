#!/bin/bash
# record_next_step.sh APP OUTDIR AGENT [PLAN]: a headless GNOME Shell with a rootful Xwayland at
# 1920x1080 (display :21, no window manager, so the app fills it undecorated), Intcad (APP) on it
# in a throwaway home with the Claude login linked in, and a driver (DRIVER, default
# next_step_demo.py; next_step_bracket.py for the bracket) recording one take into OUTDIR. AGENT:
# `claude` or a scripted agent's path. SOLIDCRAFT_DEMO_KEYS=1 turns on the app's key strip and
# pointer, so the raw recording shows the keys pressed and the person's pointer.
set -u
APP=$1; OUT=$(mkdir -p "$2" && cd "$2" && pwd); AGENT=$3; PLAN=${4:-}
HERE=$(cd "$(dirname "$0")" && pwd)
H=/tmp/sc-demo-home; rm -rf $H; mkdir -p $H
ln -s ~/.claude $H/.claude; [ -f ~/.claude.json ] && cp ~/.claude.json $H/.claude.json
export XDG_RUNTIME_DIR=/run/user/$(id -u)
SOCK=sc-demo-$$
TOKEN=$(openssl rand -hex 32)
PORT=9491
XD=:21
dbus-run-session -- bash -c "
  gnome-shell --headless --wayland --no-x11 --virtual-monitor 1920x1112 --wayland-display=$SOCK > $OUT/shell.log 2>&1 &
  SHELL_PID=\$!
  for i in \$(seq 1 60); do [ -S $XDG_RUNTIME_DIR/$SOCK ] && break; sleep 0.5; done
  sleep 3
  WAYLAND_DISPLAY=$SOCK Xwayland $XD -geometry 1920x1080 -ac -noreset > $OUT/xwayland.log 2>&1 &
  XW_PID=\$!
  for i in \$(seq 1 40); do [ -S /tmp/.X11-unix/X${XD#:} ] && break; sleep 0.5; done
  sleep 2
  env -u WAYLAND_DISPLAY DISPLAY=$XD HOME=$H XDG_CONFIG_HOME=$H/.config XDG_DATA_HOME=$H/.local/share PATH=$HOME/.npm-global/bin:$HOME/.local/bin:\$PATH \
    SOLIDCRAFT_CONTROL_TOKEN=$TOKEN SOLIDCRAFT_DEMO_KEYS=1 RUST_LOG=info \
    $APP --control $PORT > $OUT/app.log 2>&1 &
  APP_PID=\$!
  for i in \$(seq 1 120); do (echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null && break; sleep 0.5; done
  sleep 4
  DISPLAY=$XD python3 $HERE/${DRIVER:-next_step_demo.py} $PORT $TOKEN $OUT $AGENT $PLAN > $OUT/driver.log 2>&1
  echo driver-exit \$? >> $OUT/driver.log
  kill \$APP_PID; sleep 2; kill \$XW_PID; sleep 1; kill -9 \$XW_PID 2>/dev/null; kill -9 \$SHELL_PID
"
