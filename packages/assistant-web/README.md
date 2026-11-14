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

The build needs a checkout of `mastra-ai/mastra` at the commit in `mastra.json`, with its
packages installed (`pnpm install --frozen-lockfile` in the checkout). `tools/build.mjs` builds
the page in a scratch folder that holds nothing but the source, and writes down what went in:

```sh
node packages/assistant-web/tools/build.mjs --mastra <checkout> --write   # dist/ and the two files below
node packages/assistant-web/tools/build.mjs --mastra <checkout> --check   # what CI runs
```

Without `--mastra` it takes `MASTRA_CHECKOUT`, or the workspace layout
(`<workspace>/open-source/mastra` beside the `platform/` folder). Commit `dist/`,
`THIRD-PARTY-NOTICES.txt` and `inventory.json` together: `--check` compares all three with a
fresh build byte for byte, and the job `Assistant page` in `.github/workflows/packaging-lint.yml`
runs it against a fresh clone of the pinned commit. The build refuses:

- a checkout that is not at the pinned commit, or has changes where the page reads it;
- any module under a directory named `ee/` (Mastra's enterprise licence), and a stylesheet
  that does not keep Tailwind from scanning those files: no rule of `dist/style.css` may come
  from a file there;
- any module from somewhere other than this page, the checkout, an npm package or the bundler;
- an npm package whose licence is not one of the reviewed expressions listed in the tool.

Build with the tool, not with Vite in this folder. In place, Tailwind also reads whatever lies
here: it used to scan the last build's `dist/` for class names, so the stylesheet depended on
what had been built before. `src/assistant.css` now excludes `dist/` and Mastra's `ee/` from
the scan, and its imports still use the relative path of the workspace layout, which the tool
recreates in its scratch folder.

To check the page's types against Mastra's components (`tsconfig.json` extends the checkout's
own, with the same relative path), with `node_modules` here a symlink to the checkout's
`packages/playground-ui/node_modules` (not checked in):

```sh
ln -sfn "$MASTRA_CHECKOUT/packages/playground-ui/node_modules" node_modules
node node_modules/typescript/bin/tsc -p tsconfig.json
```

The names of the page's words (`StringKey` in `src/host.ts`) and the list the app sends
(`strings` in `web.rs`) are kept equal by a test in `crates/ui-egui/src/agent_chat/tests.rs`.

## What `dist/` is built from

`inventory.json` is the list, taken from the bundler's own record of the modules in the script;
`THIRD-PARTY-NOTICES.txt` has the licence and notice texts. Both are written by the build. The
app shows the notices under Help > About > Licences, and the packages carry them as
`ASSISTANT-THIRD-PARTY-NOTICES.txt`.

| Input | Version | Licence |
|---|---|---|
| `mastra-ai/mastra`, `packages/playground-ui` | the commit and version in `mastra.json` | Apache-2.0; no file under an `ee/` directory is in the script, and the build stops if one is reached |
| Mona Sans, Commit Mono (from `packages/playground/src/assets/fonts`) | as in that commit | SIL OFL 1.1, the texts beside the fonts |
| The npm packages those components import (React, Base UI, lucide, react-markdown and others) | the exact versions in `inventory.json` | MIT, ISC and Apache-2.0, each stated by the package and listed there |

## Status: not cleared for distribution

- The owner decided on 2026-11-10 to replace the Assistant's interface with Mastra's. The
  repository's rules (`AGENTS.md`, `skills/solidcraft-ground-rules/SKILL.md`,
  `docs/mastra-distribution.md`) still say that no Mastra source is importable and that the host
  stays Rust; their wording has not been changed, and nothing in `packaging/mastra/` records
  this use.
- Owed before any build of this leaves the team: the rule change in the owner's words, and a
  review of `inventory.json` and `THIRD-PARTY-NOTICES.txt` by someone who knows licences. Those
  two files are what a build tool found and what the packages state about themselves; they are
  not a rights clearance. `ATTRIBUTION.md` rows are pending in
  `attribution/pending/assistant-web.md`.
- The package version is an alpha: component interfaces may change between Mastra commits.

## Where it runs

Only inside the app's own window: the owner decided on 2026-11-10 that the Assistant is a panel
of the app, not a separate window, so with the web view the panel's **Own window** is not offered
and a saved one is put back.

- **macOS**: WKWebView. The view is seen through outside what the page draws, so the panel's
  rounded frame shows around the page.
- **Linux**: WebKitGTK, in a child of the app's X11 window. In a Wayland session that window is
  an XWayland one, which is what the app opens there by default: a Wayland desktop gets the page
  this way. Building needs the system's GTK 3 and WebKitGTK 4.1 development packages
  (`libgtk-3-dev libwebkit2gtk-4.1-dev` on Debian and Ubuntu), and the built app needs those
  libraries at run time. A child X11 window is not seen through: the view is square, with the
  page's own surface colour behind it. winit does not run GTK, so the app gives GTK its turns
  each frame, and asks for a frame about every 16 ms while the page is at work, every 50 ms
  while it has the keyboard and every 100 ms while it rests: with the panel open the app draws
  ten to twenty frames a second even when nothing changes.

  The keyboard: the view's window never has it of its own accord, and the X server sends typed
  keys to the window that has it, the app's. So a press on the page gives the keyboard to the
  window GTK made inside the view's (`XSetInputFocus`), and a press on the rest of the app's
  window gives it back. While the page has it, the app's window reads as unfocused to the app,
  as on Windows. GTK itself is not told (to the X server its window is no top-level one), so
  the page's `focus` event does not fire.

  A native Wayland window (on niri, with **Window system: Wayland** in Preferences, or where
  there is no XWayland) holds no child of another toolkit: there the panel draws the built-in
  conversation, and the app says so once on its error output. To have the page there the view
  would have to be drawn off screen and painted by the app itself, with every key and pointer
  event passed on by hand: that is not built.

  Checked in a container only (Ubuntu 24.04, software Vulkan, WebKitGTK 2.52, GTK 3.24): in a
  Wayland session (sway 1.9 on a headless output, with XWayland) at output scales 1 and 1.5,
  and on a plain X display (Xvfb, openbox). Not on a real desktop or graphics card, not under
  GNOME or KDE, and input methods (IBus, Fcitx) were not tried.
- **Windows**: WebView2, in a child window of the app's. WebView2 serves no scheme of the app's
  own, so the page has an `http` address there (`http://openskyfall-assistant.localhost/…`, the
  form `wry` gives a custom scheme), which only this view answers. WebView2 keeps its files in
  `%LOCALAPPDATA%\SolidCraft\assistant-web-view`, or in the settings folder a run was given
  (`SOLIDCRAFT_SETTINGS_DIR`); left to itself it would write beside the exe. The view is square
  and opaque, as on Linux. WebView2 takes the keyboard when it is clicked, and a press on the
  rest of the window gives it back to the window. A browser's own keys (find, print, reload,
  zoom, developer tools) are off in the view; the keys of text editing stay. Its loader is
  linked into the exe, so no DLL goes with the app. On a machine without the WebView2 runtime
  the view cannot be made, and the panel draws the built-in conversation. The view is made
  hidden and shown once it is whole: where it cannot be made, `wry` leaves the window it began
  with in the app's window, and a shown one lay over the built-in conversation as a blank box.

  **Run on Windows in CI only; no person has tried it on a Windows machine.** The host was
  written on a Mac, where the app cannot even be type-checked for Windows (a C dependency,
  `ring`, needs Windows' C headers). The CI job `Windows app startup` builds the app and runs
  `tools/windows-assistant-smoke.ps1`, which opens the Assistant on the runner and checks that
  the page reports to the app, that a click on the page gives it the keyboard and a click on the
  rest of the window takes it back, that the view leaves the window's resize edges free in the
  compact layout, and that WebView2's files are in the app's data folder. It then starts the
  app once more with the runtime hidden from it, and checks that the panel goes back to the
  built-in conversation with no window left shown over it. Its screenshots are in the job's
  `platform-windows-startup` artifact. What a run found is in that job's log, not
  here. The runner has one monitor at 100 % and no graphics card, so not known: input methods,
  a scale other than 100 %, files dropped on the panel, and the view over a real card's
  backends. While the page has the keyboard the window reads as unfocused to the app, so what
  the app does only in a focused window (moving the view by holding a modifier key, for one)
  waits for a click on it.

Where the app draws its own title bar (Windows and Linux), the window's edges resize it, and a
native view over them would take the press: the view's place stops short of the window's edges
(`titlebar::inside_resize_zones`), which matters for a panel docked to the window's edge in the
compact layout.

The Linux packages (deb, rpm, AppImage, Flatpak, snap, the Nix flake) do not name WebKitGTK or
GTK as dependencies yet. The Windows installers do not bring the WebView2 runtime: Windows 11
has it, and a machine without it keeps the built-in conversation.
