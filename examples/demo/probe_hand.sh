#!/bin/bash
# probe_hand.sh APP OUT: a throwaway headless session (display :23) running probe_hand.py.
set -u
APP=$1; OUT=$2; HERE=$(cd "$(dirname "$0")" && pwd)
rm -rf "$OUT"; mkdir -p "$OUT"
H=/tmp/sc-probe-home; rm -rf $H; mkdir -p $H
export XDG_RUNTIME_DIR=/run/user/$(id -u)
SOCK=sc-probe-$$; TOKEN=$(openssl rand -hex 32); PORT=9495; XD=:23
dbus-run-session -- bash -c "
  gnome-shell --headless --wayland --no-x11 --virtual-monitor 1920x1112 --wayland-display=$SOCK > $OUT/shell.log 2>&1 &
  S=\$!; for i in \$(seq 1 60); do [ -S $XDG_RUNTIME_DIR/$SOCK ] && break; sleep 0.5; done; sleep 3
  WAYLAND_DISPLAY=$SOCK Xwayland $XD -geometry 1920x1080 -ac -noreset > $OUT/xw.log 2>&1 &
  X=\$!; for i in \$(seq 1 40); do [ -S /tmp/.X11-unix/X23 ] && break; sleep 0.5; done; sleep 2
  env -u WAYLAND_DISPLAY DISPLAY=$XD HOME=$H XDG_CONFIG_HOME=$H/.config XDG_DATA_HOME=$H/.local/share SOLIDCRAFT_CONTROL_TOKEN=$TOKEN SOLIDCRAFT_DEMO_KEYS=1 \
    $APP --control $PORT > $OUT/app.log 2>&1 &
  A=\$!; for i in \$(seq 1 120); do (echo > /dev/tcp/127.0.0.1/$PORT) 2>/dev/null && break; sleep 0.5; done; sleep 4
  DISPLAY=$XD timeout 400 python3 $HERE/probe_hand.py $PORT $TOKEN $OUT > $OUT/probe.log 2>&1
  kill \$A; sleep 2; kill \$X; sleep 1; kill -9 \$X \$S 2>/dev/null
"
