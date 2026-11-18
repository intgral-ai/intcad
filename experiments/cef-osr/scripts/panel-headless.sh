#!/usr/bin/env bash
# Phase 3: the egui + CEF panel inside one window, on a *headless* Wayland
# compositor, with a real screenshot.
#
# Why headless: the production app is a desktop window, and a benchmark that
# steals the developer's focus is a bad benchmark. Sway's headless backend is a
# genuine wlroots compositor with EGL on the render node, so the window, the
# Vulkan swapchain, the texture import and the compositing are all real; only
# the output goes nowhere, and `grim` reads it back through wlr-screencopy.
#
#   experiments/cef-osr/scripts/panel-headless.sh [evidence-dir] [seconds]
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=${PANEL_BIN:-$root/target/release/cef-osr-panel}
wrapper=${CEF_ENV:-/tmp/cef-osr-runtime/bin/cef-osr-env}
cef=${CEF_PATH:-/tmp/cef-root/154.0.34/cef_linux_x86_64}
out=${1:-$root/evidence/$(date -u +%Y%m%dT%H%M%SZ)}
seconds=${2:-8}
mkdir -p "$out"

export CEF_PATH=$cef
export XDG_RUNTIME_DIR=/tmp/xdg-sway-cef
mkdir -p "$XDG_RUNTIME_DIR"
chmod 700 "$XDG_RUNTIME_DIR"
rm -f "$XDG_RUNTIME_DIR"/wayland-* "$XDG_RUNTIME_DIR"/sway-ipc.*

cat >"$XDG_RUNTIME_DIR/config" <<'EOF'
output HEADLESS-1 resolution 1400x900
default_border none
default_floating_border none
focus_follows_mouse no
EOF

WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=gles2 \
  sway -c "$XDG_RUNTIME_DIR/config" >"$out/sway.log" 2>&1 &
sway_pid=$!
for _ in $(seq 1 80); do
  socket=$(ls "$XDG_RUNTIME_DIR"/wayland-* 2>/dev/null | grep -v '\.lock' | head -1)
  [ -n "${socket:-}" ] && break
  sleep 0.25
done
if [ -z "${socket:-}" ]; then
  echo "sway did not start; see $out/sway.log" >&2
  kill $sway_pid 2>/dev/null
  exit 1
fi
export WAYLAND_DISPLAY=$(basename "$socket")
export SWAYSOCK=$(ls "$XDG_RUNTIME_DIR" | grep '^sway-ipc' | sed "s|^|$XDG_RUNTIME_DIR/|" | head -1)
unset DISPLAY
echo "compositor: $WAYLAND_DISPLAY ($(swaymsg -t get_version 2>/dev/null | head -3 | tr '\n' ' '))" | tee "$out/env.txt"

env -u DISPLAY PANEL_URL="${PANEL_URL:-file://$root/poc/fixtures/probe.html}" \
  PANEL_SWITCHES="${PANEL_SWITCHES:-ozone-platform=wayland,use-angle=gl-egl}" \
  "$wrapper" "$bin" >"$out/panel.log" 2>&1 &
panel_pid=$!

sleep "$seconds"
grim "$out/panel-initial.png" 2>>"$out/grim.log"
python3 "$root/scripts/tree_cpu.py" $panel_pid 12 panel-idle >>"$out/cpu.jsonl" 2>/dev/null
grim "$out/panel-idle.png" 2>>"$out/grim.log"

# A drag across the page, a wheel tick and typing, so the screenshot shows what
# the page did with the input the host forwarded.
wlrctl pointer move 700 500 2>>"$out/input.log"
wlrctl pointer click left 2>>"$out/input.log"
wlrctl pointer move 760 520 2>>"$out/input.log"
wlrctl pointer click left 2>>"$out/input.log"
wlrctl pointer axis vertical 6 2>>"$out/input.log" || true
wtype 'hello cef' 2>>"$out/input.log" || true
sleep 2
grim "$out/panel-input.png" 2>>"$out/grim.log"

# Resize the window: the panel must resize CEF, not stretch the last frame.
# A tiled container ignores `resize set`; float it first so the window really
# changes size and the panel has to resize CEF.
swaymsg -q '[title="CEF OSR panel"] floating enable' 2>>"$out/input.log" || true
sleep 0.5
swaymsg -q '[title="CEF OSR panel"] resize set 900 620' 2>>"$out/input.log" || true
sleep 2
swaymsg -t get_tree 2>/dev/null | grep -o '"width": [0-9]*, "height": [0-9]*' | tail -2 >>"$out/input.log"
sleep 2
grim "$out/panel-resized.png" 2>>"$out/grim.log"

kill $panel_pid 2>/dev/null
sleep 1
swaymsg exit >/dev/null 2>&1
kill $sway_pid 2>/dev/null
echo "evidence: $out"
grep -h "STATS" "$out/panel.log" | tail -5
