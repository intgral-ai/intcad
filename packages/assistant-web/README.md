# assistant-web

The Assistant's interface for the desktop app: a page drawn with the components of Mastra's
`@mastra/playground-ui`, shown in a native web view that `apps/solidcraft` lays over the Assistant
panel (`apps/solidcraft/src/assistant_web.rs`). The page is told the Assistant's state and may ask
for a fixed list of actions; both are defined in `crates/ui-egui/src/agent_chat/web.rs`.

- `src/App.tsx` maps the app's transcript and controls onto Mastra's components. Everything
  visual is imported from the Mastra checkout; nothing of Mastra is copied into `src/`.
- `src/host.ts` is the page's side of the bridge, and what makes a web view behave as a panel of
  the app (no navigation, no browser menu, clipboard and links through the app).
- `dist/` is the built page, embedded in the app with `include_bytes!`. It is checked in: the app
  builds without Node.

## Building `dist/`

The build needs a checkout of `mastra-ai/mastra` with its packages installed, beside the
`platform/` folder (`<workspace>/open-source/mastra`), or wherever `MASTRA_CHECKOUT` points.
`node_modules` here is a symlink to that checkout's
`packages/playground-ui/node_modules` (not checked in).

```sh
ln -sfn "$MASTRA_CHECKOUT/packages/playground-ui/node_modules" node_modules
node node_modules/vite/bin/vite.js build --config vite.config.mjs --configLoader native
```

`--configLoader native` keeps the build from writing a temporary file into the checkout. The
stylesheet imports in `src/assistant.css` use the relative path of the workspace layout.

Before building, check the page's types against Mastra's components (`tsconfig.json` extends the
checkout's own, with the same relative path):

```sh
node node_modules/typescript/bin/tsc -p tsconfig.json
```

The names of the page's words (`StringKey` in `src/host.ts`) and the list the app sends
(`strings` in `web.rs`) are kept equal by a test in `crates/ui-egui/src/agent_chat/tests.rs`.

## What `dist/` is built from

| Input | Version | Licence |
|---|---|---|
| `mastra-ai/mastra`, `packages/playground-ui` | commit `aa406eaae60717422a1dba07da5b97fb26a0b998`, package `69.0.0-alpha.4` | Apache-2.0; nothing under an `ee/` directory is imported |
| Mona Sans, Commit Mono (from `packages/playground/src/assets/fonts`) | as in that commit | SIL OFL 1.1 |
| The npm packages those components import (React, Base UI, lucide, react-markdown and others) | as installed in the checkout | not inventoried yet |

## Status: not cleared for distribution

- The owner decided on 2026-11-10 to replace the Assistant's interface with Mastra's. The
  repository's rules (`AGENTS.md`, `skills/solidcraft-ground-rules/SKILL.md`,
  `docs/mastra-distribution.md`) still say that no Mastra source is importable and that the host
  stays Rust; their wording has not been changed, and nothing in `packaging/mastra/` records
  this use.
- Owed before any build of this leaves the team: the rule change in the owner's words, the rights
  review those documents describe, a notices file for everything `dist/assistant.js` contains,
  and `ATTRIBUTION.md` rows (a pending file is in `attribution/pending/assistant-web.md`).
- The package version is an alpha: component interfaces may change between Mastra commits.

## Where it runs

Only inside the app's own window: the owner decided on 2026-11-10 that the Assistant is a panel
of the app, not a separate window, so with the web view the panel's **Own window** is not offered
and a saved one is put back.

- **macOS**: WKWebView. The view is seen through outside what the page draws, so the panel's
  rounded frame shows around the page.
- **Linux**: WebKitGTK, in a child of the app's X11 window (an XWayland window included, which
  is what the app opens by default in a Wayland session). Building needs the system's GTK 3 and
  WebKitGTK 4.1 development packages (`libgtk-3-dev libwebkit2gtk-4.1-dev` on Debian and
  Ubuntu), and the built app needs those libraries at run time. A child X11 window is not seen
  through: the view is square, with the page's own surface colour behind it. winit does not run
  GTK, so the app gives GTK its turns each frame, and asks for a frame about every 16 ms while
  the page is at work and every 100 ms while it rests: with the panel open the app draws about
  ten frames a second even when nothing changes. In a native Wayland window (niri, or **Window
  system: Wayland** in Preferences) there is no child web view, and the panel draws the built-in
  conversation.

  Known limits on Linux: the child window does not take the window's keyboard focus, so typed
  keys reach the page while the pointer is over the panel and go to the app when it is not; and
  input methods (IBus, Fcitx) were not tried. Checked in a container only (Ubuntu 24.04, Xvfb,
  openbox, software Vulkan, WebKitGTK 2.52, GTK 3.24), at window scales 1 and 1.5; not on a real desktop
  or GPU.
- **Windows** (WebView2): the host is not wired yet and was not built or tested; the built-in
  conversation is drawn.

Where the app draws its own title bar (Windows and Linux), the window's edges resize it, and a
native view over them would take the press: the view's place stops short of the window's edges
(`titlebar::inside_resize_zones`), which matters for a panel docked to the window's edge in the
compact layout.

The Linux packages (deb, rpm, AppImage, Flatpak, snap, the Nix flake) do not name WebKitGTK or
GTK as dependencies yet.
