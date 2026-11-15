#!/usr/bin/env bash
# The parts of installed-check.sh that run inside the bare system, as root. They install
# packages: never run this on a person's machine.
#
#   installed-check-inside.sh install /dist/<package> x11|wayland    (with the network)
#   installed-check-inside.sh run x11|wayland                        (without it)
#
# install: the package goes on with nothing but what it declares (no recommended packages), and
#   the app's file must then find every library it names. An AppImage brings no system
#   libraries, so there the launcher must say which are missing, and they are installed. Then
#   the test's own tools go on: a display, a software GPU, the checking script's Python.
# run: the installed app is started and packaging/assistant-check.py opens the Assistant. This
#   system has no network by then, and the step says so before it starts the app.
set -euo pipefail
STEP="$1"
# Where `install` tells `run` which program to start.
PROGRAM=/installed-check-program

lacking() { ldd "$1" 2>/dev/null | awk '/not found/ { print $1 }' | sort -u | tr '\n' ' '; }

install_step() {
  local package="$1" session="$2" exe missing said
  if command -v apt-get >/dev/null; then
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    bare() { apt-get install -y -qq --no-install-recommends "$@" >/dev/null; }
    # What the .deb recommends (the GPU's userspace), and the test's tools.
    local display_tools=(xvfb openbox python3 imagemagick dbus-x11 fonts-dejavu-core mesa-vulkan-drivers libegl-mesa0 libgl1-mesa-dri)
    local wayland_tools=(sway xwayland grim)
    # What an AppImage needs from the system: the web view's libraries (GTK comes with them)
    # and the windowing libraries the app loads by name.
    local appimage_needs=(libwebkit2gtk-4.1-0 libxkbcommon-x11-0 libxcursor1 libxi6 libx11-xcb1 libwayland-client0 libvulkan1)
  elif command -v dnf >/dev/null; then
    bare() { dnf install -y -q --setopt=install_weak_deps=False "$@" >/dev/null; }
    local display_tools=(xorg-x11-server-Xvfb openbox python3 ImageMagick dbus-x11 dejavu-sans-fonts mesa-vulkan-drivers mesa-dri-drivers)
    local wayland_tools=(sway xorg-x11-server-Xwayland grim)
    local appimage_needs=(webkit2gtk4.1 libxkbcommon-x11 libXcursor libXi libwayland-client vulkan-loader)
  else
    echo "error: neither apt-get nor dnf in this image" >&2
    exit 2
  fi

  case "$package" in
    *.deb | *.rpm)
      bare "$package"
      exe=/usr/bin/solidcraft
      missing="$(lacking "$exe")"
      if [ -n "$missing" ]; then
        echo "FAIL: the package does not bring these libraries the app names: $missing"
        exit 1
      fi
      echo "ok: installed with only what the package declares, the app finds every library it names"
      solidcraft-cli --version
      ;;
    *.AppImage)
      # Unpacked, as the AppImage runtime does where there is no FUSE (containers, CI).
      install -m755 "$package" /opt/app.AppImage
      (cd /opt && ./app.AppImage --appimage-extract >/dev/null)
      exe=/opt/squashfs-root/AppRun
      missing="$(lacking /opt/squashfs-root/usr/bin/solidcraft)"
      if [ -n "$missing" ]; then
        said="$("$exe" --version 2>&1 >/dev/null || true)"
        if ! printf '%s' "$said" | grep -q 'sudo apt install libwebkit2gtk-4.1-0'; then
          echo "FAIL: on a system without the web view's libraries the launcher did not say so; it said:"
          printf '%s\n' "$said" | head -8
          exit 1
        fi
        echo "ok: on a system without them the launcher names the missing libraries and how to get them"
        bare "${appimage_needs[@]}"
      fi
      missing="$(lacking /opt/squashfs-root/usr/bin/solidcraft)"
      if [ -n "$missing" ]; then
        echo "FAIL: with the package the launcher names installed, these are still missing: $missing"
        exit 1
      fi
      /opt/squashfs-root/usr/bin/solidcraft-cli --version
      ;;
    *)
      echo "error: not a .deb, .rpm or .AppImage: $package" >&2
      exit 2
      ;;
  esac
  printf '%s\n' "$exe" >"$PROGRAM"

  echo "--- the test's own tools (a display, a software GPU, Python)"
  bare "${display_tools[@]}"
  if [ "$session" = wayland ]; then
    bare "${wayland_tools[@]}"
    # sway does not run as root.
    id check >/dev/null 2>&1 || useradd -m check
  fi
}

run_step() {
  local session="$1" exe
  exe="$(cat "$PROGRAM")"
  if python3 -c 'import socket; socket.create_connection(("1.1.1.1", 443), timeout=4)' 2>/dev/null; then
    echo "FAIL: this system reaches the network; the installed app is not started with it open"
    exit 1
  fi
  echo "ok: this system has no network"
  chmod 777 /evidence
  local check=(python3 /packaging/assistant-check.py --exe "$exe" --evidence /evidence --screen)

  if [ "$session" = x11 ]; then
    export DISPLAY=:99
    Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp >/evidence/xvfb.log 2>&1 &
    for _ in $(seq 1 40); do [ -e /tmp/.X11-unix/X99 ] && break; sleep 0.25; done
    openbox >/evidence/openbox.log 2>&1 &
    sleep 0.5
    HOME=/tmp/home-check "${check[@]}"
    return
  fi

  cat >/tmp/session.sh <<'SESSION'
set -euo pipefail
export XDG_RUNTIME_DIR=/tmp/xdg-check HOME=/tmp/home-check
mkdir -p "$XDG_RUNTIME_DIR" "$HOME"
chmod 700 "$XDG_RUNTIME_DIR"
export WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman
export XDG_CURRENT_DESKTOP=sway XDG_SESSION_TYPE=wayland
cat >"$HOME/sway.conf" <<'SWAY'
output HEADLESS-1 resolution 1600x1000
xwayland enable
default_border none
default_floating_border none
for_window [title=".*"] floating enable, move position 0 0
SWAY
unset DISPLAY WAYLAND_DISPLAY
sway -c "$HOME/sway.conf" >/evidence/sway.log 2>&1 &
for _ in $(seq 1 80); do ls "$XDG_RUNTIME_DIR"/sway-ipc.* >/dev/null 2>&1 && break; sleep 0.25; done
SWAYSOCK="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'sway-ipc.*' | head -1)"
WAYLAND_DISPLAY="$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'wayland-[0-9]*' ! -name '*.lock' -printf '%f\n' | head -1)"
if [ -z "$SWAYSOCK" ] || [ -z "$WAYLAND_DISPLAY" ]; then
  echo "FAIL: the Wayland session did not start"
  tail -20 /evidence/sway.log
  exit 1
fi
export SWAYSOCK WAYLAND_DISPLAY
# The X display sway gives its children (XWayland): ask a child.
swaymsg -q exec "sh -c 'echo \$DISPLAY >$HOME/display'"
for _ in $(seq 1 40); do [ -s "$HOME/display" ] && break; sleep 0.25; done
DISPLAY="$(cat "$HOME/display" 2>/dev/null || true)"
if [ -z "$DISPLAY" ]; then
  echo "FAIL: the Wayland session has no XWayland display"
  exit 1
fi
export DISPLAY
echo "session: WAYLAND_DISPLAY=$WAYLAND_DISPLAY DISPLAY=$DISPLAY $(sway --version 2>&1 | head -1)"
exec "$@"
SESSION
  su check -s /bin/bash -c "bash /tmp/session.sh $(printf '%q ' "${check[@]}")"
}

case "$STEP" in
  install) install_step "$2" "$3" ;;
  run) run_step "$2" ;;
  *) echo "unknown step: $STEP" >&2; exit 2 ;;
esac
