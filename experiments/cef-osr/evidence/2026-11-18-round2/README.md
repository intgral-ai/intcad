# 2026-11-18 round 2 evidence

| File | What it shows |
|---|---|
| `soak2-100.log`, `soak2-100.jsonl` | 100 create/destroy cycles on native Wayland: 100 first frames, 0 failures, RSS 280.2→282.2 MB, fds 119→116, VRAM 21 MiB flat; first-frame queue replay (`queue_typed=true`) with the pre-frame control keystroke dropped (`queue_control_X_delivered=false`); renderer crash `terminated_event=true, recovered_frame=true` |
| `soak-sandboxed.log` | The same cycle with CEF's process sandbox (`SOAK_SANDBOX=sandboxed`): 10/10, 0 failures |
| `panel-x11/panel.log`, `checks.txt` | Real X11 seat (XWayland on niri): copy out `clipboard-test`, Chinese paste from the system clipboard, focus handover both ways, wheel, `<select>` popup, context-menu request |
| `ime-seq-x11.log` | IME composition: `zhong` → cancel (`compositionend ""`), `ni hao` → `你好` commit, then continuous `世界` → `你好世界` |
| `no-dmabuf.log` | Device built without the interop extensions: typed `VulkanForeignQueueExtensionNotEnabled`, `imported=0`, exit 1, no crash, no CPU fallback |
| `sizes.txt` | CEF distribution / `libcef.so` before and after strip / archive / locales |

Round-one evidence stays in `../2026-11-18/` (Wayland vs X11 matrix, GPU imports,
first-round screenshots and idle measurements).
