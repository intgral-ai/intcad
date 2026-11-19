# Assistant visuals: rendering and theme parity with the native window

Branch `probe/cef-welding-osr`. Scope: the Assistant's page as an egui texture — resolution,
colour, transparency, typography and every built-in theme. The CAD renderer, the chat/MCP
contract and the CEF single-implementation cutover are unchanged.

**Current status: source WIP, not visual or native-density acceptance.** The approved bounded
design B is implemented and source-reviewed: stock CEF 154 with `WaylandFractionalScaleV1`
disabled before initialization on Linux native Wayland Assistant OSR only, version-verified and
merged with `Translate`, with explicit conflict/identity checks and inherited-argv suppression.
Native frame geometry/admission and the native DSF notification/retry are also in source. A
no-emulation diagnostic on a pre-existing stock CEF binary with that feature flag produced an
effective native DSF of 1.25 and a 463×1123 native capture at 1.25. That is not current-binary
validation: the compiled app, native CEF/GPU execution and the 1/1.25/1.5/2 native matrix, every
source theme and input/IME remain BLOCKED/NOT RUN. No CEF fork or custom Chromium build is required
or approved; the per-window Chromium guard is unapproved research (approach A).

> **Correction (2026-11-19):** earlier text in this file described research approach A (a
> Chromium source guard / custom CEF build) as required and said the matrix must wait for a CEF
> fix. That was wrong: the approved design B is the stock-CEF feature-disable configuration above.
> The earlier wording and the corresponding report text are corrected here; raw diagnostic logs
> are untouched.

## 1. Root causes

### 1.1 The page was too dark (sRGB decoded twice)

CEF's accelerated frame is a GPU texture in an sRGB format (`Bgra8UnormSrgb` on Linux). egui's
shader (`vendor/egui-wgpu/src/egui.wgsl`) samples a texture and treats the sample as
**gamma-encoded**: it multiplies by the vertex colour and calls `linear_from_gamma` itself before
the sRGB target encodes on write. Sampling an sRGB *view* makes the hardware decode first, so the
page is decoded twice and every mid-tone came out dark: a `#3a88ee` swatch measured `(25, 59,
205)` instead of `(58, 136, 238)`, and `#0d0d0d` measured `(5, 6, 8)`.

Fix: the frame is copied **on the GPU** once per painted frame into a texture created with the
non-sRGB format in `view_formats`, and egui reads that non-sRGB view. WebGPU refuses to let a view
pick a format the texture did not declare (`Bgra8Unorm` of a `Bgra8UnormSrgb` texture is a
validation error), and the imported texture belongs to welding, so the copy is the way to keep the
page's bytes. No readback, no CPU work. Measured after: the same swatches are exact and the page's
background equals the theme's surface token byte for byte.

### 1.2 White corners and a rectangular page

The page's CSS is see-through outside its rounded surface (`html, body, #root { background:
transparent }`), but CEF paints its own default background behind it — opaque white — so every
transparent pixel became a white square with sharp corners inside the panel. Fix:
`CefSurfaceConfig.background_color = None`, and the app's rounded panel shows through.

### 1.3 The page ignored the window's theme

The page styled itself from its own `dark` class and Mastra's palette; the host sent only a
`dark: bool`. Fix: the app now sends the **tokens it is drawing with** (`theme` in the state
JSON), and the host injects a small applier that writes them as CSS variables on the page's root
(the variables Mastra's components already read). One source of truth: `Tokens::get()` after
IntCAD's own theme resolution, so built-in theme files, colour-vision modes and the user's theme
files all arrive without a branch on the theme's name.

### 1.4 Everything was 25 % too large

The page sets `html { font-size: 125% }` (a WRY-era decision for a browser-sized palette).
The first fix changed the root to `Tokens::font_body` (12 points). Manual review exposed an
overcorrection: Mastra's body is `0.875rem` and labels are `0.8125rem`, yielding actual text of
10.5 and 9.75 CSS pixels. A root-size measurement did not establish readable text sizes.

The bridge now maps the existing semantic `--text-*` variables to explicit native point sizes.
Prose and labels follow `font_label` (12.5), the same role used by native Assistant prose and
form labels in `ui_kit`. Caption, menu and heading roles retain separate token mappings. The root
stays 12 for layout spacing; there is no CSS `zoom`, transform scaling or per-control patch.

### 1.5 Low native capture density at fractional scale (root cause historical; design B implemented)

At native `pixelsPerPoint=1.25` the page lays out a 370×898 CSS viewport, while native CEF
without the design-B configuration produced only a 370×898 raster. The root cause is known
from native callback telemetry, GDB and the unmodified Chromium source:

1. welding answers `GetViewRect` in DIP and `GetScreenInfo` with DSF=1.25 correctly.
2. CEF converts that into Chromium `ScreenInfo` with DSF=1.25.
3. Chromium 154 `RenderWidgetHostViewBase::UpdateScreenInfo` overrides the screen DSF when
   Ozone supports per-window scaling: `GetPreferredScaleFactorForView(GetNativeView())`
   falls back to 1.0.
4. A CEF windowless OSR view has no native view, so Wayland's per-window fallback replaces
   welding's 1.25 with 1.
5. CEF consequently produces a 370×898 native raster. Stretching it to 463×1122 blurred it;
   drawing it 1:1 at physical size makes the same page undersized.

The decisive line from the unmodified Chromium log is `UpdateScreenInfo: Overriding scale
for screen '' from 1.25 with windows scale 1`. A task-owned debugger counterfactual that
skipped only that Ozone override block produced native and imported 463×1123 (correct
ceiling for 898×1.25) on the same app, CEF, hardware and settings. That is evidence for
research approach A; A (a Chromium source guard or custom CEF build) is **unapproved and
unimplemented**, and the selected design does not require it.

Selected design B (implemented and source-reviewed; not runtime-accepted): before
`CefRuntime::initialize`, `assistant_cef_config` builds one effective policy for Linux
native-Wayland Assistant OSR only. It merges the feature names `Translate` and
`WaylandFractionalScaleV1` into one deduplicated `disable-features` value, rejects an
`enable-features` conflict, an incompatible Ozone/headless choice, an unknown runtime
identity or a conflicting second initialization, and suppresses inherited browser argv. It
version-verifies CEF 154.0.34 / Chromium 154.0.8037.98 (API 15400, commit `14c5a08…`);
Windows/macOS and other Linux backends keep their previous settings. Tasks 1–3 also added
the geometry/admission seam and native DSF notification/retry. No CEF fork, Chromium patch,
registry edit or X11 fallback is used or approved.

No-emulation flag proof (historical pre-existing stock CEF binary; no
`Emulation.setDeviceMetricsOverride`, no `LD_PRELOAD`, no X11): with
`WaylandFractionalScaleV1` disabled, effective native `GetDeviceScaleFactor` became 1.25 and
the native callback settled at coded/content/source 463×1123 while CSS layout stayed
370×898; the page capture was 463×1123. Only 1.25 was measured, on the baseline artifact.
It supports the mechanism, not current-binary or 1/1.5/2 acceptance.

The former `Emulation.setDeviceMetricsOverride` attempt changed the JS DPR but did not repair
native density; it has been removed from the app, together with the app-only density protocol
enablement and polling. The native producer's `notify_screen_info_changed`, `was_resized` and
`invalidate` calls and the host's correlated, bounded retry remain.

Current source behaviour: imported texels are drawn at native physical size from a
pixel-aligned origin, stale resize frames are clipped rather than stretched, input uses the
same origin, and native popup surfaces are not clipped to the page rectangle. Source-level
config/identity/geometry/viewport/admission tests pass; the compiled app, native CEF/GPU
execution, the 1/1.25/1.5/2 capture/import/input matrix and all-theme visuals are
**BLOCKED / NOT RUN** until a trusted stock-CEF build runs them.

The black strip was independently traced to the native resize rail: the page reports its
startup surface before receiving theme tokens, and `web_canvas` retained that old colour.
Pixel samples were `(13,13,13)` in the rail versus `(39,43,52)` in the themed page (historical
measurement). Native frame/backing now use the same `Tokens::panel` source as the theme
bridge. The existing page-readiness IPC is unchanged.

## 2. The bridge

`crates/ui-egui/src/agent_chat/theme_bridge.rs` turns the live `Tokens` into JSON:

- colours: `window/surface/panel/raised/field/control/popup`, `text/textDim/onAccent`,
  `border/borderStrong/hover/selected/selectedText`, `accent/accentSoft`,
  `error/warning/success`;
- metrics: `fontBody/fontProse/fontLabel/fontSmall/fontCaption/fontHeading/fontTitle`, `controlHeight`,
  `radiusSm/Md/Lg/Xl`, `space2/4/8/12`.

`apps/solidcraft/src/assistant_cef.rs` injects `window.__intcadTheme(t)` once and calls it
whenever the tokens change (so a theme switch repaints the page without reloading the browser),
and the JSON is also in `web::state`, beside the old `dark` bit. `theme_bridge` has a unit test
that resolves **every** built-in theme and requires complete colours, a positive body font and a
control height. That test is source-reviewed here but was not executed in this task (it needs
the UI crate and Cargo).

## 3. Evidence (Linux, real app; historical unless marked current-source)

**Evidence status.** Everything in this section is historical: it came from app binaries
whose source/binary SHA identity was not established, and those cached executables were not
rerun for this task. It is retained as the record of what was observed, not as current-source
validation. In particular, the 25×3 matrix compared flat surface colours at sampled pixels; it
does **not** prove native raster resolution or readable type, and it cannot be cited as
all-theme or all-DPI acceptance.

Theme matrix generated from **every** source theme (25 JSON files at that revision), each
applied through `ui.prefs {theme}` at runtime in a historical binary, page background measured from the app's own screenshot and compared with the
token it was given:

```
themes=25 mismatched=0            (125 % DPI, /tmp/themes/matrix-1_25.json)
themes=25 mismatched=0            (200 % DPI, /tmp/themes200/matrix-2_0.json)
themes=25 mismatched=0            (100 % DPI, /tmp/themes100/matrix-1_0.json)
```

The ids below are the files present at that revision. The current matrix script, its fixtures
and the app hook enumerate `assets/styles/ui/themes/*.json` from source; there is no literal
25-theme gate, and the requirement is to cover every source theme, whatever the count. Screenshots:
`/tmp/themes/<id>.png` for all 25 ids (`1-bit`, `1-bit-dark`, `akari`,
`amber-phosphor`, `black`, `blueprint`, `catppuccin-mocha`, `cga`, `clay`, `clay-dark`,
`cupertino`, `graphite`, `green-phosphor`, `gruvbox-dark`, `gruvbox-light`, `high-contrast`,
`nord`, `nosferatu`, `okibi`, `one-dark`, `onyx`, `platinum-9`, `solarized-light`,
`solidcraft-95`, `tokyo-night`).

Historical typography/density measurements (`getBoundingClientRect`):

```
font: 12px (= Tokens::font_body)
buttons: 28, 28, 28, 28, 27, 28  (Tokens::control_height = 28)
composer textarea: 30px high, 435 wide   inputs: 30
```

Before the density rule the same measurement gave buttons of 21 px. The bridge applies a
`control_height` minimum to text-only buttons and inputs; icon buttons keep their shape.
The root-font measurement above did not measure actual prose/label sizes; section 1.4 records
the manual-review correction. It must not be read as a completed typography acceptance.

Text contrast: the bridge computes WCAG contrast of the theme's body text on the panel and, only
when it falls under 4.5:1, nudges the text towards the theme's own light/dark end until it passes.
`solarized-light` is such a theme (4.39:1 before). Secondary text gets the same treatment at
3:1 — `solidcraft-95`, a Win95-grey palette, measured 2.17:1 for its muted text. The JSON carries
`textAdjusted` / `textDimAdjusted` when either happened, so the deviation is visible rather than
silent. The unit test asserts every built-in theme ends at ≥4.4:1 (body), ≥2.8:1 (secondary) and
≥3:1 (selected text on the selection).

Lifecycle (`scripts/theme-lifecycle.py`):

```
startup_theme: dark                    (default theme at start)
switch to nord: painted #2e3440 at once, no white/black flash frame
reopen panel: theme nord, page true
restart (same settings): gruvbox-light persisted
55 switches: RSS 932 -> 1001 MB, fds 401 -> 429
```

Real browser reload (test hook `SOLIDCRAFT_ASSISTANT_CEF_TEST_RELOAD_AFTER`): after a reload the
page comes back and the theme is applied again — measured `#2e3440` (nord's surface) at two points
of the panel, `webView {host: true, page: true}`. This exposed a real bug, now fixed: the theme
function lived only in the page's JS context, so after a reload it was gone and the re-apply call
was a no-op; `ready` now reinstalls the bridge before asking for the theme.

200 theme switches sampled every 50 (`evidence/2026-11-19-states/leak-200-switches.json`):

```
switch   0: RSS 915 MB, fds 399
switch  50: RSS 1026 MB, fds 429
switch 100: RSS 1064 MB, fds 456
switch 150: RSS 1084 MB, fds 480
switch 200: RSS 1101 MB, fds 504
```

RSS is flattening (+99, +37, +20, +17 MB per 50); **fds grow linearly (+25 per 50)** and do not
plateau, so a descriptor is retained per switch somewhere in the CEF/execute-script path. It is an
open defect, recorded rather than hidden.

Hover/focus/disabled per theme: the disabled send control is visible in every one of the 25 theme
screenshots. Hover and keyboard focus could **not** be captured reliably on this host: pointer and
key injection go through XWayland, where the compositor's real pointer is not addressed by
xdotool, so the page's `:hover`/`:focus-visible` states did not trigger (34 pixels differed between
"pointer away" and "pointer over the button", none of them on the button). The variables behind
those states (`hover`, `border-focus`, `selected`) are set from the theme and covered by the
contrast test; the visual state matrix is UNTESTED and needs a session with real input.

The descriptor growth is tied to the switch path: 25 *identical-theme* prefs calls in the same
build left fds at 398–401, so it is not the query/state path; the browser is never recreated.
It is recorded as an open item to re-measure over a longer run.

Other checks:

- sRGB swatches in the page: `#ffffff`, `#ff0000`, `#3a88ee` measured exactly (after the fix);
  before: red `(255,26,26)`, blue `(25,59,205)`.
- Transparency: no white or square corners; the panel's rounded frame is the page's own edge.
- Hot switch: the same process changes theme (`ui.prefs`) and the page repaints in place.
- Popup: a native `<select>` in the page paints as a CEF popup surface, drawn at its reported
  rectangle (not a fixed offset), with the same gamma fix.
- No validation errors in the app log across the runs.

## 4. Known limits

- The page's CSS still belongs to the Assistant bundle (Mastra + `assistant.css`); the host
  overrides its variables and root font size. Moving the override into the bundle would let the
  page work without the host, but the Mastra source checkout is not in this worktree, so the
  bundle cannot be rebuilt here. The injected layer is one function (`__intcadTheme`) and is
  intended to move into the page source in a follow-up.
- Chromium's **native** `<select>` popup (not the page's React menus) paints its own form-control
  colours and does not follow `color-scheme`; the Assistant page itself does not use `<select>`,
  this only showed in the test hook.
- The page's icons and logo art are the Assistant's own; they are not recoloured by the bridge.

## 4.1 Adding a theme later (no Assistant change)
### Theme and density evidence chain

A theme reaches the page in observable stages, and each is separate evidence:
`assets/styles/ui/themes/*.json` → the existing theme resolver (`Tokens`, colour-vision
modes and user files) → `agent_chat::theme_bridge::json` (semantic values only) → the page's
injected CSS variables → computed DOM styles/rects reported as bounded `SCOPE` messages →
the composited pixels in an app-owned window screenshot. A PASS at one stage is not a PASS at
another: requested DIP/scale is not an observed native DSF, computed CSS is not painted pixels,
and a historical screenshot is not current-source visual acceptance.


The current build's themes are every JSON file under `assets/styles/ui/themes/` (25 at this
revision; do not hard-code the count elsewhere). Each is resolved by the
existing theme system into `Tokens`, including colour-vision modes and user theme files, and
the bridge forwards those tokens; the page applies them as CSS variables. There is **no
per-theme branch in the Assistant**, no new CSS file and no rebuild of the page. A new
built-in theme is therefore one JSON file with a complete palette; the `theme_bridge` unit
test walks `theme_files::builtin()`, so a theme that does not resolve to a complete palette
fails the test instead of showing a half-styled panel. The current `theme-matrix.py`
self-test enumerates the same source files rather than a fixed list and accepts 24, 25 or 26
names (the set is the contract, not the number).

Verification limits: the 25×3 historical matrix proves sampled flat colours, not native
raster resolution, type sizes or every interactive state; it was produced by executables of
unproven source identity and was not rerun. Source-level checks that do run in this task are
the config, vendor `runtime_identity`, geometry, viewport and admission Rust modules and the
Python self-tests. The compiled app, the real 1/1.25/1.5/2 capture matrix, every source theme on a
trusted current binary, hover/focus and OS IME remain **BLOCKED / NOT RUN**.

The few semantic values IntCAD's palette does not name directly are filled in the bridge and are
the only place to review a mapping change:

| Page need | Source |
|---|---|
| window / surface | `Tokens::panel` |
| panel header, control well | `Tokens::panel_header`, `Tokens::control_fill` |
| raised surface (cards, popups) | `Tokens::dialog_bg` |
| input background | `Tokens::field` |
| text, secondary text | `Tokens::text`, `Tokens::text_dim` |
| borders, focus | `Tokens::border`, `Tokens::control_border`, `Tokens::accent` |
| hover / selection | `Tokens::hover`, `Tokens::selected_bg` / `selected_ink` |
| accent / on accent | `Tokens::accent`, `Tokens::on_accent` |
| error / warning / success | `Tokens::error`, `Tokens::warning`, `Tokens::agent_ok` |
| body, small, caption, heading, title | `Tokens::font_*` |
| control height, radii, spacing | `Tokens::control_height`, `radius_*`, `space_*` |

## 5. VM verification

The Linux E2E guest renders with **llvmpipe** and has no `VK_EXT_external_memory_dma_buf`, so by
design the app cannot show the page there: it must report the missing GPU interop and keep CAD
working. That is the no-interop gate, not a visual test. Visual/theme acceptance is on the host
(build and run in VMs first, host last). In this task no VM, host session, live CEF run or
screenshot was performed; those gates remain **NOT RUN**.
