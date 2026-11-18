# CEF accelerated OSR + welding: feasibility probe

Standalone, throwaway experiment. It does **not** touch the production Assistant
(`apps/solidcraft/src/assistant_web.rs`), the WRY/webview path or the Cargo
workspace: this crate has its own `[workspace]`, so `IntCAD`'s `Cargo.lock` is
unchanged.

Read the findings in [docs/cef-welding-feasibility.md](../../../docs/cef-welding-feasibility.md).
This file is only *how to run it again*.

## What is here

| Path | What it is |
|---|---|
| `poc/src/bin/probe.rs` | Headless probe: real CEF accelerated OSR → DMABUF → host wgpu texture, with no window at all. Prints `RECEIPT {...}` lines as evidence. |
| `poc/src/bin/panel.rs` | Minimal egui host: the same CEF texture drawn by `egui_wgpu` in one `eframe` window, with pointer/wheel/keyboard/IME/pop-up forwarding. |
| `poc/fixtures/probe.html` | Test page: solid `#1e90ff` background, an input, a `<select>`, and a log that mirrors every gesture into `document.title` (which comes back as a navigation event). |
| `nix/runtime.nix` | The CEF runtime libraries as one `cef-osr-env` wrapper (NixOS has no `/usr/lib`). |
| `scripts/matrix.sh` | The Wayland/X11/headless matrix. |
| `scripts/panel-headless.sh` | The panel on a headless sway (wlroots) compositor, with `grim` screenshots and a CPU sample. |
| `scripts/tree_cpu.py` | CPU of a process **and its descendants** (CEF renders in child processes). |
| `scripts/ppm2png.py` | PPM → PNG without third-party packages. |
| `evidence/2026-11-18/` | The logs and screenshots the report quotes. |

## Prerequisites

- Linux x86-64. Tested on NixOS 26.11, Mesa 26.3.0, Rust 1.98.1.
- CEF 154.0.34 Linux x64, as `CEF_PATH` (`libcef.so` at its root). The
  `cef-dll-sys` crate can download it itself:
  `https://cef-builds.spotifycdn.com/cef_binary_154.0.34%2Bg14c5a08%2Bchromium-154.0.8037.98_linux64_minimal.tar.bz2`.
- `cmake`, `ninja` and a C++ compiler for `cef-dll-sys` (it builds
  `libcef_dll_wrapper`).
- On NixOS: `nix build --impure --expr 'import ./nix/runtime.nix' -o /tmp/cef-osr-runtime`.

## Build

```sh
export CEF_PATH=/path/to/cef_binary_154.0.34+g14c5a08+chromium-154.0.8037.98_linux64
nix shell nixpkgs#cmake nixpkgs#ninja nixpkgs#gcc -c bash -lc 'cargo build --release'
```

## Run

Environment knobs are documented at the top of `probe.rs` and `panel.rs`
(`PROBE_*`, `PANEL_*`). The three that matter most:

- `PROBE_EXPECT=1e90ff` — the centre pixel must be the page's colour, so a
  frame that imports but is tiling-garbage fails.
- `PROBE_SWITCHES` — Chromium switches, e.g. `ozone-platform=wayland,use-angle=gl-egl`.
- `PROBE_SANDBOX=sandboxed` — CEF's process sandbox instead of
  `UnsandboxedTrustedContent`.

The matrix, the panel screenshots and the CPU sample:

```sh
scripts/matrix.sh /tmp/cef-evidence/matrix
nix shell nixpkgs#sway nixpkgs#grim nixpkgs#wlrctl nixpkgs#wtype \
  -c ./scripts/panel-headless.sh /tmp/cef-evidence/panel 10
```

Everything in `evidence/` was produced by those two commands plus the
`PROBE_INPUT` scripts recorded in the logs.

## Reading a probe run

```
RECEIPT {"tag":"adapter",...}          host wgpu adapter, and whether it has VULKAN_EXTERNAL_MEMORY_DMA_BUF
RECEIPT {"tag":"switches",...}         the Chromium switches and the display environment
RECEIPT {"tag":"first_frame_ms",...}   from process start to first imported frame
RECEIPT {"tag":"native_frame",...}     DMABUF planes: size, fourcc, modifier, stride
RECEIPT {"tag":"pixels",...}           readback of the imported texture (test instrumentation only)
RECEIPT {"tag":"expect",...}           centre pixel against PROBE_EXPECT
RECEIPT {"tag":"popup",...}            a `<select>` popup surface CEF painted separately
RECEIPT {"tag":"result","ok":...}      exit code follows this
```

`import_errors` is distinct from "no frame": a refused import prints the typed
error (`VulkanForeignQueueExtensionNotEnabled`, …) instead of silently
presenting nothing.

## Known limits of this probe

- It is not IntCAD. It uses the same `eframe`/`egui`/`wgpu` generations, but the
  real Assistant page needs the app's IPC bridge (see the report).
- The panel is deliberately small: no menus, no dialogs, no drag-and-drop from
  a file manager, no OS input-method engine (IME is driven at CEF's API).
- Screenshots come from a headless wlroots compositor; nothing here opens a
  window on the developer's desktop.
