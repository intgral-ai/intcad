# CEF Native Wayland Density Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair Assistant blur/tiny text/black rails using scoped native Wayland OSR configuration and compatible-frame admission, without CDP density emulation.

**Architecture:** Configure the audited OSR-only CEF process before initialization, preserving stock CEF and native screen scale. Vendor the existing welding 0.15.0 package through the repository's normal Cargo patch convention for safe runtime identity access and owned frame metadata; keep GPU ownership unchanged. Separate page/theme initialization, native setter retry and frame/input admission; do not invent native causal epochs.

**Tech Stack:** Rust 1.99.1, std-only policy tests, welding 0.15.0/MPL-2.0, cef/cef-dll-sys 154.5.0+154.0.34, CEF 154.0.34/Chromium 154.0.8037.98, wgpu 30, egui/eframe 0.36, Python stdlib evidence checks.

**Spec:** `docs/superpowers/specs/2026-11-18-cef-wayland-dsf-design.md` — human-approved sections 1–5; planning baseline `08d0ef0e9a2eaf210aa80c5c25f483cb6f7ecc44`.

## Global Constraints

- Native Wayland, without X11/XWayland fallback; stock CEF, no custom CEF build.
- Windows/macOS configuration remains unchanged. Gate B on actual host/backend identity, not an XDG session label.
- Disable `WaylandFractionalScaleV1` before CEF initialization; retain and merge `Translate`.
- Initially verified against CEF 154.0.34 / Chromium 154.0.8037.98. Upgrade compatibility is an explicit recheck, not a perpetual insecure pin.
- Native density matrix: 1/1.25/1.5/2. At 1.25, 370×898 DIP gives 463×1123; no stretching.
- Keep semantic prose/label sizes, themed backing, transparency, gamma-copy and pixel alignment; no CSS zoom, transform or theme forks.
- Preserve fd duplication, close-on-error, import/drop ownership and GPU synchronization; no callback pointers retained.
- Same-geometry late A→B→A renderer freshness remains unproven; a user-visible stale-frame/click failure blocks acceptance.
- No product unsafe/unwrap/expect/panic; unsafe FFI stays within the audited vendored native boundary, with safety comments.
- No registry edits, dependency upgrades, GPL/commercial CAD code/data, generated-license hand edits or new asset copying.
- All 25 built-in themes; VM first, host last; final human visual approval and independent L3/spec/quality gates.
- Aggregate remains above 200 GiB: no new heavy app/CEF/Chromium build, CI push, extra cleanup, system change, service/fleet, merge or public action.
- Controller orchestrates implementers/reviewers and checkpoint commits. Workers do not delegate or push. This plan itself does not start implementation.

## Review Focus

1. Environment overrides enabling `WaylandFractionalScaleV1<Trial` or replacing Translate must fail/merge predictably, never silently win — Task 1.
2. Hidden browser creation via DevTools/target-blank/fullscreen must not escape OSR scoping or disable genuine OSR popup textures — Tasks 1 and 4.
3. A plausibly sized coded allocation with smaller/letterboxed source must never become clickable current content — Tasks 2 and 3.
4. Page reload/theme change while no frame is admitted must still initialize the theme and reject old acknowledgements/input — Task 3.
5. An artifact with matching `.text` but different whole-file/runtime/resources must not inherit an old VM pass — Tasks 4 and 5.

---

## File map and test execution rules

- Task 1: create `vendor/welding/` from the exact published package; modify root `Cargo.toml`, `Cargo.lock`, `vendor/README.md`, `packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt`; create `vendor/welding/UPSTREAM.md`, `vendor/welding/LICENSE.txt`, `attribution/pending/cef-wayland-welding.md`. Modify vendor `src/{lib.rs,runtime.rs,surface.rs,linux_cef/cef_backed.rs}`, add `src/runtime_identity.rs`; add app `assistant_cef_config.rs`; modify app `assistant_cef.rs`, `main.rs`.
- Task 2: add vendor `src/frame_geometry.rs`; modify vendor `src/{lib.rs,native_frame/mod.rs,native_frame/vulkan_dmabuf.rs,linux_cef/mod.rs,linux_cef/cef_backed.rs}`; modify `src/native_frame/{dx12.rs,metal.rs}` only to initialize the optional metadata field to None.
- Task 3: add app `assistant_cef_frame.rs`; modify existing `assistant_cef.rs`, `assistant_cef_viewport.rs` and vendor `src/linux_cef/mod.rs` for explicit native focus clearing; extend existing `crates/ui-egui/src/agent_chat/tests.rs` only where integrated font/backing coverage needs it.
- Task 4: add `experiments/cef-osr/scripts/native-density.py`; extend existing `theme-matrix.py`, `theme-lifecycle.py`, `app-input.py`, `app-ime.sh`, existing `poc/fixtures/probe.html` and the fixed test-only injection hook in app `assistant_cef.rs`; add no separate per-bug test file.
- Task 5: update `docs/cef-assistant-visuals.md`, `docs/cef-welding-cutover.md`, vendor/notices/provenance descriptions from actual results; raw evidence remains task-local, not committed.

Read `vendor/README.md` first: root `[patch.crates-io]`, original licenses and
`SolidCraft:` marks are the established convention. Select **local vendoring**,
not an unapplied patch against a mutable registry. Keep version/features fixed.
Copy only `src/`, normalized `Cargo.toml`, `build.rs`, `README.md` and source
provenance; exclude registry markers, Cargo.lock, caches/examples/assets.
The archive omits a license file: retain all MPL headers and add the canonical
MPL-2.0 text, recording its source. Record package checksum
`e94298e32686b5d447c2abade5bb4594d4e2c4467a7116d3eb253fc7d230fed7`,
upstream commit `1836887569367fe8476b014ed8563d302a5c829e`, original file hashes,
local modifications and upstream-removal criteria in `UPSTREAM.md`.

### Cheap RED/GREEN harness

The new `assistant_cef_config.rs`, vendor `frame_geometry.rs` and
`assistant_cef_frame.rs` must be std-only apart from the latter's geometry import.
Put their tests inline, alongside the production functions. During execution:

```sh
E=/tmp/opencode/cef-review/implementation-checks
mkdir -p "$E"
rustc --version > "$E/toolchain.txt"
rustc --edition=2024 --test apps/solidcraft/src/assistant_cef_config.rs -o "$E/config-tests"
"$E/config-tests" --nocapture
rustc --edition=2024 --test vendor/welding/src/runtime_identity.rs -o "$E/identity-tests"
"$E/identity-tests" --nocapture
rustc --edition=2024 --test vendor/welding/src/frame_geometry.rs -o "$E/geometry-tests"
"$E/geometry-tests" --nocapture
rustc --edition=2024 --test apps/solidcraft/src/assistant_cef_viewport.rs -o "$E/viewport-tests"
"$E/viewport-tests" --nocapture
```

For `assistant_cef_frame.rs`, create a **local**, logged direct-import harness:
`extern crate self as welding; #[path="<absolute worktree>/vendor/welding/src/frame_geometry.rs"] pub mod frame_geometry; #[path="<absolute worktree>/apps/solidcraft/src/assistant_cef_frame.rs"] mod admission;`.
Compile it with `rustc --edition=2024 --test "$E/admission-harness.rs" -o "$E/admission-tests"`.
Run `"$E/admission-tests" --nocapture`. If the available rustc differs from the
requested 1.99.1, label these results auxiliary with that exact compiler; do not
install/upgrade a toolchain or call them target-toolchain integration proof.
No mirrored implementation, replacement types or GPU/CEF stubs. Save harness,
input hashes, commands, outputs and exit codes. This proves only those exact
source modules, **not** app/FFI/GPU integration.

For every runnable task: add its test, run and inspect a real RED, implement the
minimum change, rerun GREEN. A missing tool or blocked compile is not RED.
Do not claim deferred Cargo/FFI tests passed; their actual RED/GREEN cycle waits
for a permitted build. No heavy build is authorized by commands listed below.

### Task 1: Pre-init policy, runtime identity and OSR request boundary

**Files:** Task 1 file-map entries; tests inline in `assistant_cef_config.rs`,
vendor `runtime_identity.rs` and existing Linux callback modules.

**Interfaces — define these exact owned types:**

- Vendor `runtime_identity.rs`: `RuntimeIdentity { cef: [u32; 3], chromium: [u32; 4], api_version: i32, platform_api_hash: String, cef_commit: String }`.
- `RuntimeIdentity::from_parts(versions: [i32; 8], api_version: i32, platform_api_hash: &str, cef_commit: &str) -> Result<Self, &'static str>`: std-only checked conversion of exported entries0–7; entry3 is the CEF commit number, Chromium is entries4–7. Reject negative components, nonpositive API and non-40-byte-hex hashes. Unit fixture `[154,0,34,3633,154,0,8037,98]` maps to the arrays above; a negative component or malformed hash returns Err.
- Vendor `CefRuntime::inspect_identity(cef_path: &Path) -> Result<RuntimeIdentity, WeldError>`: load the configured runtime through existing loading logic and inspect before `cef_initialize`; do not create a runtime owner or call shutdown.
- App config module: `HostBackend::{LinuxWayland,LinuxX11,Windows,Macos,Unknown}`, `Switch { name: String, value: Option<String> }`, `RuntimeIdentity` with the same scalar/string fields (adapter conversion, no vendor dependency in this std-only module), `EffectivePolicy { backend: HostBackend, osr_only: bool, identity: Option<RuntimeIdentity>, switches: Vec<Switch> }`. These owned value types derive Clone/Debug/PartialEq/Eq; log only the selected safe fields, not a Debug dump of switches.
- `PolicyError::{InvalidSwitch,FeatureConflict,OzoneConflict,UnknownRuntime,InvalidScope,AlreadyInitialized,ConflictingRuntime}`.
- `parse_overrides(raw: &str) -> Result<Vec<Switch>, PolicyError>`.
- `build_policy(host: HostBackend, osr_only: bool, identity: Option<&RuntimeIdentity>, overrides: &[Switch]) -> Result<EffectivePolicy, PolicyError>`.
- `claim_runtime(slot: &std::sync::OnceLock<EffectivePolicy>, policy: &EffectivePolicy) -> Result<(), PolicyError>`: first claim succeeds; identical second claim is AlreadyInitialized; differing claim is ConflictingRuntime. Never initialize CEF twice or shut down another owner's runtime.
- App integration: `host_backend(cc: &eframe::CreationContext<'_>) -> HostBackend`; change `CefAssistant::new(render_state: &eframe::egui_wgpu::RenderState, backend: HostBackend) -> Option<Self>`.
- Vendor `NavigationEvent::BrowserCreated { browser_id: i32, windowless: bool }` is emitted by Linux life-span on_after_created through the existing event queue; current NewWindowRequested still represents denied browser popups. The app logs only browser identity/mode/action, not target URLs.

- [ ] **1. Write policy RED tests.** Name the tests `merged_features_preserve_translate`, `feature_enable_trial_conflicts`, `backend_identity_and_claims_are_scoped`, `diagnostic_overrides_are_bounded`. Assert merged names exactly `Translate,WaylandFractionalScaleV1` for LinuxWayland+OSR, duplicates collapse, an extra disabled feature survives, and other platforms do not acquire this feature. Assert enabled `WaylandFractionalScaleV1` and `WaylandFractionalScaleV1<Trial` conflict; Ozone=x11/headless conflicts in Wayland scope; Unknown backend/unknown identity refuse. First/same/different runtime claims follow the errors above, including a different runtime identity. Reject production B overrides requesting remote-debugging listeners or native DevTools opening; test scripts use IPC/control, not an exposed CDP port. Bound legacy raw input to16 KiB/64 switches, each feature/switch value to1024 bytes; switch names are lowercase ASCII/digits/hyphens. Reject ambiguous bare CamelCase feature tokens instead of pretending comma-delimited feature lists were parsed.
- [ ] **2. Run the config source test RED** using the cheap harness; save the failing assertions/exit status, not just compilation errors from unavailable dependencies.
- [ ] **3. Vendor and implement the minimum policy/identity adapter.** Add only `welding = { path = "vendor/welding" }` to root patches; run `cargo update --offline -p welding --precise 0.15.0` and inspect the lock diff; stop/reconcile rather than accepting any unrelated version change. Query CEF's actual exported `cef_version_info(0..=7)` and existing `api_hash(15400, 0/2)` through a safe vendor wrapper; any manual FFI declaration/owned C-string conversion is isolated to this vendor file. Native querying is cfg-gated to Linux+cef-runtime; its owned identity data/validation tests stay std-only. Check CEF `[154,0,34]`, Chromium `[154,0,8037,98]`, API `15400`, Linux platform hash `5c8344ae7d7f6421bc801401e35d4dbf6d38e101`, commit `14c5a089e8452874cb1a556dc3fde26ed2d87723`. Null/malformed/unknown identity refuses B; do not hardcode a purported observed result. Gate identity tightening on LinuxWayland only, leaving existing Windows/macOS runtime behavior intact.
- [ ] **4. Wire actual backend and request scoping.** Use `welding::wgpu::rwh::{HasDisplayHandle,RawDisplayHandle}` on CreationContext; no new raw-window-handle dependency is necessary. Build effective Ozone Wayland/feature configuration once before initialize; merge feature values structurally, recognizing `<trial`/`:parameters` on enabled names. Tightened override/identity/claim rules apply to B only; preserve the existing non-B platform defaults/parsing/init behavior. Log only effective backend, OSR, known feature names and runtime identity, never arbitrary override values or environment contents. Keep root `windowless_rendering_enabled=1`, existing on-before-popup cancellation and host context-menu routing. Do not add ShowDevTools/native-window entrypoints; verify created hosts with existing `is_window_rendering_disabled()` as an additional invariant, not sole prevention. Retain select/fullscreen child OSR. Reuse existing failure UI/CAD survival; do not silently switch backend.
- [ ] **5. Run GREEN and source-scope checks.** Config source tests must pass. Add native wrapper/callback tests for missing identity and window.open/target-blank vs PET_POPUP; these require real vendor crate compilation and remain BLOCKED if the permitted cache cannot run them cheaply. Search app/vendor for `show_dev_tools`, `browser_host_create_browser`, `remote-debugging-port` and review all callers/production defaults; a grep alone does not prove runtime denial.
- [ ] **6. Reviewable checkpoint.** Report identity/scoping behavior, exact vendor/license/lock changes, actual RED/GREEN and blocked integration checks. Controller coordinates review and commit. No native CEF notice change for the flag; mark welding modified and provide its source obligations.

### Task 2: Owned native geometry and import pairing

**Files:** Task 2 entries; existing native-frame/DMABUF ownership tests plus inline
tests in new `frame_geometry.rs`.

**Interfaces (public in `welding::frame_geometry`):**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRect { pub x: i32, pub y: i32, pub width: i32, pub height: i32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameGeometry {
    pub coded_size: [i32; 2], pub visible_rect: FrameRect,
    pub content_rect: FrameRect, pub source_size: Option<[i32; 2]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintElement { View, Popup }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureMetadata {
    pub element: PaintElement, pub geometry: FrameGeometry,
    pub timestamp_micros: u64, pub capture_counter: Option<u64>, pub generation: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameWatermarks { pub view: u64, pub popup: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryError { InvalidSize, InvalidRect, MissingSource, NotFullSurface }
```

- `FrameGeometry::validate_full_surface(&self, max_edge: u32) -> Result<[u32; 2], GeometryError>` returns coded dimensions only if nonzero/bounded, source equals coded, and content/visible are the full allocation at origin; checked arithmetic.
- `DmaBufImage::with_capture_metadata(self, metadata: CaptureMetadata) -> Self`;
  `NativeFrame::capture_metadata(&self) -> Option<CaptureMetadata>`.
- Add `pub capture_metadata: Option<CaptureMetadata>` to ImportedTexture; initialize None in unchanged Windows/macOS/CPU import paths, transport the Linux record unchanged. Existing constructors default None.
- `PendingFrameSlot::discard(&mut self) -> u64` drops its frame and returns the current generation high-watermark without resetting it.
- Linux-only `LinuxCefProducer::discard_pending_frames(&mut self) -> Result<FrameWatermarks, WeldError>` drains view and popup mailboxes; no driver wait or frame allocation.

- [ ] **1. Write geometry RED tests.** Use `full_capture_is_valid`, `letterboxed_and_oversized_coded_frames_are_refused`, `hostile_geometry_never_panics`, `popup_is_its_own_surface`. A full `[463,1123]` record passes; coded `[900,600]`, content `(326,0,248,600)`, source `[463,1122]` fails; coded `[579,1403]`, full coded content but source `[463,1123]` fails. Missing source, negative/zero/over-limit sizes, negative/outside/overflowing rectangles fail. Full popup geometry still passes as its own surface, not a page-sized texture.
- [ ] **2. Run geometry test RED**, then implement the std-only validator and run GREEN. No GPU surrogate types.
- [ ] **3. Write pairing/drop regressions before wiring.** In existing vendor tests assert attaching/reading metadata retains the native generation and owned descriptors, missing metadata stays None, replacing/discarding a mailbox drops descriptors once, and generation does not reset after discard. Preserve existing fd-table test lock. These real native-frame unit tests must run RED/GREEN when a permitted cached vendor build is possible; otherwise record BLOCKED, not a fake stub-based pass. Task3's std-only mismatch test covers the admission comparison; Task4's real GPU receipt covers actual native→import propagation, not a stubbed importer.
- [ ] **4. Wire callback and importer.** Copy only existing `info.extra` scalar fields/validity in Linux CEF callback, with actual slot generation and View/Popup kind. Reject impossible signed coded sizes before conversion/duplication. No CEF ABI change. Attach metadata to the owned frame, carry it through Linux import and all popup imports, recover lock errors via Result, and drain old view/popup slots on reset. Keep generations monotonic on discard; saturate on exhaustion and refuse repeated identities rather than panic/wrap. Do not stamp app placement or lifetime onto callback metadata as native provenance.
- [ ] **5. Re-run cheap GREEN and inspect ownership diff.** Verify all ImportedTexture initializers compile at the eventual crate gate; no changes to foreign queue synchronization, fd duplication/import ownership, Windows/macOS behavior beyond None initialization, or GPU copy mechanics.
- [ ] **6. Hand off checkpoint** with metadata fixture results and explicit deferred native ownership/compile checks; update vendor change list/MPL source record.

### Task 3: Native retry, page/theme-before-texture and input admission

**Files:** Task 3 entries. Retain and repurpose `assistant_cef_viewport.rs` rather
than deleting its tested native retry/scale-only/error semantics.

**Interfaces:**

- Linux-only vendor `LinuxCefProducer::set_page_focus(&mut self, focused: bool) -> Result<(), WeldError>` calls the existing browser-host focus API. `move_focus(Backward)` is not blur: current welding sets focus true for both directions. No focus API change on Windows/macOS.
- Existing viewport Metrics stays `([u32; 2], f32)`. `ViewportState::new(now: Instant) -> Self`, `needs_update(&self,target: Metrics,now: Instant) -> bool`, `begin(&mut self,target: Metrics)`, `finish(&mut self,target: Metrics,success: bool,now: Instant)`, `reset(&mut self,now: Instant)`. No CDP IDs/receipts. Begin invalidates old applied state; failed partial setter transitions retry after 500 ms.
- New std-only `assistant_cef_frame.rs` imports `welding::frame_geometry`:
  `Placement { origin_px: [i32;2], physical: [u32;2], scale: f32 }`,
  `InputTicket { lifetime: u64, placement_revision: u64 }`,
  `ThemeTicket { lifetime: u64, revision: u64 }`,
  `FrameReject::{InvalidTarget,MissingMetadata,Geometry(GeometryError),WrongElement,WrongSize,OldGeneration,ImportMismatch,ThemePending,IdentityExhausted,Timeout}`.
- `FrameAdmission::new(max_edge: u32) -> Self`;
  `reset(&mut self,lifetime: u64,placement: Option<Placement>,floors: FrameWatermarks,now: Instant) -> Result<(),FrameReject>`;
  `theme_pending(&mut self,ticket: ThemeTicket,now: Instant) -> Result<(),FrameReject>`;
  `theme_applied(&mut self,ticket: ThemeTicket,floors: FrameWatermarks,now: Instant) -> bool`;
  `check_view(&self,metadata: &CaptureMetadata) -> Result<(),FrameReject>`;
  `admit_view(&mut self,metadata: &CaptureMetadata,imported_size: [u32;2],imported_generation: u64) -> Result<InputTicket,FrameReject>`;
  `check_popup(&self,metadata: &CaptureMetadata,physical_rect: FrameRect,imported_size: [u32;2],imported_generation: u64) -> Result<(),FrameReject>`;
  `input_ticket(&self) -> Option<InputTicket>`;
  `permits_input(&self,ticket: InputTicket) -> bool`;
  `retry_due(&mut self,now: Instant) -> bool`;
  `pending_error(&self,now: Instant) -> Option<FrameReject>`.
  Retry requests at most four repaints, spaced ≥500 ms; after 2 s pending report Timeout and remain backed/noninteractive. A later compatible frame or explicit existing Retry can recover; no busy loop.

- [ ] **1. Write RED state tests and run them.** Use `native_ceiling_extent_is_not_resampling`, `aba_clears_known_frames_and_input_tickets`, `stale_theme_ack_cannot_initialize_new_page`, `pending_has_bounded_retry_and_no_input`. Check ceil extents at all four scales, 463×1123 accepted for physical463×1122/1.25 with one-row clip, 370×898 rejected there. A→B→A resets local admission/floors/tickets even for equal final dimensions; known queued/repeated generations and mismatched import identities refuse. Invalid/NaN scale, same-size origin changes, palette/reload stale ThemeTicket, timeout/retry bounds and old input tickets refuse.
- [ ] **2. Implement only those state transitions; run direct-source GREEN.** Every reset/theme invalidation advances the checked app-local admission revision (named placement_revision in InputTicket); overflow returns IdentityExhausted and stays noninteractive. Require finite scale in [0.25,8], nonzero bounded physical size and checked ceil arithmetic. This is an input/queue ticket, not native renderer-commit proof. Current theme_applied updates drained-generation floors and starts its bounded frame-wait deadline without discarding its theme acknowledgement or restarting the admission revision; retries never restart deadlines. Test and document that a late same-size A frame with a newer capture identity remains indistinguishable. Do not turn that limitation into an invented epoch guarantee.
- [ ] **3. Wire theme initialization independently of texture.** On ready/reload install THEME_BRIDGE_JS and send latest theme/state without `self.texture.is_some()`. Add one bounded acknowledgement to the existing `/ipc` bridge: `{"type":"theme-ready","ticket":"<lifetime>:<revision>"}` emitted after the applier finishes. Limit ticket parsing to two checked u64s/41 ASCII bytes; ignore stale/unsolicited acknowledgements. On current ack, drain known queued frames, update floors and request native repaint before admission. No new channel/framework. Preserve independent palette updates even when chat state is unchanged.
- [ ] **4. Wire native admission and input.** Strict geometry admission applies to B's LinuxWayland path; absent metadata there is MissingMetadata, not a legacy bypass. Explicit other-platform paths retain their native transport behavior and None metadata, with no B feature injection. Validate view before import, recheck actual imported extent/generation afterward, then publish. Existing acquire_popup imports internally: check its transported metadata/identity before publishing; require current admitted view, Popup kind, full source/content, generation above popup floor and dimensions within one pixel of its physical popup rect (round/ceil difference), with no texture stretching. On placement/lifetime/theme reset, hide old page/popup admission, drain slots, clear InputQueue/motion, call ime_cancel_composition and set_page_focus(false), clear app focus and disable host IME. Pending clicks/wheel must not focus or interact with an invisible page; ambiguous held input is cancelled, not replayed. Keep the existing queue capacity256 for current admitted input. Only current InputTicket can flush; no app physical→DIP double conversion. Test an admitted button/key down followed by reset and delayed release for stuck drag/modifier state; runtime failure blocks acceptance, not a reason to send stale clicks.
- [ ] **5. Remove density emulation, keep native behavior.** Native resize + set_scale_factor feed synchronous setter success into ViewportState; frame readiness is separate. Remove Emulation.setDeviceMetricsOverride, CDP viewport ack machinery and protocol enablement used solely for it. Existing clipboard script requests stay; no native DevTools window/listener appears. Preserve gamma-copy/pixel-aligned draw and token backing. Existing Retry resets failure/retry state, not just told chat text.
- [ ] **6. Verify/review checkpoint.** Direct config/geometry/viewport/admission GREEN; existing app tests cover wiring when compilation is permitted. Retain readable-font and rail tests. Do not claim source-module harnesses prove actual callback/IME integration.

### Task 4: Repeatable offline fixtures and identified runtime evidence

**Files:** Task 4 entries; tests via `native-density.py --self-test` and the
existing probe fixture/input/theme scripts, no committed screenshots or tokens.

**Interfaces / receipt contract:**

- App logs bounded JSON `CEF_RUNTIME` with backend, osr_only, merged disabled feature names, cef/chromium/API/commit identity; `CEF_BROWSER` with browser_id, windowless, action (`created`/`popup_denied`); `CEF_FRAME` with lifetime, local placement revision, element, generation, capture_counter, requested_scale, dip, physical, coded, visible, content, source, imported_size, imported_generation, decision/reason. Never label requested_scale as an independently observed native getter.
- `native-density.py`: `validate_receipts(records: list[dict], expected_scales: tuple[float,...]) -> dict`; `capture_bundle(app: Path, cef_root: Path, pid: int) -> dict`. JSON reports use `status: PASS|FAIL|BLOCKED|NOT_RUN`, `checks`, `artifacts`, `limitations`. Hash whole files, inventory resources and actual Linux `/proc/<pid>/maps` library path; parse PID/starttime correctly. Do not trust a supplied CEF_PATH or `.text` alone.
- CLI: `--self-test`; offline `--check-log PATH --manifest PATH --out PATH`; live evidence `--app PATH --cef-root PATH --out DIR --task NAME`. Live mode owns private settings/control, bounded deadlines and authenticated quit; no host OS input/capture.
- App test hook: `CefAssistant::inject_scope_probe(&mut self) -> Result<(), welding::WeldError>`, enabled only by `SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE=1`. Inject fixed in-source controls/listeners through the existing execute_script test-hook pattern, once per page lifetime; no arbitrary URL/file/script parameter. Emit DOM rects/results via existing `/ipc` log messages so app-control clicks can use the current pixel-aligned origin. Extend the existing POC HTML with the same test IDs for standalone probe use; do not claim that HTML is automatically the app's shipped page.

- [ ] **1. Add failing self-tests and run RED.** Reject missing metadata/runtime identity, import/source mismatch, missing scales, incomplete 25-theme set, altered whole-file hash with same text hash, mismatched resources, and unknown/blocking statuses. Fixture A→B→A explicitly reports the stated late-same-geometry limitation, not causal-freshness PASS.
- [ ] **2. Implement the minimal parser/provenance collector; run GREEN.** Use stdlib only; raw output and transformation records retained locally. A llvmpipe/no-interop run marks accelerated steps BLOCKED even when graceful refusal passed; overall must not become PASS.
- [ ] **3. Extend existing fixture/input scripts and fixed app test hook.** Add window.open/target-blank, select popup, fullscreen enter/exit, JS-dialog current cancellation, forbidden native-DevTools entrypoints and multiple-root/process scoping assertions. Add startup/reload/resize/scale-only and origin-change sequences; key/IME during pending must not reach old content. Keep OSR popup textures. Record any unavailable real OS IME candidate test as BLOCKED.
- [ ] **4. Extend theme matrix/lifecycle evidence.** Enumerate all25 themes from source; check computed prose/textarea/label sizes (12.5 for reviewed roles, root12), token/rail colors, states/transparency/swatches and lifecycle. Remove reliance on WINIT_X11_SCALE_FACTOR for Wayland: use authenticated `ui.prefs {"ui_scale":...}` to exercise producer scales and verify actual `ui.rects.pixelsPerPoint` reaches1/1.25/1.5/2 (derive native base from measured ppp/current ui_scale). Label this UI-scale-driven coverage, not actual compositor/monitor movement; unavailable real monitor transitions stay BLOCKED. No external device-metrics override. Record requested scale vs actual capture/source/import density; do not substitute screenshot dimensions for native evidence.
- [ ] **5. Run only offline/self-tests now.** `python3 -B experiments/cef-osr/scripts/native-density.py --self-test`; parse all changed Python using `ast.parse` without launching an app. Live fresh-artifact matrix remains BLOCKED until build/resources/VM prerequisites permit it.
- [ ] **6. Hand off source/evidence-script checkpoint** with schema, failure fixtures, all real command exits and native/live gaps for independent review.

### Task 5: Focused integration gates, license closure and truthful delivery

**Files:** Task 5 entries and the reviewed diffs from Tasks 1–4.
**Consumes:** exact interfaces above; produces a revision-bound evidence index,
not an integration service or a fabricated acceptance artifact.

- [ ] **1. Verify source checks.** Re-run the five exact direct-source test commands/harness (config, identity, geometry, viewport, admission), Python self-test/AST parse, and `git diff --check`. Record Rust version/input hashes. Review the root Cargo.lock diff: only welding source selection, no unrelated version churn.
- [ ] **2. Reuse unchanged font/backing evidence honestly.** If their source inputs are unchanged, run an explicitly identified existing cached UI test binary with filters `the_web_panel_rail_uses_the_native_theme_not_the_pages_startup_surface`, `the_web_theme_exposes_readable_native_prose_and_label_sizes`, `every_builtin_theme_produces_a_complete_palette`. Record the binary/hash/revision; it does not compile or validate new app/vendor code.
- [ ] **3. Record deferred real compile/TDD gates.** When, and only when, a resource-permitted check is authorized: `CARGO_INCREMENTAL=0 cargo test --locked -p welding --lib --features cef-runtime`; `CARGO_INCREMENTAL=0 cargo test --locked -p solidcraft --bin intcad assistant_cef`; `CARGO_INCREMENTAL=0 cargo test --locked -p solidcraft-ui-egui --lib agent_chat`; `CARGO_INCREMENTAL=0 cargo check --locked -p solidcraft --bin intcad --target x86_64-pc-windows-msvc`; the same check with `--target aarch64-apple-darwin` on a suitable provisioned toolchain; `cargo xtask licences --check`, `cargo xtask lists`, `cargo xtask layers`, `cargo xtask skills`. No cross toolchain installation is authorized; missing SDK/target is BLOCKED. Do not run Cargo to discover whether it triggers a prohibited rebuild. Capture actual RED/GREEN for tests that could not run earlier.
- [ ] **4. Close source/license records.** Retain original MPL headers, mark changes `SolidCraft:`, record modified welding source availability and license text, amend the unmodified-welding notice, add the attribution-pending row. Stock CEF notices remain stock; generated credit/license catalogs are not manually edited. Describe provenance mismatch and workaround upgrade/removal ownership.
- [ ] **5. After resources permit, validate the matched bundle.** Fresh trusted app artifact + authenticated stock CEF; full hashes/config/loaded paths/subprocess resources/transforms; stable VM task first, actual1/1.25/1.5/2 callback/import matrix and input/theme checks, host last/manual visual approval. Use `native-density.py --app "$APP" --cef-root "$CEF_ROOT" --out "$E/native" --task cef-dsf`; existing theme-matrix/lifecycle scripts take that same app/bundle identity. No old c14… VM receipt is reused for changed bytes.
- [ ] **6. Controller delivery gate.** Independent task and whole-branch spec/quality review plus L3 review; fix valid findings and rerun affected checks. Report source-ready separately from binary-validated/accepted, revision/dirty/unpushed state and every blocker. Controller chooses meaningful checkpoint commits; no push/merge/public action is implied.

## Plan self-review and handoff

- Coverage: config/scope→Task1; owned metadata→Task2; theme/readiness/input/native retry/CDP removal→Task3; all-scale/browser/theme/provenance fixtures→Task4; build/license/VM-host/independent gates→Task5.
- Interfaces use one optional CaptureMetadata record carried through import; no new CEF ABI, frame protocol, causal epoch or stock-native fork.
- Same-size late-frame ambiguity and missing real GPU/OS-IME evidence are explicit blockers/limitations, not green assertions.
- Execution method is already selected: **controller-orchestrated subagents** with independent review. No user model/command/strategy selection is requested.
- Stop here for controller plan review. No product edits, vendoring, tests/builds or commits have been performed by writing this plan.
