# CEF Assistant: native Wayland density and compatible frames

**Human-approved design — user answered “批准按此实施” for sections 1–5.**
Approved direction: bounded approach B, native Wayland OSR configuration, not a
custom CEF fork. Implementation awaits controller review of the implementation
plan; no runtime acceptance or binary validation is implied by design approval.

Risk **L3**: native runtime configuration, GPU-frame ownership, input and artifact
identity. Worktree `worktrees/cef-osr`, branch `probe/cef-welding-osr`, source
baseline `2284165658172c0ed528ed5f8377eab11b16b748`.

## Approved design

### 1. Outcome and selected proposal

Make the Assistant as readable as surrounding Intcad controls: no texture
upscaling blur, tiny prose/labels, or black startup/resize rail.

The user selected **native Wayland, without X11/XWayland fallback**. Propose
**approach B first**: a version-verified, Linux-native-Wayland, **OSR-only Intcad
CEF runtime configuration** that disables `WaylandFractionalScaleV1` before CEF
initialization. Keep stock CEF, native client ScreenInfo DSF and accelerated OSR.
The winit host continues to use native Wayland independently.

This is not a general Chromium policy or a promise to preserve windowed CEF
behavior in unrelated applications. That stricter contract belongs to fallback
approach A, not to the user's original requirement. Independent Astra guidance
now prefers bounded B over a custom CEF build, subject to scope and regression
proof. The user approved bounded B; A remains fallback research, not permission
to start a native fork/build.

**Acceptance contract:** native Wayland OSR preserves live density at 1/1.25/1.5/2,
shows compatible themed frames and readable semantic text with aligned input,
and passes identified-artifact VM-first/host-last verification without emulation,
X11 fallback or concealed blocked checks.

### 2. Native configuration and OSR-only boundary

Before `CefRuntime::initialize`, apply B only to Linux native-Wayland Assistant
OSR. Windows/macOS configuration remains unchanged. Other Linux backends must
not be silently selected or used as recovery.
Use the actual host/backend initialization identity, not merely an XDG session
label, to establish that scope.

- Merge feature **names** into one effective disabled-feature list containing
  existing `Translate` and `WaylandFractionalScaleV1`; deduplicate deterministically.
  Do not concatenate through the diagnostic comma-separated switch parser, which
  treats commas as separate switches rather than feature-list values.
- Parse/check effective enabled and disabled features. A conflicting enable,
  incompatible Ozone choice, already-initialized conflicting runtime, or unknown
  runtime baseline produces a clear failure, not last-switch-wins behavior.
- Log effective backend, OSR policy, feature names and actual CEF/Chromium identity,
  without tokens, keys or private environment contents. All instances in one
  process must share the same immutable initialization policy.
- Verify the actual loaded CEF/Chromium baseline, not only Cargo bindings or
  `CEF_PATH`. Initially verified against CEF 154.0.34 / Chromium 154.0.8037.98.
  Upgrade compatibility is an explicit recheck; do not freeze insecure native
  versions indefinitely or silently apply the workaround to unknown versions.
- Remove WIP production CDP device-metrics emulation only after native proof.
  Production has no remote-debug listener, native DevTools window or arbitrary
  caller-controlled DevTools messages.

The source audit in Appendix B establishes the current intended OSR boundary,
but does not prove that all future CEF auxiliary entrypoints are impossible.
Enforce it at real host creation/request boundaries: root producers must request
windowless rendering; keep popup-browser cancellation; do not expose native
DevTools-opening requests. Assert/observe created browser identities and OSR
status in tests. An unexpected windowed browser blocks B, not a hidden fallback.
An after-created check alone is not prevention of a windowed request.

Do **not** deny OSR `<select>` popup textures or fullscreen OSR child widgets.
Do not remove working dialogs, attachment paths or other user-facing features
to make the invariant pass. The audit found no required feature removal; if a
real supported capability conflicts during implementation, report that material
decision once. Do not invent an invisible DevTools browser as a substitute.

### 3. Retained fixes and the smallest frame-readiness work

Keep the independent font-role, themed-backing, transparency, gamma-copy and
pixel-alignment work. Prose/labels use native semantic roles (currently 12.5);
the root remains 12 for spacing. Do not use CSS zoom, transforms or per-theme
palette forks. Measure computed prose/textarea/label sizes, not just root font.

Keep native resize/scale notifications and one physical→DIP input conversion.
Draw one imported texel per framebuffer pixel. For target P, scale s and the
producer's rounded DIP L=P/s, expect `ceil(L×s)` native pixels. At 1.25,
370×898 DIP gives 463×1123; clipping one bottom row to a 463×1122 placement is
rounding, not stretching or blur.

**Bounded metadata seam:** carry an owned scalar record from the CEF callback
through welding native-frame acquisition to the matching imported texture:
coded size, visible/content rectangles, source size with validity, capture
counter/timestamp with validity, and existing frame generation/browser lifetime.
Copy values during the callback, never borrowed CEF pointers. Preserve fd
duplication, close-on-error, drop/import ownership and GPU synchronization.
Use a repository-owned welding patch/local input if required, not registry edits.

Admission checks must reject known incompatible frames:

- Zero/hostile dimensions, invalid/missing required geometry or rectangles outside
  allocation; mismatched import/metadata identities.
- Source density inconsistent with current target/scale; old letterboxed content
  instead of a compatible visible/content region.
- Frames already queued before a resize/scale/reload invalidation, old browser
  lifetimes and repeated/older identities treated as new transition evidence.

On transition, invalidate page/popup admission, retire pending old frames and
held pointer gestures, and retain themed native backing. Do not route page input
through an old transform or replay a queued click into changed layout. Bounded
ordered key/IME queues must belong to their valid lifetime/placement; cancel
rather than replay ambiguous input. Re-enable against an admitted compatible
frame and current transform; separately validate popup/IME geometry.

**Exact remaining limitation:** size/source/counter checks and queue invalidation
do not prove renderer-commit freshness. In A→B→A, a late old-A capture can have
the same geometry and a newer observed counter. Never stamp the callback with
the current app target and call that native provenance. Test delayed/reordered
A→B→A frames and visible/input behavior explicitly. A user-visible stale-frame
or stale-click regression blocks acceptance.

Do not reflexively add a generic epoch framework, CEF ABI extension or custom
native build to solve a stronger abstract ordering problem. If the bounded
checks cannot meet the original visible/input goal, specify the concrete failing
case and the smallest native fence/transport change as a separate reviewed
extension. Capture counters, BeginFrame IDs and capture versions are not
interchangeable. Strict causal freshness beyond the demonstrated goal remains
unproven, not silently waived or claimed solved.

**No theme deadlock:** initialize/reinstall the bridge and push latest theme/state
on page/browser readiness independently of texture admission. Currently bridge
initialization follows first-frame publication; split those dependencies before
rejecting startup frames. Startup surface IPC is not an authoritative theme token.
Reload resets page readiness. Pending/mismatched/error paths keep themed backing,
gate input and report a bounded failure while CAD stays usable.

### 4. Verification and delivery gates

Use focused tests in existing area's files, not a new framework.

| Gate | Required proof |
|---|---|
| Configuration | Linux Wayland OSR scoping; Translate merge; duplicate/conflicting feature handling; initialization identity; Windows/macOS unchanged |
| Browser scope | Root windowless creation; `window.open` and target-blank denial; no native DevTools opening/remote listener; context-menu routing; OSR select/fullscreen/auxiliary paths; multiple roots/process instances |
| Frame metadata | Validity/bounds/ownership/import pairing; padded/stale/reordered captures; A→B→A limits; current-transform input gating; theme-before-admission |
| Native density | At 1/1.25/1.5/2: requested DIP/DSF, effective native DSF, callback coded/visible/content/source, imported extent and draw clip, with CSS/DPR only supporting |
| Lifecycle/input | Startup, scale-only both directions, resize bursts/same-size changes, hide/reopen/reload, clicks/wheel/focus/selection/drag, popup position, IME preedit/commit/cancel/candidate placement |
| Visuals | All 25 built-in themes: background/rail parity, semantic text size, existing contrast policy, disabled/hover/focus, transparency and color swatches; real pixel-scale comparison beside native controls |
| Artifact | Authenticated stock CEF origin, fresh whole-bundle hashes/config, actual loaded runtime/subprocess/resources, recorded transforms; no historical recertification |

Extend existing `agent_chat/tests.rs` and relevant app/welding test files.
Keep regression tests for native metrics/readiness after deleting
`assistant_cef_viewport.rs` if its CDP-only machinery has no remaining user.
Direct CEF IME events do not prove a real OS candidate window works.

Order: source checks → resource-permitted trusted local build → **VM first** →
same identified artifact **host last**, with final manual user review.
Use task-owned app-control and VM evidence, not host niri input/capture.
Log PASS/FAIL/BLOCKED/NOT_RUN per step. The last VM run
`20261119T034242Z-1317238` had 1–5 PASS, 6–8 BLOCKED on llvmpipe, overall FAIL.
Missing GPU interop proves graceful refusal, not accelerated visuals.
Relevant shared-Rust and Windows/macOS compile coverage must be reported where
touched; host GPU success is not a VM or cross-platform pass.

Independent quality/spec review and L3 review cover the final revision and raw
evidence. Earlier architecture audits and implementer self-review do not approve
the new diff. Rollback uses a matched app/runtime/resource bundle, not emulation,
X11, another WebView or a CPU fallback.

### 5. Resources, authorization and state

The one authorized cleanup pass released **19.81 GiB**. No further cleanup cycle
or system change is authorized. Aggregate usage remains above **200 GiB**:
**no new heavy app/CEF/Chromium build now; no CI push**. Do not infer a waiver
from “continue”, repeatedly ask the same budget question, or create a builder
service. Spec/source work and suitably tiny checks may continue within approval.

This design requires no custom CEF build for B. App/welding changes still need a
new trusted app artifact when resources permit. Existing artifacts can support
bounded diagnostics, not silently become newly built/accepted outputs.

| State | Meaning |
|---|---|
| Design approved — current | Human-approved bounded B; controller plan review precedes product implementation |
| Source-ready | Scoped source/patch/config and reviews complete; lists unrun builds/tests |
| Binary-validated | Identified bundle built/verified and actual required native checks run |
| Accepted | Required VM/visual/independent gates and final host user approval satisfied |

The design approval authorizes planning bounded B, with product work gated on
controller review of that plan. No build, commit or push is authorized by this
planning stage. No new provider, public posting, release, artifact service, fleet
or persistent native fork is authorized. There is presently no additional
material product question.

## Research and evidence appendices

### Appendix A. Confirmed root cause and versioned sources

welding supplies DSF 1.25 and 370×898 DIP for a 463×1122 placement. Chromium 154
`RenderWidgetHostViewBase::UpdateScreenInfo` overwrites DSF on Ozone platforms
with per-window scaling using a preferred scale for `GetNativeView()`, falling
back to 1. CEF OSR's native view is null. Native capture becomes 370×898.
Baseline stretches it; WIP's 1:1 drawing exposes an undersized image instead.
WIP CDP emulation changes JS DPR, not this native override.

Pinned CEF: **`14c5a089e8452874cb1a556dc3fde26ed2d87723`**,
154.0.34+g14c5a08+chromium-154.0.8037.98.
Its CHROMIUM_BUILD_COMPATIBILITY.txt pins **refs/tags/154.0.8037.98**.
Cargo.lock pins welding **0.15.0**, cef/cef-dll-sys **154.5.0+154.0.34**.

Primary sources:

- [Chromium UpdateScreenInfo](https://github.com/chromium/chromium/blob/154.0.8037.98/content/browser/renderer_host/render_widget_host_view_base.cc#L531), overwrite lines 577–595.
- [CEF OSR](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/libcef/browser/osr/render_widget_host_view_osr.cc): native view lines 354–358, client ScreenInfo lines 767–806.
- [Ozone capability](https://github.com/chromium/chromium/blob/154.0.8037.98/ui/ozone/platform/wayland/ozone_platform_wayland.cc#L420),
  [connection gate](https://github.com/chromium/chromium/blob/154.0.8037.98/ui/ozone/platform/wayland/host/wayland_connection.cc#L534),
  [fractional manager](https://github.com/chromium/chromium/blob/154.0.8037.98/ui/ozone/platform/wayland/host/fractional_scale_manager.cc#L24),
  [feature definition](https://github.com/chromium/chromium/blob/154.0.8037.98/ui/ozone/common/features.cc#L13).
- [CEF capture metadata](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/libcef/browser/osr/video_consumer_osr.cc#L82):
  coded allocation may contain letterboxing; content/source/validity matter.
- [Chromium metadata](https://github.com/chromium/chromium/blob/154.0.8037.98/media/base/video_frame_metadata.h):
  capture counter/version and BeginFrame identifiers have different meanings.

The feature is upstream source-defined and default-enabled; it is **not a stable
OSR-only per-view switch**. It prevents fractional protocol binding/capability.
Real CEF Wayland windows then use entered-output rather than per-surface scaling.
That broader process effect is why B must stay in Intcad's audited OSR-only
runtime, not a generic CEF configuration. Ozone's capability test overrides are
test-only static state, not shipping flags.

### Appendix B. Browser/auxiliary scope audit

Inspected welding 0.15.0 `linux_cef/{mod.rs,cef_backed.rs}`, runtime/app/surface
sources, Intcad `assistant_cef.rs` and `main.rs`. Findings:

| Path | Existing behavior / design obligation |
|---|---|
| Root browser | `linux_cef/mod.rs:185–216` creates WindowInfo with windowless=1, shared textures=1 and no parent; `main.rs:1443–1450` has one Assistant root construction path |
| window.open / target-blank | `WeldLifeSpanHandler::on_before_popup`, `cef_backed.rs:359–387`, emits NewWindowRequested and returns 1 to cancel creation; retain/test it |
| Native DevTools | No ShowDevTools opening call in app/welding; protocol observation/snapshots do not require a window. The default life-span DevTools-popup hook is not an explicit denial and has no cancel return. Prevent opening at actual host request entrypoints; do not claim blanket callback cancellation exists |
| Context menu | `cef_backed.rs:856–900` emits host event, clears native model, claims/cancels native menu; preserve routing and test no new browser |
| JS dialogs | WeldClient supplies no JS-dialog handler. Pinned CEF's Linux native delegate inherits a null default runner; `CefJavaScriptDialogManager::CanUseChromeDialogs` cancels default dialogs for windowless/no-parent. Preserve existing behavior, not a newly removed feature |
| Select/autocomplete popup | Separate PET_POPUP OSR texture, not a popup browser; `surface.rs:532–542` explicitly distinguishes them. Preserve popup metadata/input and drawing |
| Fullscreen / child widgets | Pinned `CefWebContentsViewOSR::CreateViewForChildWidget` constructs CefRenderWidgetHostViewOSR for popup/fullscreen. Display fullscreen notification is not wired into a new native app window; preserve/test existing path |
| Other auxiliaries / instances | No general app windowed-CEF constructor found. Audit keyboard, IPC, attachment/file-dialog and future auxiliary requests in tests; reject unsupported windowed-browser creation at request boundary. Separate app processes get the same config; same-process roots cannot reinitialize with different policy |

Pinned primary references:
[OSR child creation](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/libcef/browser/osr/web_contents_view_osr.cc#L103),
[popup OSR geometry](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/libcef/browser/osr/render_widget_host_view_osr.cc#L645),
[default JS dialog policy](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/libcef/browser/javascript_dialog_manager.cc#L320),
[DevTools opening path](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/libcef/browser/devtools/devtools_window_runner.cc#L12).
Source contracts are not runtime popup/IME or auxiliary-window acceptance.

### Appendix C. Runtime experiments and raw evidence

All paths below are local under `/tmp/opencode/cef-review/`.

| Experiment | Result / evidence |
|---|---|
| Baseline/WIP ordinary Wayland | Native final 370×898; `baseline-probe.log`, `candidate-probe.log` |
| Chromium's own override log | `baseline-wayland-verbose.log`, `wayland-override-evidence.txt`: “from 1.25 with windows scale 1” |
| Native conversion/gathered DSF | `baseline-gdb-v2.log`, `baseline-gdb-scale.txt` |
| Diagnostic skip counterfactual | `baseline-wayland-skip.log` and `candidate-wayland-skip.log` contain raw callback metadata; WIP log also records imported 463×1123. Extracts end in `-evidence.txt` |
| Supporting CDP only | `baseline-wayland-skip-stable.txt`, `candidate-wayland-skip-stable.txt`; not native callback proof |
| WIP + verified feature disable | `candidate-fractional-disabled.log` imports 463×1123; `-runtime.json` records no preload/debugger, Wayland/no DISPLAY; internal WIP CDP emulation still present |
| Baseline + feature disable, uninstrumented | `baseline-fractional-disabled.log`, `baseline-fractional-disabled-stable.txt`, `baseline-fractional-disabled-provenance.json`, `baseline-fractional-disabled-maps.txt`: CSS 370×898 / DPR 1.25; no production metrics emulation, preload or debugger |
| Baseline + same flag, separate native observation | `baseline-fractional-disabled-observe.log`, extracted `baseline-fractional-disabled-native.txt`: effective native DSF 1.25; coded/visible/content/source all 463×1123; validity flags true |

The last baseline run used observational debugger breakpoints/register-memory
reads only (`observe-baseline-paint.gdb`): **no inferior data assignments, calls,
control-flow skips or preload**. No external CDP commands were issued during
that native callback observation; app-control rect queries only pumped the loop.
This removes the WIP internal Emulation confound. Callback ABI sizes were 272
and 112, matching the pinned Linux headers. Both baseline probes were closed
through authenticated task app.quit.

Fresh uninstrumented baseline executable SHA:
`fa7d6888a5069a8b87cad24858a0cebf34f17fa6114aa390321397595030c0aa`.
Its actual mapped CEF was `/tmp/cef-root/154.0.34/cef_linux_x86_64/libcef.so`,
SHA **`8952fb40b6bd1d8f145a27c9f4103dac92438309768d928235c4a8913eb1dba3`**.
Observer maps were not separately captured before closure: a PID-file parse
failed because it contains PID plus start time. `-observe-provenance.json` labels
that limitation; its loaded-map identity comes from the preceding uninstrumented
run, not an invented observer receipt. Disk app files were not modified.

At 1.25 both baseline native callback proof and WIP import proof now recover
463×1123 with the feature disabled. This is a bounded diagnostic result, not the
full 1/1.25/1.5/2, readiness, popup/IME, theme, VM or final host acceptance matrix.

Rejected alternatives: X11/XWayland violates user direction; JS emulation alone
did not repair native capture; headless Ozone could not allocate accelerated
capture here; force-scale/startup-only/rectangle-only probes did not repair the
ordinary Wayland case. No blind feature search or flag permutation was used.

The historical `verification-summary.json` **8/8** result checked evidence
consistency, hash-record stability, source cleanliness and a closed control port.
It was not eight product tests or eight acceptance gates. Diagnostic image
`native-skip-diagnostic.png` was app-captured, not an accepted shippable artifact.

### Appendix D. Provenance, licenses and upgrade responsibility

B uses stock CEF and does not require patched-CEF notices or a custom native
builder. It still requires authenticated origin, exact CEF/Chromium revisions,
whole-bundle inventory/SHA-256, effective config and actual loaded identity.
Rust binding version/CEF_PATH or a matching `.text` hash is insufficient.
Record app revision/dirty diff, Cargo.lock/toolchain, wrapper and subprocess
identity, ICU/V8/pak/locales/sandbox/GPU resources, and every strip/RPATH/relocation/
packaging transform with tool, arguments and pre/post hashes. Prevent stale
loader/cache paths or automatic downloads from selecting a different runtime.

Earlier recorded VM SHA
`c14d3f2e69fc733b5e1402ebe6d0bb9e173b1d0a0db56c239c5b191ad5461f10`
and adapted host SHA
`d62533ef599f3be7e33dd413fecffe6e0e2cbba62ddbf2bbd3b38e5867a9264f`
do not match fresh existing files:

- WIP host: `4cae2be541d0553f76aa763a1b65079e52cf0705234e073ce1a41292afcaf4b7`.
- VM-path `0e3927c0ad4e-d8cfe0d1b0f8/solidcraft`:
  `ad62560bf64201090d6a8cc65caab93651c6cce58e22c416ac635bf11ad910f7`.
- Their matching `.text`:
  `fc277719ff5426c9a15489b55b772da1224a224e15897e3aeb0901d2e2514e9d`.

The historical mismatch remains unresolved. Preserve old manifests; never
recertify an earlier VM pass against these bytes. A newly authenticated stock
CEF bundle may avoid rebuilding CEF; a fresh trusted app build/config/bundle run
is still required for delivery. No heavy rebuild merely to repair a label.

Keep sandbox/security settings. Stock CEF license/credits must match the actual
bundle, not only Cargo checks. If welding changes, correct its “unmodified”
notice, meet MPL source obligations and use repository-owned source/patch inputs.
No CEF modification notice is needed merely for a flag, but native license
closure remains required. Use normal generators/attribution-pending paths;
do not hand-edit generated catalogs.

Build coordinates ownership of version checks, security updates and workaround
removal when upstream behavior passes the same regression matrix. No perpetual
insecure pin, new public artifact service or persistent fork is implied.

### Appendix E. Approach A — fallback research, not the selected build project

If B cannot be scoped or regresses required OSR behavior, reconsider a small
default-preserving CEF OSR policy at Chromium's exact per-window overwrite point.
Existing CEF `views_widget.patch` makes GetNewScreenInfosForUpdate virtual, but
that hook is too early; UpdateScreenInfo overwrites afterward. GetDeviceScaleFactor
is final. No inspected existing hook provides the exact OSR-only policy.

The base policy would preserve current behavior; CEF OSR/null alone would opt
out. Real native and non-OSR null views, native nullopt→1, unsupported Ozone and
non-Ozone paths would stay unchanged. Preserve capture scaling, rotations,
notifications, visual-property ordering and popup synchronization. Do not return
early, duplicate UpdateScreenInfo or restore scale after notifications. Null
native view alone is not OSR identity.

Prospective patch input:
`packaging/cef/patches/0001-osr-client-screen-dsf.patch`, including tests in
Chromium's existing `render_widget_host_view_base_unittest.cc`. Pin CEF/Chromium
commits, upstream patch order and pre/post source hashes; zero-fuzz application.
No patch is created at this stage.

A requires separately reviewed local-build feasibility: exact DEPS/depot_tools,
Chromium Clang/sysroot, GN/Ninja/GN args, ABI-matched bindings/resources, native
unit tests and new bundle hashes. A full Chromium checkout/output may not fit
the 200 GiB aggregate limit at all. It also requires truthful modified-CEF/
Chromium notices and actual generated credits. No opaque ELF edit, shipped
preload, registry edit, new service/fleet or persistent fork substitutes for this.
Do not automatically start A because one B check is blocked.

### Appendix F. Review and remaining evidence

Initial independent L3 audit:
`target/cef-review/native-architecture-review.md`, Astra session
`ses_e0df1ef1cffeuZmFv3ovy7CDJp`, delivery BLOCKED and no binary execution.
Controller's revised Astra decision prefers bounded B; the actual final design/
implementation and raw evidence still require independent review.

Remaining: effective-config/OSR boundary enforcement and negative tests, native
metadata transport/admission and its stated freshness limits, upgrade/provenance
verification, full density/input/popup/IME/25-theme matrix, resource-permitted
trusted app build, required VM evidence and final manual host approval.
Self-review or the diagnostic native success cannot replace those gates.
