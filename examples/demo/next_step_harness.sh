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

wait_for_control() {
  local pid=$1 port=$2
  # A listening port alone can belong to a different instance. Authenticate with this run's
  # token and wait for the UI to answer a read-only request. Never print the token or reply.
  PROBE_TOKEN=$3 python3 - "$pid" "$port" <<'PY'
import json
import os
import socket
import sys
import time

pid, port = map(int, sys.argv[1:])
request = {"id": 1, "method": "ui.inspect", "params": {}, "token": os.environ["PROBE_TOKEN"]}
wire = (json.dumps(request) + "\n").encode()
deadline = time.monotonic() + 30
while time.monotonic() < deadline:
    try:
        os.kill(pid, 0)
    except OSError:
        sys.exit("Intcad exited before its control channel was ready")
    attempt = min(deadline, time.monotonic() + 1)
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=max(0, attempt - time.monotonic())) as conn:
            conn.sendall(wire)
            reply = bytearray()
            while len(reply) < 1024 * 1024 and time.monotonic() < attempt:
                conn.settimeout(max(0, attempt - time.monotonic()))
                chunk = conn.recv(8192)
                if not chunk:
                    break
                reply.extend(chunk)
                if b"\n" in chunk:
                    data = json.loads(reply.partition(b"\n")[0])
                    if isinstance(data, dict) and data.get("ok") is False and str(data.get("error", "")).startswith("unauthorized"):
                        sys.exit("Control port refused this run's token")
                    if isinstance(data, dict) and data.get("id") == 1 and data.get("ok") is True and isinstance(data.get("result"), dict):
                        os.kill(pid, 0)
                        sys.exit(0)
                    break
    except (OSError, ValueError, RecursionError):
        pass
    time.sleep(max(0, min(0.25, deadline - time.monotonic())))
sys.exit("Timed out waiting for Intcad's authenticated control response")
PY
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
  wait_for_control "$!" "$port" "$token" || return
  DISPLAY=$display timeout "$timeout_s" python3 "$driver" "$port" "$token" "$out" "$@" > "$out/driver.log" 2>&1 || rc=$?
  printf 'driver-exit %s\n' "$rc" >> "$out/driver.log"
  return "$rc"
}
