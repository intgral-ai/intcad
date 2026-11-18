#!/usr/bin/env bash
# Phase 2: the Wayland/X11 matrix for CEF accelerated OSR.
#
# Each case runs `cef-osr-probe`, which creates **no window**, so this does not
# take the desktop's focus. What changes per case is only the display
# environment and Chromium's Ozone switch.
#
#   experiments/cef-osr/scripts/matrix.sh [evidence-dir]
#
# Exit status is 0 even when cases fail: the point is the receipts, and the
# report reads them. `grep RECEIPT` over the logs is the evidence.
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
bin=${PROBE_BIN:-$root/target/release/cef-osr-probe}
wrapper=${CEF_ENV:-/tmp/cef-osr-runtime/bin/cef-osr-env}
cef=${CEF_PATH:-/tmp/cef-root/154.0.34/cef_linux_x86_64}
out=${1:-$root/evidence/$(date -u +%Y%m%dT%H%M%SZ)}
mkdir -p "$out"
wayland=${WAYLAND_DISPLAY:-wayland-1}

export CEF_PATH=$cef
export PROBE_SECS=${PROBE_SECS:-8}
export PROBE_EXPECT=${PROBE_EXPECT:-1e90ff}

run() {
  local label=$1
  shift
  echo "--- $label"
  timeout 150 env PROBE_DUMP="$out/$label.ppm" "$wrapper" "$@" >"$out/$label.log" 2>&1
  local code=$?
  echo "exit=$code" >>"$out/$label.log"
  grep -h '"tag":"frames"\|"tag":"result"\|"tag":"native_frame"' "$out/$label.log" | sed "s/^/$label /"
}

# A. X11/XWayland baseline (what production does today).
run 01-x11-wayland-unset \
  env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET DISPLAY=:0 XDG_SESSION_TYPE=x11 "$bin"

# B. Native Wayland, Ozone left to choose (session variables intact).
run 02-wayland-auto \
  env -u DISPLAY WAYLAND_DISPLAY="$wayland" XDG_SESSION_TYPE=wayland "$bin"

# C. Native Wayland, explicitly asked for.
run 03-wayland-explicit \
  env -u DISPLAY WAYLAND_DISPLAY="$wayland" XDG_SESSION_TYPE=wayland \
  PROBE_SWITCHES=ozone-platform=wayland "$bin"

# D. No display connection at all, Ozone headless (surfaceless GPU).
run 04-headless-ozone \
  env -u DISPLAY -u WAYLAND_DISPLAY XDG_SESSION_TYPE=wayland \
  PROBE_SWITCHES=ozone-platform=headless "$bin"

# E. No display connection at all, Ozone left to choose.
run 05-no-display-auto \
  env -u DISPLAY -u WAYLAND_DISPLAY XDG_SESSION_TYPE=wayland "$bin"

# F. ANGLE/Vulkan spellings older CEF documentation recommends, native Wayland.
run 06-wayland-angle-vulkan \
  env -u DISPLAY WAYLAND_DISPLAY="$wayland" XDG_SESSION_TYPE=wayland \
  PROBE_SWITCHES=ozone-platform=wayland,use-angle=vulkan,enable-features=Vulkan "$bin"

# G. The switch CEF's own fix names (issue #3953): ANGLE on EGL. Wayland Ozone.
run 07-wayland-gl-egl \
  env -u DISPLAY WAYLAND_DISPLAY="$wayland" XDG_SESSION_TYPE=wayland \
  PROBE_SWITCHES=ozone-platform=wayland,use-angle=gl-egl "$bin"

# H. ANGLE/EGL with no display connection at all.
run 08-headless-gl-egl \
  env -u DISPLAY -u WAYLAND_DISPLAY XDG_SESSION_TYPE=wayland \
  PROBE_SWITCHES=ozone-platform=headless,use-angle=gl-egl "$bin"

# I. X11 Ozone under a Wayland-only environment: the control that shows whether
#    a failing case is really an X11 dependency.
run 09-wayland-but-x11-ozone \
  env -u DISPLAY WAYLAND_DISPLAY="$wayland" XDG_SESSION_TYPE=wayland \
  PROBE_SWITCHES=ozone-platform=x11,use-angle=gl-egl "$bin"

# J. Same as G with Chromium's own GPU logging, for diagnosis rather than a claim.
run 10-wayland-gl-egl-verbose \
  env -u DISPLAY WAYLAND_DISPLAY="$wayland" XDG_SESSION_TYPE=wayland \
  PROBE_SWITCHES=ozone-platform=wayland,use-angle=gl-egl,enable-logging=stderr,v=1 "$bin"

# K. The CEF-recommended Linux accelerated-OSR spelling on X11.
run 11-x11-gl-egl \
  env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET DISPLAY=:0 XDG_SESSION_TYPE=x11 \
  PROBE_SWITCHES=ozone-platform=x11,use-angle=gl-egl "$bin"

echo "evidence: $out"
