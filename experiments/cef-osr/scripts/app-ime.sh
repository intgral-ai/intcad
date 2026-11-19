#!/usr/bin/env bash
# In-app Chinese input through the system input method (fcitx5) into the CEF Assistant.
#
# fcitx5 draws its own candidate window and sends the client an empty preedit plus the
# committed text (the matrix egui-winit documents for Linux + fcitx5 shuangpin), so the
# evidence here is the committed Chinese and the dismissed composition, not the candidate
# window, which belongs to the input method rather than the app.
#
# X11/XWayland, because winit drives the input method through XIM there and xdotool
# can type into an X client. The app must be the cutover build (CEF, no WRY).
#
#   nix shell nixpkgs#xdotool nixpkgs#xclip -c ./scripts/app-ime.sh [evidence-dir]
set -uo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
exe=${APP_BIN:-$root/../../../target/release/intcad}
[ -x "$exe" ] || exe=$(cd "$root/../../.." && pwd)/target/release/intcad
wrapper=${CEF_ENV:-/tmp/cef-osr-runtime/bin/cef-osr-env}
cef=${CEF_PATH:-/tmp/cef-root/154.0.34/cef_linux_x86_64}
out=${1:-$root/evidence/$(date -u +%Y%m%dT%H%M%SZ)/ime}
port=${IME_PORT:-18120}
work=$out/app
mkdir -p "$work"/{settings,validation,crash,control,recovery} "$out"

export CEF_PATH=$cef DISPLAY=${DISPLAY:-:0} XDG_SESSION_TYPE=x11 SOLIDCRAFT_WAYLAND=0
export XMODIFIERS=${XMODIFIERS:-@im=fcitx} GTK_IM_MODULE=fcitx QT_IM_MODULE=fcitx
export SOLIDCRAFT_ASSISTANT_CEF_SWITCHES=ozone-platform=x11,use-angle=gl-egl
export SOLIDCRAFT_SETTINGS_DIR=$work/settings SOLIDCRAFT_VALIDATION_DIR=$work/validation
export SOLIDCRAFT_CRASH_DIR=$work/crash SOLIDCRAFT_CONTROL_DIR=$work/control
export SOLIDCRAFT_RECOVERY_DIR=$work/recovery SOLIDCRAFT_UPDATE_CHECK=0
export SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE=1
[ -n "${IME_RELOAD_AFTER:-}" ] && export SOLIDCRAFT_ASSISTANT_CEF_TEST_RELOAD_AFTER=$IME_RELOAD_AFTER

"$wrapper" "$exe" --control "$port" >"$out/app.log" 2>&1 &
app=$!
trap 'kill $app 2>/dev/null; pkill -x intcad 2>/dev/null' EXIT
sleep 7
cp "$work/control/$port.token" /tmp/intcad-shot-token 2>/dev/null || true

before=$(fcitx5-remote -n 2>/dev/null || echo unknown)
# Prefer plain pinyin for a predictable candidate; restore the session's method at the end.
fcitx5-remote -s pinyin >/dev/null 2>&1 || true
fcitx5-remote -o >/dev/null 2>&1 || true
echo "ime before=$before now=$(fcitx5-remote -n 2>/dev/null)" | tee "$out/ime.txt"

python3 - "$port" <<'PY' | tee -a "$out/ime.txt"
import json, socket, subprocess, sys, time
port=18120 if len(sys.argv)<2 else int(sys.argv[1])
tok=open('/tmp/intcad-shot-token').read().strip()
s=socket.create_connection(("127.0.0.1",port),timeout=5); s.settimeout(60)
f=s.makefile("rw",encoding="utf-8",newline="\n"); n=0
def call(m,p=None):
    global n; n+=1
    r={"id":n,"method":m,"params":p or {}}
    if n==1: r["token"]=tok
    f.write(json.dumps(r)+"\n"); f.flush(); return json.loads(f.readline())
call("ui.agent_chat",{"open":True}); time.sleep(2)
rects=call("ui.rects",{"prefix":"agent_chat:"})["result"]; ppp=rects["pixelsPerPoint"]
web=[r for r in rects["rects"] if r["name"]=="agent_chat:web"][0]
wid=subprocess.run(["xdotool","search","--name","Intcad"],capture_output=True,text=True).stdout.split()[0]
g={}
for line in subprocess.run(["xdotool","getwindowgeometry","--shell",wid],capture_output=True,text=True).stdout.splitlines():
    if "=" in line: k,v=line.split("=",1); g[k]=v
ox,oy=int(g["X"]),int(g["Y"])
cx=round(ox+(web["x"]+web["width"]*0.45)*ppp); cy=round(oy+(web["y"]+web["height"]*0.885)*ppp)
subprocess.run(["xdotool","windowactivate","--sync",wid],stderr=subprocess.DEVNULL); time.sleep(0.3)
subprocess.run(["xdotool","mousemove",str(cx),str(cy),"click","1"]); time.sleep(0.8)
print(json.dumps({"clicked":[cx,cy],"web":web}))
PY

sleep 0.5
# Composition: preedit should show while typing, before any commit.
xdotool type --delay 180 "nihao"
sleep 1.5
niri msg action screenshot-window >/dev/null 2>&1 || true
sleep 0.4
# Commit the first candidate.
xdotool key space
sleep 1.2
# What the page now holds, through the app's clipboard bridge.
xdotool key --clearmodifiers ctrl+a; sleep 0.2
xdotool key --clearmodifiers ctrl+c; sleep 1.0
committed=$(timeout 10 xclip -o -selection clipboard 2>/dev/null || true)
echo "committed_from_page: ${committed:-<empty>}" | tee -a "$out/ime.txt"
case "$committed" in
  *[!\ -~]*) echo "ime_commit: PASS (non-ASCII text reached the page)" | tee -a "$out/ime.txt" ;;
  *) echo "ime_commit: FAIL (the page holds only Latin text)" | tee -a "$out/ime.txt" ;;
esac

# Cancel: type a new composition and dismiss it; the field must not change.
xdotool key --clearmodifiers End; sleep 0.2
xdotool type --delay 180 "zhong"
sleep 1.0
xdotool key Escape
sleep 0.8
xdotool key --clearmodifiers ctrl+a; sleep 0.2
xdotool key --clearmodifiers ctrl+c; sleep 1.0
after=$(timeout 10 xclip -o -selection clipboard 2>/dev/null || true)
echo "after_cancel_from_page: ${after:-<empty>}" | tee -a "$out/ime.txt"
if [ "$after" = "$committed" ]; then
  echo "ime_cancel: PASS (a dismissed composition added nothing)" | tee -a "$out/ime.txt"
else
  echo "ime_cancel: FAIL (the field changed)" | tee -a "$out/ime.txt"
fi

grep -E "IME composition|IME commit" "$out/app.log" | tail -12 | tee -a "$out/ime.txt"

# Pending/reload coverage is opt-in: the real page must not receive the old
# composition after a reload. Without a requested reload this is not run.
if [ -n "${IME_RELOAD_AFTER:-}" ]; then
  # Put a sentinel in the old page, then reload. If it survives into the new
  # page, pending input/old content leaked; if the reload never happens, BLOCKED.
  if grep -q "CEF: test reload" "$out/app.log"; then
    echo "ime_pending_during_reload: BLOCKED (reload happened before the sentinel was typed)" | tee -a "$out/ime.txt"
  else
  xdotool type --delay 40 "stale-probe"
  sleep 0.3
  for _ in $(seq 1 20); do
    grep -q "CEF: test reload" "$out/app.log" && break
    sleep 1
  done
  if grep -q "CEF: test reload" "$out/app.log"; then
    python3 - "$port" <<'PY' || true
import json, socket, subprocess, sys, time
port=int(sys.argv[1])
tok=open('/tmp/intcad-shot-token').read().strip()
s=socket.create_connection(("127.0.0.1",port),timeout=5); s.settimeout(60)
f=s.makefile("rw",encoding="utf-8",newline="\n"); n=0
def call(m,p=None):
    global n; n+=1
    r={"id":n,"method":m,"params":p or {}}
    if n==1: r["token"]=tok
    f.write(json.dumps(r)+"\n"); f.flush(); return json.loads(f.readline())
time.sleep(2)
rects=call("ui.rects",{"prefix":"agent_chat:"})["result"]; ppp=rects["pixelsPerPoint"]
web=[r for r in rects["rects"] if r["name"]=="agent_chat:web"][0]
wid=subprocess.run(["xdotool","search","--name","Intcad"],capture_output=True,text=True).stdout.split()[0]
g={}
for line in subprocess.run(["xdotool","getwindowgeometry","--shell",wid],capture_output=True,text=True).stdout.splitlines():
    if "=" in line: k,v=line.split("=",1); g[k]=v
ox,oy=int(g["X"]),int(g["Y"])
cx=round(ox+(web["x"]+web["width"]*0.45)*ppp); cy=round(oy+(web["y"]+web["height"]*0.885)*ppp)
subprocess.run(["xdotool","windowactivate","--sync",wid],stderr=subprocess.DEVNULL); time.sleep(0.3)
subprocess.run(["xdotool","mousemove",str(cx),str(cy),"click","1"]); time.sleep(0.6)
PY
    xdotool key --clearmodifiers ctrl+a; sleep 0.2
    xdotool key --clearmodifiers ctrl+c; sleep 1.0
    after_reload=$(timeout 10 xclip -o -selection clipboard 2>/dev/null || true)
    case "$after_reload" in
      *stale-probe*) echo "ime_pending_during_reload: FAIL (old content reached the reloaded page)" | tee -a "$out/ime.txt" ;;
      *) echo "ime_pending_during_reload: PASS (sentinel absent after bounded reload)" | tee -a "$out/ime.txt" ;;
    esac
  else
    echo "ime_pending_during_reload: BLOCKED (reload not observed within the bounded wait)" | tee -a "$out/ime.txt"
  fi
  fi
else
  echo "ime_pending_during_reload: NOT_RUN (set IME_RELOAD_AFTER)" | tee -a "$out/ime.txt"
fi
# The candidate window belongs to fcitx5, not the app; this script cannot prove it.
echo "ime_candidate_window: BLOCKED (owned by the input method; not observable from the app process)" | tee -a "$out/ime.txt"
grep -E "CEF_RUNTIME|CEF_BROWSER|CEF_FRAME" "$out/app.log" | tail -8 | tee -a "$out/ime.txt"
fcitx5-remote -s "$before" >/dev/null 2>&1 || true
echo "evidence: $out"
