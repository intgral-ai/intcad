#!/usr/bin/env bash
# Phase 1 input checks on a real X11 seat (XWayland on niri, or an Xorg
# session): clipboard both ways including Chinese, focus handover between page
# and egui, wheel, `<select>` popup, right-click context menu.
#
#   nix shell nixpkgs#xdotool nixpkgs#xclip -c ./scripts/panel-x11-input.sh [evidence-dir]
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=${PANEL_BIN:-$root/target/release/cef-osr-panel}
wrapper=${CEF_ENV:-/tmp/cef-osr-runtime/bin/cef-osr-env}
cef=${CEF_PATH:-/tmp/cef-root/154.0.34/cef_linux_x86_64}
out=${1:-$root/evidence/$(date -u +%Y%m%dT%H%M%SZ)/panel-x11}
mkdir -p "$out"

export CEF_PATH=$cef
export DISPLAY=${DISPLAY:-:0}
unset WAYLAND_DISPLAY
export XDG_SESSION_TYPE=x11
log=$out/panel.log

env PANEL_TRACE=1 PANEL_SWITCHES=ozone-platform=x11,use-angle=gl-egl \
  "$wrapper" "$bin" >"$log" 2>&1 &
panel=$!
cleanup() { kill $panel 2>/dev/null; pkill -x xclip 2>/dev/null; }
trap cleanup EXIT
sleep 6

wid=$(xdotool search --name "CEF OSR panel" | head -1)
if [ -z "$wid" ]; then
  echo "no panel window" >&2
  exit 1
fi
xdotool windowfocus "$wid"
sleep 0.4
eval "$(xdotool getwindowgeometry --shell "$wid")"
echo "window=$wid focused=$(xdotool getwindowfocus)" | tee "$out/env.txt"

# Where the page really is: the panel logs the egui rect and the scale factor,
# so the script does not guess the sidebar width or the DPI.
sleep 0.5
page=$(grep -o 'PAGE min_x=[0-9.]* min_y=[0-9.]* ppp=[0-9.]*' "$log" | tail -1)
px=$(sed -E 's/.*min_x=([0-9.]+).*/\1/' <<<"$page")
py=$(sed -E 's/.*min_y=([0-9.]+).*/\1/' <<<"$page")
ppp=$(sed -E 's/.*ppp=([0-9.]+).*/\1/' <<<"$page")
origin_x=$(python3 -c "print(round($X + $px * $ppp))")
origin_y=$(python3 -c "print(round($Y + $py * $ppp))")
input_x=$(python3 -c "print(round($origin_x + 150 * $ppp))")
input_y=$(python3 -c "print(round($origin_y + 98 * $ppp))")
select_x=$(python3 -c "print(round($origin_x + 430 * $ppp))")
select_y=$input_y
page_x=$((origin_x + 300))
page_y=$((origin_y + 300))

checks=$out/checks.txt
: >"$checks"
result() { printf '%s\n' "$*" | tee -a "$checks"; }
result "page origin $origin_x,$origin_y input $input_x,$input_y select $select_x,$select_y scale $ppp"

page_input() { xdotool mousemove "$input_x" "$input_y" click 1; sleep 0.5; }

# --- copy out: ASCII typed, selected, copied, read from the X clipboard -----
printf '' | xclip -selection clipboard >/dev/null 2>&1 &
sleep 0.5
page_input
xdotool type --delay 40 "clipboard-test"
sleep 0.7
xdotool key --clearmodifiers ctrl+a
sleep 0.3
xdotool key --clearmodifiers ctrl+c
sleep 0.8
copied=$(timeout 5 xclip -o -selection clipboard 2>/dev/null || true)
result "copy_event: $(grep -c 'copy event' "$log")"
if [ "$copied" = "clipboard-test" ]; then
  result "copy_out: PASS ($copied)"
else
  result "copy_out: FAIL (got '${copied:-<empty>}')"
fi

# --- Chinese in: system clipboard -> page -----------------------------------
printf '你好，世界' | xclip -selection clipboard >/dev/null 2>&1 &
sleep 0.6
page_input
xdotool key --clearmodifiers ctrl+a
sleep 0.2
xdotool key --clearmodifiers ctrl+v
sleep 1.0
if grep -q '你好，世界' "$log"; then
  result "paste_cn: PASS"
else
  result "paste_cn: FAIL"
fi

# --- focus: page -> CAD (sidebar) -> page -----------------------------------
page_input
xdotool type --delay 40 "abc"
sleep 0.5
before=$(grep -c 'key z' "$log" || true)
xdotool mousemove 120 700 click 1
sleep 0.5
xdotool type --delay 40 "zzz"
sleep 0.7
after=$(grep -c 'key z' "$log" || true)
if [ "$before" = "$after" ]; then
  result "focus_cad_click_stops_page_keys: PASS"
else
  result "focus_cad_click_stops_page_keys: FAIL ($before -> $after)"
fi
page_input
xdotool type --delay 40 "ok"
sleep 0.6
if grep -q 'key o' "$log"; then
  result "focus_return_to_page: PASS"
else
  result "focus_return_to_page: FAIL"
fi

# --- wheel ------------------------------------------------------------------
xdotool mousemove "$page_x" "$page_y"
xdotool click 4; xdotool click 4
sleep 0.7
result "wheel: $(grep -c 'wheel -' "$log") page wheel event(s)"

# --- <select> popup (before any context menu, which would capture the next clicks)
xdotool key Escape; sleep 0.3
xdotool mousemove "$page_x" "$page_y" click 1; sleep 0.3
xdotool mousemove "$select_x" "$select_y" click 1
sleep 1.2
result "select_popup: $(grep -c '^POPUP rect' "$log") frame(s) with a popup surface"
xdotool key Escape
sleep 0.4

# --- right-click context menu ----------------------------------------------
xdotool mousemove "$page_x" "$page_y" click 3
sleep 1.0
result "context_menu: $(grep -c 'ContextMenuRequested' "$log") request(s)"
xdotool key Escape
sleep 0.3

result "trace tail:"
grep -E "^NAV|^INPUT|^QUEUE|^POPUP" "$log" | tail -14 | tee -a "$checks"
echo "evidence: $out"
