# Shared by the next-step demo launchers. Source this in Bash.

fresh_output() {
  # Refuse existing paths, including symlinks. Never remove a caller's output.
  mkdir -- "$1" && (cd -P -- "$1" && pwd)
}

stop_demo_children() {
  local pid
  for pid in "$@"; do
    kill "$pid" 2>/dev/null || :
  done
  for pid in "$@"; do
    wait "$pid" 2>/dev/null || :
  done
}

wait_for() {
  local pid=$1 attempts=$2
  shift 2
  while ((attempts-- > 0)); do
    kill -0 "$pid" 2>/dev/null || return 1
    "$@" && return 0
    sleep 0.5
  done
  printf 'Timed out waiting for %s\n' "$1" >&2
  return 1
}

control_ready() {
  ( : > "/dev/tcp/127.0.0.1/$1" ) 2>/dev/null
}

# Xwayland selects its own free display; the caller owns the app's process and settings.
run_recording() {
  local app=$1 out=$2 home=$3 port=$4 driver=$5 timeout_s=$6
  shift 6
  local sock=sc-demo-$$ token display rc=0
  token=$(openssl rand -hex 32) || return
  pids=()
  trap 'stop_demo_children "${pids[@]}"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  gnome-shell --headless --wayland --no-x11 --virtual-monitor 1920x1112 --wayland-display="$sock" > "$out/shell.log" 2>&1 &
  pids+=("$!")
  wait_for "$!" 60 test -S "$XDG_RUNTIME_DIR/$sock" || return
  WAYLAND_DISPLAY=$sock Xwayland -displayfd 3 -geometry 1920x1080 -ac -noreset 3>"$out/display" > "$out/xwayland.log" 2>&1 &
  pids+=("$!")
  wait_for "$!" 40 test -s "$out/display" || return
  read -r display < "$out/display" || return
  [[ $display =~ ^[0-9]+$ ]] || return 1
  display=:$display
  env -u WAYLAND_DISPLAY DISPLAY="$display" HOME="$home" XDG_CONFIG_HOME="$home/.config" XDG_DATA_HOME="$home/.local/share" \
    SOLIDCRAFT_SETTINGS_DIR="$home/settings" SOLIDCRAFT_CONTROL_TOKEN="$token" SOLIDCRAFT_DEMO_KEYS=1 RUST_LOG=info \
    "$app" --control "$port" > "$out/app.log" 2>&1 &
  pids+=("$!")
  wait_for "$!" 120 control_ready "$port" || return
  DISPLAY=$display timeout "$timeout_s" python3 "$driver" "$port" "$token" "$out" "$@" > "$out/driver.log" 2>&1 || rc=$?
  printf 'driver-exit %s\n' "$rc" >> "$out/driver.log"
  return "$rc"
}
