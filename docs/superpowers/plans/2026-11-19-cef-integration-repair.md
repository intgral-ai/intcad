# CEF Integration Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans
> for the single consolidated owner, sequentially. Steps use checkbox (`- [ ]`)
> syntax. Do not dispatch parallel per-finding implementations/reviewers.

**Goal:** Repair all thirteen final integration findings in one coherent wave,
without changing Agent/MCP business contracts or sacrificing CAD correctness.

**Architecture:** Dedicated document-scoped private HTTP capability; optional
device factory after CAD adapter/descriptor selection; explicit native close
completion before runtime shutdown and host quit. One owner carries these
interfaces through input, rendering and evidence tools.

**Tech Stack:** Rust 1.99.1, egui/eframe/egui-wgpu 0.36, wgpu 30, local welding
0.15 / grafting 0.6.2, pinned CEF 154 bindings; std-only Python/shell evidence tools.

**Spec:** `docs/superpowers/specs/2026-11-19-cef-integration-repair-addendum.md`,
plus approved sections 1–5 of `2026-11-18-cef-wayland-dsf-design.md`.

## Implementation authorization

The user approved both additional scope decisions after the independent builder
check and scoped R1 written-contract review passed. Execute this reviewed plan
as one consolidated owner/wave. Historical DRAFT and planning-stage labels refer
to the prior approval gate, not permission to reopen settled choices or expand
the exact scope. Native enablement/acceptance remains proof-gated.

## Global Constraints

- Frozen input `c355edb977cd828f8d6a2f2de7bbdde792f8d1df`, worktree
  `/home/hyjing/Code/intgral/intcad/worktrees/cef-osr`, branch `probe/cef-welding-osr`.
- The written-spec/plan gate is complete. Product source edits and local WIP
  checkpoints within this exact wave are authorized; public push, merge,
  prohibited builds, unapproved experiments and cleanup remain unauthorized.
- Human-approved principles are documented, not blanket approval of new detail.
  Controller arranges an independent builder check before implementation.
  Revision B3 addresses `integration-builder-check-2.md` Q1–Q7 using the
  controller rulings and completed pinned excerpts; no builder approval is
  claimed. HAL dependency repair and the startup-false
  policy are separately marked **DRAFT extensions**, not authority inferred
  from the approved hook/close-timeout recommendation or approved defaults.
- R1-only follow-up addresses independent `integration-builder-check-3.md`:
  state-bound quit consent, not another dependency/native policy decision.
  Both DRAFT human approval sentences and all other contracts remain unchanged.
- One consolidated repair owner, nine sequential work units, one scoped
  independent Sol re-review of the final fix against the existing broad verdict.
  Checkpoints are not nine separate implementation/review dispatches.
- Serialized business payload ≤400,000 bytes; send text ≤100,000 scalar values;
  attachments/external-link behavior and genuine OSR widgets preserved.
- No CAD adapter/backend substitution or descriptor reductions; no overlapping
  CEF runtime, forced product exit, new WebView/XWayland/CPU Assistant fallback.
- No unsafe/panic/unwrap/expect in new app code; native unsafe stays behind
  reviewed vendor boundaries. Hostile values use checked/fallible operations.
- No per-theme product branches, CSS zoom, CDP metrics emulation or CPU readback.
- New UI strings: `assets/locales/pending/cef-integration-repair.json`.
  Attribution pending only; no hand-edit/fold of generated catalogs/credits.
- No CI/public push, new LGPL exception, registry mutation or provider/billing
  action. No second cleanup. Developer ≥180/≥200 GiB and ≥150 GiB free-space rules
  remain binding; prior user budget-waiver acceptance was corrected.
- Only cheap source-module/static checks are available now. Full Cargo/native/
  wasm/Windows/macOS/GPU/VM/OS-IME/host proof remains BLOCKED, not waived.

## Review Focus

These are integration traps not covered by the historical pure-module suites.
Each is assigned a new test below; native counterparts remain gated.

1. An old document sends a valid-looking approval just as replacement starts:
   revoke authority atomically and discard queued old work (W1/W3).
2. A native device opens and wrapper construction fails: release every temporary
   owner before ordinary CAD device creation; do not infer cleanup from Err (W2).
3. A late browser-created/before-close event arrives after quit timeout: preserve
   CAD and ownership, never revive cancelled quit or Retry (W3).
4. Pointer motion coalesces across down/up and a non-BMP composition moves with
   popup/placement: keep order, units and ticket validity (W4).
5. A perfect theme receipt accompanies unrelated PID/exe or missing effective DSF:
   incomplete identity/density can never PASS (W6/W7).

---

## File ownership and interface map

All listed ownership is **one implementer**, sequentially. New files below are
small production seams, not independent frameworks or bug-specific test files.
Existing tests stay next to their owning implementation or in the existing
area test file. Line references are frozen-base navigation, not patch offsets.

| Unit | Exact files / responsibility |
|---|---|
| W1 | New `apps/solidcraft/src/assistant_cef_http.rs`: bounded std-only HTTP server, authority/atomic navigation permit, inbox, cancellation and inline tests. `assistant_cef.rs`: constructor/inbox/URL/shim/log sinks and private-host runtime settings; existing `assistant_cef_config.rs`: private-HTTP override containment and tests. `main.rs`: desktop-gated HTTP module declaration. `assistant_page.rs`: existing file resolver to server setup, no bundle rebuild. New `vendor/welding/src/request_policy.rs`: narrow policy event types; `surface.rs`, `lib.rs` and all `{linux_cef,macos_cef,windows_cef}/{mod.rs,cef_backed.rs}`: synchronous life-span binding and actual pre-navigation/resource callbacks. |
| W2 | `vendor/egui-wgpu/src/{setup.rs,lib.rs}`: optional factory and selected-descriptor invocation. `apps/solidcraft/src/{main.rs,startup.rs,assistant_cef.rs}`: retain graphics configuration, install eligible factory, interop refusal. `vendor/welding/src/native_frame/vulkan_dmabuf.rs`: helper boundary/cleanup contract if needed. **Only if (a) approved:** faithful `vendor/wgpu-hal/` 30.0.1 archive input with licences, local delta/tests only in `src/vulkan/adapter.rs`, root `Cargo.toml` patch and `Cargo.lock` same-version path-source switch. No registry edits. W8 owns its inventory/notices. |
| W3 | New `apps/solidcraft/src/assistant_cef_lifecycle.rs`: pure transition/timeout policy with inline tests. `assistant_cef.rs`, `main.rs`: desktop-gated lifecycle module declaration, native owner bundle/quit/None-Retry integration; conditional DRAFT early runtime gate at AppCreator entry and typed normal-return startup cancellation. Conditional `apps/solidcraft/src/startup.rs`: owned startup tickets/ledger and legacy-counter import; `main.rs`: ticket transfer/controlled abort vs real second-frame success; `crates/engine/src/crash.rs`: owned journal ticket, checked end-session helper/terminal cancellation parsing, existing tests. `vendor/welding/src/runtime.rs`: unique runtime, conditional narrow existing `cef::sys::cef_initialize` call and non-dropping raw-input/loader ownership in its native module; no generated-binding/binary fork or new dependency. `surface.rs` and three platform producer/callback pairs: completion. `crates/ui-egui/src/{lib.rs,documents.rs,control.rs}`: host-close status, staged discard/guards, Outcome-based outer refusal and endpoint-specific additive hostClose. `agent_chat/{web.rs,tests.rs}`: retry status/UI without conversation loss. `docs/control-protocol.md`, locale pending: documented result/status. |
| W4 | `assistant_cef.rs` input/IME and its existing tests; `vendor/welding/src/{cef_input.rs,ime.rs,surface.rs}` and three `cef_backed.rs`: UTF-16/bounded composition; existing `view.rs` conversion tests. |
| W5 | `assistant_cef.rs` existing page/popup target ownership; `crates/ui-egui/src/agent_chat/mod.rs` cfg; main desktop cfg. Tests in these existing modules and `agent_chat/tests.rs`; no fake GPU test module. |
| W6 | `vendor/welding/src/view.rs`, `surface.rs`, three producer modules: requested metrics snapshot. `assistant_cef.rs`, `assistant_cef_frame.rs`, `assistant_cef_viewport.rs`: placement/request/receipt integration; `experiments/cef-osr/scripts/native-density.py`: density/ABA validators and existing selftests. |
| W7 | `assistant_cef.rs` fixed hook; `experiments/cef-osr/poc/fixtures/probe.html`; `experiments/cef-osr/scripts/{app-input.py,app-ime.sh,native-density.py,theme-matrix.py}`: schema, identity, ownership, report cleanup and existing selftests. |
| W8 | `vendor/{README.md,welding/UPSTREAM.md}`, `NOTICE`, `docs/{cef-assistant-visuals.md,cef-welding-cutover.md,cef-welding-production-readiness.md,web-view-licences.md}`, `packaging/notices/WEB-VIEW-PLATFORM-NOTICES.txt`: accurate existing source inventory, inline historical labels and explicit minor deferrals. In `attribution/pending/cef-wayland-welding.md` remove ONLY the `vendor/welding/` CODE row; preserve the four-column header and `vendor/welding/LICENSE.txt` asset row unfolded. If (a) approved, add HAL code origin/licence/delta to existing vendor README inventory/notices and only original licence-text asset rows via existing four-column pending format. No pending parser/README/format/workflow change or direct ATTRIBUTION edit/fold. Canonical Vendored-table curation deferred. |
| W9 | Existing SDD `progress.md` and repair evidence under its `final-review/repair/` local directory: fixed-revision evidence/status and handoff, not new product infrastructure. No raw secrets/media committed. |

R1 remains in **W3, the same owner/unit**: `crates/engine/src/lib.rs` adds only
the opaque QuitSessionSnapshot/read-only capture+comparison accessors, using
existing state/counters; existing session/computed-command test areas exercise
them. `ui-egui/src/{documents.rs,lib.rs,control.rs}` owns typed quit consent,
validation/cancellation/terminal close latch; existing `{control_job.rs,
dialogs.rs,file_load.rs,save_job.rs,deferred.rs,feature_edit.rs,edit_in_place.rs}`
owns checks around its real owner-thread completion/adoption and pending-work
inspection, not a new worker/transaction framework. `main.rs` moves final
validation/Close to App::ui's tail and guards post-commit logic/UI/on_exit.
Existing documents/control/save-job/completion test modules contain R1 cases.

No Cargo change is presently authorized. **Conditional exception only if human
approves (a):** root `[patch.crates-io] wgpu-hal = { path = "vendor/wgpu-hal" }`
and Cargo.lock switch of locked **30.0.1** from registry to that faithful patched
path source; no version/dependency/feature churn. The full proposed user sentence
is addendum §4, decision (a). Same owner performs W2 source/Cargo and W8
provenance/licences, not an extra unit. Update vendor provenance for **all**
changed vendor paths, including new policy/runtime/callback/IME changes, not the
old eleven-file list alone.

## W1 — Authorize and bound the complete page transport

**Consumes:** real `assistant_page::page_file`, `web::act` body contract,
`getrandom::fill`, `Pictures` Arc bytes, actual CEF callback signatures.

**Produces (proposed exact interfaces):**

In `assistant_cef_http.rs`, std-only:

```rust
pub(super) struct DocumentId(pub u64);
pub(super) struct BrowserOwnerId(pub u64);
pub(super) struct NavigationIdentity {
    pub document: DocumentId,
    pub owner: BrowserOwnerId,
    pub browser_id: i32,
    pub url: String,
}
pub(super) struct IpcEnvelope { pub document: DocumentId, pub body: String }
pub(super) enum ServerProgress { Running, Stopping, Stopped, Failed }
pub(super) struct PageServer { /* all socket/thread/queue owners, private */ }
pub(super) struct DocumentAuthority { /* synchronized current lease/phase */ }
pub(super) struct PageAssets {
    pub resolve: fn(&str) -> Option<(&'static str, &'static [u8])>,
    pub font: &'static [u8],
}
impl DocumentAuthority {
    pub fn new() -> Self;
    pub fn reserve_browser_owner(&self) -> Result<BrowserOwnerId, TransportError>;
    pub fn begin_document(&self, owner: BrowserOwnerId, random: [u8; 32])
        -> Result<DocumentId, TransportError>;
    pub fn bind_browser(&self, owner: BrowserOwnerId, browser_id: i32)
        -> Result<(), TransportError>;
    pub fn bound_browser(&self, owner: BrowserOwnerId)
        -> Result<Option<i32>, TransportError>;
    pub fn check_inert_navigation(&self, owner: BrowserOwnerId,
        browser_id: i32, target_url: &str, main_frame: bool, is_redirect: bool)
        -> Result<(), TransportError>; // never returns DocumentId/authority
    pub fn arm_main_navigation(&self, identity: NavigationIdentity)
        -> Result<(), TransportError>;
    pub fn navigation_identity(&self, owner: BrowserOwnerId, browser_id: i32,
        target_url: &str) -> Result<NavigationIdentity, TransportError>;
    pub fn claim_main_navigation(&self, identity: &NavigationIdentity,
        main_frame: bool, is_redirect: bool) -> Result<DocumentId, TransportError>;
    pub fn check_resource(&self, owner: BrowserOwnerId, browser_id: i32,
        target_url: &str, is_image: bool) -> Result<DocumentId, TransportError>;
    pub fn authorize_http(&self, head: &RequestHead)
        -> Result<DocumentId, TransportError>;
    pub fn revoke(&self);
    pub fn permits(&self, document: DocumentId) -> bool;
    pub fn dispatch_lease(&self, document: DocumentId)
        -> Result<DispatchLease<'_>, TransportError>;
    pub fn bootstrap_url(&self, document: DocumentId, port: u16)
        -> Result<String, TransportError>;
}
impl PageServer {
    pub fn start(assets: PageAssets, pictures: Pictures,
        authority: std::sync::Arc<DocumentAuthority>) -> std::io::Result<Self>;
    pub fn port(&self) -> u16;
    pub fn take_batch(&mut self) -> Result<Vec<IpcEnvelope>, TransportError>;
    pub fn request_stop(&mut self);
    pub fn poll_stop(&mut self) -> ServerProgress;
    pub fn take_failure(&mut self) -> Option<TransportError>;
}
fn parse_head(bytes: &[u8], expected_host: &str)
    -> Result<RequestHead, TransportError>;
fn remaining(now: std::time::Instant, deadline: std::time::Instant)
    -> Result<std::time::Duration, TransportError>;
```

`Pictures` moves to this transport seam unchanged in shape. `TransportError`
enumerates BadHead, BadFraming, Unauthorized, TooLarge, InvalidUtf8, QueueFull,
StaleDocument, BrowserMismatch, PermitNotArmed, PermitConsumed, Deadline,
Allocation, Stopped and Io; diagnostics expose only a
constant error category. `RequestHead` owns validated method/relative route/
length, never unbounded raw headers. Authority has monotonically checked
DocumentId and separate secret bytes, with no Debug-derived secret output.
`DispatchLease<'_>` is the read-side authority guard; revocation is exclusive.
Apply queued business action under this guard, then release it before CEF work
or any socket IO. Defer theme-ready repaint/drain operations until after release.
`take_batch` implements eight/800,000 limits, not caller-selected unbounded caps.

Reserve owner in CreatingInert; binding sets BoundInert. `begin_document` and
arming require the stored bound id; the private permit starts only then.
Store Armed/Claimed/Revoked permit state with the exact
DocumentId/owner/browser/url tuple under the authority lock. Only
`claim_main_navigation` spends Armed, atomically; `navigation_identity` resolves
the target against the stored URL/capability without spending it.
`check_resource`/`authorize_http` require Claimed and never spend/bind/arm.
`permits`/dispatch leases also require current Claimed authority. Neither an
HTTP header nor a business ready message supplies a native browser id.

Initial creation is explicitly inert, not a callback-order assumption:
reserve owner → CreatingInert producer with literal `about:blank` → actual
on_after_created directly binds id/BoundInert → bounded owner-only host
notification → host begin_document/fresh namespace → arm exact tuple → release
lock → existing navigate_to_url. No private URL is passed to create; no blank
ready/load ack is needed. Host also waits for its actual producer handle, so
binding before or after create returns is safe. Lifecycle binding/blank request
order is not assumed. Only exact nonredirect main-frame about:blank for the
captured creation owner is an inert exception; it grants no private permit,
capability/resources, injected shim/scripts/state, IPC or input/paint admission.
After Armed, late blank callbacks cancel without spending the private permit.
Host reload: revoke, begin fresh document, arm on already bound browser, navigate.
Duplicate same-id bind is idempotent; different-id bind/consumed rearm/private
navigation before binding fails. CreatingInert/BoundInert have one 5-second
absolute deadline from creation request; missing notification/binding/navigation
failure revokes and enters W3 owned close, including pending/late creation.
No CEF call under authority lock and no UI-thread wait. Pinned creation/lifespan
docs do not guarantee a general on_after_created-versus-create-return order.
Native callback delivery and real host navigation remain BLOCKED.

In `request_policy.rs`:

```rust
pub enum RequestCheck<'a> {
    BrowserCreated { browser_id: i32 },
    BeforeBrowse { browser_id: i32, main_frame: bool,
        target_url: &'a str, is_redirect: bool },
    Resource { browser_id: i32, target_url: &'a str, is_image: bool },
    Redirect { browser_id: i32, target_url: &'a str, is_image: bool },
}
pub enum RequestDecision { Allow, Cancel, RenewDocument }
pub type RequestPolicy = std::sync::Arc<
    dyn for<'a> Fn(RequestCheck<'a>) -> RequestDecision + Send + Sync
>;
```

Add `CefSurfaceConfig::request_policy: Option<RequestPolicy>`, default None to
preserve other consumers. Assistant supplies a closure over authority. No
generic request router, credential-native transport or arbitrary policy scripts.
Callbacks emit a bounded renewal event without URL/capability. Redirect handler
also uses pre-load cancellation: its void return is not a cancel result.
The closure captures BrowserOwnerId; it is not fabricated as a CEF callback
field. Life-span handler invokes BrowserCreated directly, not through the
asynchronously polled app event queue; it additionally posts/coalesces at most
one owner-only host notification. These are owned policy events, not invented
CEF callbacks. `check_inert_navigation` admits only the inert exception until
Armed, even if the first blank browse precedes binding; all Resource/Redirect
checks still deny blank-page private resources. Request callbacks use existing
`browser.identifier()`, `frame.is_main()`, the Request's target URL and existing
is_redirect argument. Resource callbacks with absent browser/frame/request
(per pinned CEF docs, possible for service workers) cancel; they cannot promote
an unbound document. Redirect uses new_url, not the old request URL.

Production module hierarchy: `main.rs` declares `mod assistant_cef_http;` and
`mod assistant_cef_lifecycle;`, each with the same desktop cfg as assistant_cef.
`assistant_cef.rs` imports these via `crate::assistant_cef_http` and
`crate::assistant_cef_lifecycle`. Its existing nested config module remains
`assistant_cef::config`; no second copy of its implementation is introduced.
The direct-source test wrapper below mirrors those real module paths.

- [ ] **RED: existing `assistant_cef.rs::tests` business seam and new transport
  inline tests.** Existing native input tests do not cover authorization/framing.
  Add the following assertions; they are not claims of present tests:
  - `private_bootstrap_and_every_asset_require_current_capability`: each embedded
    name, shim, Plex and picture with absent/wrong/old capability → no bytes;
    after bind/arm/main-claim current GET succeeds, absent Origin allowed for
    bootstrap GET. Armed-before-claim GET remains refused.
  - `ipc_requires_capability_exact_host_and_origin`: authorized origin+Host+
    current cap passes; no/foreign Origin, localhost/IPv6/Host suffix/duplicate
    Host, foreign port, token in query, and a control-token-shaped value fail.
  - `revocation_discards_queued_ready_and_approval`: enqueue ready and approve,
    revoke, start next document; neither old entry dispatches, old ready cannot
    renew; exercise racing enqueue with std barriers, not timing sleeps.
  - `main_navigation_permit_is_single_use_for_exact_identity`: matching
    document/owner/browser/url/main=true/redirect=false succeeds once; wrong
    document, owner, browser, URL, subframe or redirect fails without consuming.
  - `two_concurrent_main_claims_have_one_winner`: barrier-start two claims for
    the same Armed tuple, exactly one succeeds; replay/rearm is PermitConsumed.
  - `revocation_linearizes_against_navigation_claim`: either claim happens
    first and subsequent revocation denies all dispatch/resources, or claim
    fails stale; no success can retain authority past completed revocation.
  - `resource_or_ipc_first_does_not_consume_navigation_permit`: while Armed,
    valid-cap asset/IPC refuses with state unchanged; main claim then succeeds;
    repeated post-claim assets/IPC remain authorized without changing permit.
  - `inert_blank_before_or_after_bind_has_no_authority`: exercise blank browse
    before/after on_after_created, and creation returning before/after binding;
    no cap/private bytes/IPC/privileged injection or permit consumption.
    Actual host navigation is issued only with bound id and producer handle.
  - `inert_creation_deadline_replay_and_late_binding_fail_closed`: at 5s revoke
    and close pending owner; late bind cannot arm; foreign/duplicate conflicting
    ids refuse, duplicate same-id is idempotent; delayed blank after Armed
    cancels without consuming; no private resource-first request gains authority.
  - `reload_cannot_reuse_claimed_document`: consumed same-url browse cancels
    and schedules host renewal; only a freshly armed document on the same bound
    browser succeeds, and old queues/resources remain revoked.
  - `business_byte_boundary_is_not_log_cap`: valid bodies of 400,000 bytes pass
    transport; 400,001 fail before allocation. In existing `agent_chat/tests.rs`,
    `web_send_preserves_scalar_and_serialized_limits` checks 100,000 ASCII chars
    accepted, 100,001 refused, and multibyte JSON total >400,000 refused even if
    scalar count fits. Existing attachment/link tests remain unchanged.
  - `hostile_http_head_never_allocates_body`: unterminated 2,049-byte request
    line, 8,193-byte header, 16,385-byte head, 65 headers, duplicate/conflicting
    CL, `+1`, `-1`, decimal overflow, CL=400001, TE+CL, folded/invalid header,
    absolute target, `%2f`, `%00`, `../`, EOF body and invalid UTF-8 fail.
  - `deadlines_are_absolute_across_progress`: `remaining(start+5s, start+5s)`
    is Deadline even after partial reads; response at its 5s deadline fails.
  - `queue_limits_count_bytes_and_drain`: 32 tiny entries allowed, 33rd refused;
    eight 400,000-byte entries fill byte budget; failed push never 200; batch
    takes at most two such entries or eight tiny entries in FIFO order.
  - `server_stop_reaps_owned_workers_and_rejects_enqueue`: stop with idle/active
    readers and failed browser creation; all owned handles complete, port closes,
    pictures leases release; stop polling never joins an unfinished worker.
  - `transport_errors_never_log_capability_or_business_text`: sentinel credential
    in current/old URL, error/log text and private message does not reach sinks.
  - In existing `assistant_cef_config.rs::tests`,
    `private_http_overrides_refuse_url_logging_on_every_backend`: each name
    ASCII-lowercased containing log/starting trace/equal v or vmodule refuses via
    `validate_private_http_overrides`, including enable-logging=stderr,
    log-file, log-net-log and trace-startup-file; a normal feature switch passes.
    Include mixed-case legacy logging names and the existing prohibited
    remote-debugging*/auto-open-devtools-for-tabs names on non-B backends too.
    Retain generic parser/non-B behavior tests, but private-host composition
    must reject the old parser's accepted logging example.
  - In real app tests,
    `private_http_runtime_suppresses_browser_argv_and_native_logs`: all desktop
    private-host configurations use command_line_args_disabled=true and
    CefLogSeverity::Disable before initialize. No generic welding default change.
- [ ] Run pure transport RED using section “Cheap commands” after creating the
  production module/tests; expect explicit assertion/undefined-interface failures,
  not an unrelated dependency error. Real `web::act` tests remain BLOCKED if
  app/UI test compilation is unavailable; do not substitute a fake act function.
- [ ] Implement bounded parser/server, fixed four workers, queue+byte budgets and
  5s absolute IO deadlines. `try_reserve` before attacker-sized growth; retain
  static/Arc resource bytes; 503 and visible failure on enqueue refusal.
- [ ] Wire dedicated entropy, fresh namespace, HTML/shim/all probes, constructor
  failure cleanup, sanitized logs/native logging containment and bounded UI drain.
  Update all direct `/ipc` users in this file, not just `IPC_SHIM`.
- [ ] Implement `assistant_cef_config::validate_private_http_overrides(
  overrides: &[Switch]) -> Result<(), PolicyError>` and invoke on every desktop
  private-host constructor before build_policy/initialize; set the host's native
  log severity/argv suppression exactly as addendum §3.1. Include main module
  declarations/imports, not only the new source files.
- [ ] Implement and wire precommit/resource/redirect policy in the real three
  platform handlers; preserve no-policy default. Add
  `navigation_policy_rejects_consumed_permit_redirect_and_foreign_frame`,
  `blocked_reload_requests_host_renewal_without_old_authority` and
  `osr_widgets_are_not_popup_browser_navigation` to existing policy/platform
  tests. Authority decision assertions belong in the actual HTTP authority's
  inline tests; real callback wiring tests belong in platform tests, not a
  standalone typedef-only policy harness. Check missing browser/frame/request
  fail closed.
- [ ] Run pure transport GREEN, static producer-caller audit and diffcheck.
  Native callback delivery and real CEF all-assets/attachments/link execution:
  BLOCKED, not green from a string/source test.
- [ ] After controller permits implementation checkpoints, save W1 locally with
  explicit paths, message `fix: authorize and bound Assistant page transport`.
  No push. Do not commit an interim state that claims integrated readiness.

## W2 — Preserve CAD authority at the actual device seam

**Consumes:** W1 refusal/lifecycle prerequisites; selected real wgpu adapter and
base descriptor from existing CAD callback.

**Produces:** `NativeDeviceCreator` and
`WgpuSetupCreateNew::native_device_creator` exactly as addendum §4;
`assistant_cef::configure_interop(setup: &mut
eframe::egui_wgpu::WgpuSetupCreateNew, eligible: bool) -> Result<(), String>`.
Errors here describe configuration refusal, never trigger another adapter probe.
Native factory default None and wasm exclusion are explicit.

`eligible` is **coarse only**. Actual source timing is
`main.rs:1168 wgpu_options` → `1254 run_native` → selected adapter/device →
CreationContext closure → `1443–1458 CefAssistant::new(..., host_backend(cc))`.
No early bool can encode a display handle not yet created. Coarse eligibility
checks runtime path presence, no explicit NO_INTEROP and no CAD software/safe/
recovery mode; it neither loads/initializes CEF nor uses WAYLAND_DISPLAY, DISPLAY
or XDG_SESSION_TYPE as backend authority. The factory checks the actually
selected Vulkan adapter/features and the cleanup gate. Later construction checks
the actual display handle, B policy and loaded identity. If that later policy
refuses, preserve the same CAD device/configuration and report Assistant
unavailable; do not rerun adapter selection or downgrade CAD requirements.
That is the **current** construction order. Conditional on approval of addendum
§5's startup policy, actual host policy/runtime initialization moves to entry
of the same CreationContext closure, still after selected-device creation;
browser attachment remains later. The coarse bool never becomes Wayland
authority in either order.

- [ ] **RED in existing `setup.rs`/`lib.rs` tests, `startup.rs` tests and
  `main.rs` tests:** `device_factory_defaults_and_clone_preserve_configuration`,
  `selected_device_descriptor_retains_cad_features_and_limits`,
  `unsupported_interop_uses_original_selected_adapter`,
  `cpu_recovery_and_missing_cef_do_not_install_interop`,
  `early_interop_eligibility_does_not_claim_wayland_policy`,
  `later_display_handle_refusal_keeps_selected_cad_device`,
  `interop_failure_releases_native_owners_before_cad_request`.
  Use real `wgpu::DeviceDescriptor`/Features/Limits. Assert original feature set
  is a subset, every limit and memory/trace/experimental field is unchanged,
  FLOAT_DEPTH_STENCIL and texture-format features retained. Selection/surface/
  backend environment/preference tests reuse `startup.rs` fixtures.
- [ ] Record actual RED only if real dependency compilation is possible under
  authorization. Otherwise mark these tests BLOCKED. A scalar bitmap/model or
  invented Adapter is not a substitute for actual wgpu signature/type tests.
- [ ] Add field/default/Clone/Debug/destructure; invoke after the existing
  compatible-surface adapter selection and once-evaluated descriptor. Keep
  `Existing`, MSAA/surface and `DeviceLost::watch` paths intact.
- [ ] Remove independent `interop_setup`; install Linux callback only with
  coarse eligibility, never CAD CPU recovery or an environment-derived display
  assertion. Callback adds only supported interop
  feature to a descriptor clone, passes same adapter to the existing welding
  helper and returns ordinary CAD path on unsupported/clean failure.
- [ ] Trace the locked grafting→HAL→core cleanup paths and retain file/version/
  source hashes and ownership notes under `W2/cleanup-audit.md`, reusing the
  verified primary copies in SDD `builder-evidence/INDEX.md`, not re-extracting
  them. Addendum §4 records the source-supported early-layout cleanup gap and
  minimum **DRAFT** HAL dependency-patch proposal; it is not approved work.
  Until that gate and remaining wrapper failure proof close, never enter the
  helper or invoke ordinary CAD creation after an unproven failed attempt.
  Leave ordinary CAD untouched and Assistant explicitly BLOCKED/refused.
  Descriptor equivalence then proves only **F4 source CAD-preservation**, not
  working accelerated CEF or complete F4 delivery.
- [ ] Record the INDEX addendum's now-supplied core Device/Queue/late-init and
  wgpu30.0.1 front-end bodies. The access gap is closed; later fault/cleanup,
  partial allocations, disposal/idle waits and ABI proof remain explicit
  **native BLOCKED** gates, not claims inferred from boxed/Arc locals.
  No recursive full-wgpu callgraph extraction or new worker requests.
  Do not silently vendor/patch core or grafting to chase an unproven late path.
  The known narrow HAL DRAFT remains the only dependency-patch scope proposal.
- [ ] **Conditional (a), not authorized now:** verify the exact locked archive
  checksum in addendum §4; copy faithful 30.0.1 crate with original licences,
  record input-vs-delta hashes, implement only early layout-error ownership fix
  and real seam tests in `vendor/wgpu-hal/src/vulkan/adapter.rs`. Register root
  Cargo patch and same-version lock path source; inspect lock diff for zero
  other resolution/version churn. W8 records provenance/licences through the
  existing inventory/asset routes. None destroy once before Err; Some callback
  once/no independent destroy; success no early destroy; late faults retain
  existing destruction order. All native fault/ABI/late cleanup tests remain
  BLOCKED, not executed by a source model; no enablement from partial RAII.
- [ ] GREEN real-type/source checks when allowed; native fault injection/device
  loss/FD count/refcount/device lifetime checks remain BLOCKED. No mutation of
  registry, new native graphics fork or another adapter to get a passing test.
- [ ] Local checkpoint, if authorized: `fix: preserve CAD device selection for CEF interop`.

## W3 — Own close completion, quit cancellation and Retry

**Consumes:** W1 document/server owner; W2 selected device; actual per-platform
creation and callback completion. Runtime lifecycle must be integrated before
any user-visible “Retry succeeded” or “quitting” result.

**Produces (proposed):**

```rust
// assistant_cef_lifecycle.rs, pure policy, not a mock native runtime
pub(super) enum Phase { Unavailable, Initializing, Running, Closing, Closed,
    Shutdown, Quarantined, StartupCancelled }
pub(super) enum CloseProgress { Pending, Complete, Quarantined }
pub(super) struct Lifecycle { /* phase, ownership counts, close deadline */ }
impl Lifecycle {
    pub fn new() -> Self;
    pub fn begin_initialization(&mut self) -> Result<(), LifecycleError>;
    pub fn browser_requested(&mut self) -> Result<(), LifecycleError>;
    pub fn observe_created(&mut self, browser_id: i32) -> Result<(), LifecycleError>;
    pub fn observe_creation_cancelled(&mut self) -> Result<(), LifecycleError>;
    pub fn native_initialization_returned_false(&mut self)
        -> Result<(), LifecycleError>;
    pub fn begin_close(&mut self, now: std::time::Instant);
    pub fn observe_closed(&mut self, browser_id: i32) -> Result<(), LifecycleError>;
    pub fn poll(&mut self, now: std::time::Instant) -> CloseProgress;
    pub fn mark_shutdown(&mut self) -> Result<(), LifecycleError>;
    pub fn permits_retry(&self) -> bool;
}
// real vendor surface trait, default unsupported for unrelated producers
fn close_progress(&mut self) -> Result<BrowserCloseProgress, WeldError>;
// vendor runtime; error returns the owner, never drops it into shutdown
pub fn shutdown(self) -> Result<(), (Self, WeldError)>;
// DRAFT startup policy: these are conditional interfaces, not approved work
pub enum AssistantBootstrap {
    Unavailable { reason: InitFailure, restart_required: bool },
    Initialized(PreparedCefRuntime),
}
pub struct StartupCancelled; // Error + Display; no CEF refs/secret fields
pub fn prepare_runtime(cc: &eframe::CreationContext<'_>)
    -> Result<AssistantBootstrap, StartupCancelled>;
// app native owner / browser attachment methods
pub enum AssistantStart {
    Running(CefAssistant),
    Unavailable { reason: InitFailure, restart_required: bool },
    Quarantined(CefAssistant),
}
pub fn from_initialized(prepared: PreparedCefRuntime,
    render_state: &eframe::egui_wgpu::RenderState) -> AssistantStart;
pub fn begin_close(&mut self, ctx: &egui::Context);
pub fn poll_close(&mut self, ctx: &egui::Context) -> CloseProgress;
pub fn retry(&mut self, ctx: &egui::Context) -> Result<(), RetryError>;
```

Define `BrowserCloseProgress::{Pending, Complete}` in `surface.rs`.
`LifecycleError` is MismatchedBrowser, InvalidTransition or IdentityExhausted.
`RetryError` distinguishes RestartRequired, Closing, Quarantined and Navigation.
`InitFailure` categorizes MissingRuntime, Identity, Policy, Entropy, Server,
Interop and Browser without raw URLs/secrets. `Unavailable` has no live native
ownership; a partially initialized live owner must be returned as Quarantined,
not dropped into the None branch.
Exception: actual native initialize=false is the DRAFT **StartupCancelled**
terminal branch before App/SolveApp/documents exist, not an AssistantStart value
returned after document startup. PreparedCefRuntime is one opaque, non-Clone,
init-thread-affine successful runtime owner; no page server/browser exists yet.
`CefAssistant` stores runtime/producer/host/server/resources in takeable owner
slots; phase/progress is not derived from `browser_id == 0`. Vendor runtime
tracks creation leases/live producers and init-thread ownership; remove Clone.
`close_progress` accounts for late creation and callback return before Complete.

In `documents.rs`, `HostCloseState::{Ready, Required, Closing, Quarantined}`;
`SolveApp::host_close_state`, initialized Ready. This is host status, no native
types in UI. Successful control results add a nested `hostClose` object;
app host is sole sender of final Close while barrier exists.
Add `documents::finish_quit(app: &mut SolveApp)
-> Result<QuitCommit, QuitIntentError>` to commit
the existing Don't Save/open-design preference finalization only when host state
is Ready and **every approved session snapshot is still current**. The exact
QuitCommit/QuitIntentError and snapshot types are defined in addendum R1 below
the shared quit contract. Keep pending discard indices in
`Documents::pending_quit_discard: Option<Vec<usize>>`, preserving dirty/recovery
state until then; cancellation clears only staged decisions, not designs.
Add `documents::request_quit_programmatic(app: &mut SolveApp, force: bool)
-> Result<(), String>` using existing `open_sessions`/`Session::is_dirty`,
without `request_quit` or dialogs. Add
`documents::host_close_status(app: &SolveApp) -> Value` returning exactly
`{"state":"ready|required|closing|quarantined","pending":bool,
"blocked":null|"assistant-close-incomplete"}`.
`pending = matches!(host_close_state, Required | Closing)` exactly, independent
of quit_requested; do not overload document prompt state.

Concrete document-state ownership (existing `quitting` is not obsolete):

| Stage | docs.quitting | pending_quit_discard | closing / quit intent |
|---|---|---|---|
| Idle | None | None | actual prompt index/null; no quit intent |
| UI prompt sequence | Some(accumulating Don't Save indices) | None | closing is actual prompt tab; quit_requested=false |
| All prompts resolved / programmatic accepted | Some(empty), retained as workflow/mutation guard | Some(moved indices), even when empty | closing=None, saving=false; quit_requested=true |
| Commit once, host Ready + R1-validated current intent | clear to None | take once, mark only those saved | clear prompt/saving; terminal Committed latch/one QuitCommit for final Close |
| Cancel / timeout / cancelled or failed Save | clear to None | clear without mark_saved | closing=None, saving=false, quit_requested=false |

Do not duplicate mutable discard indices in both vectors. `quit_step` moves the
existing `quitting` Vec at its old finalization point732–745 instead of
mark_saved/clear. Programmatic force stages actual dirty indices and sets the
same Some(empty) guard; nonforce succeeds only clean. `answer_prompt`1298–1309
and `saved_for_close`1319–1338 clear **both** stages and intent on cancellation/
Save failure, keeping real successful saves. Centralize that path in
`documents::cancel_quit(app: &mut SolveApp)` and call it on host timeout too.
`finish_quit` checks Ready/current staged intent, validates all identities/state
and pending-work absence as R1 below, commits the old reopen-preference/
mark_saved effects once and clears document guards; indices alone are not proof.
Pending-staged quit_step is a no-op, not another prompting round.
All UI/control and async tab-open/close/reorder/replacement completions must
respect the quitting guard; pending file loads count as document-operation-
pending in preflight, and no index mutation may complete while staged. A
rejected/held mutation must not silently lose queued document work.
Query `ui.documents.quitting=docs.quitting.is_some()` remains true through
staged close, false on commit/cancel; closing stays Option<usize>, not host
status. Late host completion can clear quarantine but never recreate intent
or either vector. Host-only close may be Required/Closing with intent=false:
hostClose.pending still true, document quitting still false.

### W3 R1 — state-bound quit intent and final atomic close commit

Consumes `integration-builder-check-3.md` R1 only; no change to prior Q1–Q7 or
either DRAFT human sentence. The index guard does not prevent run/control/
asynchronous completion from changing a Session while CEF closes.

**Use the exact existing-field snapshot in addendum R1**, not a new global
counter/renderer epoch: engine QuitSessionSnapshot contains uid, document_id,
replacements, **Session::revision**, strong doc/saved Arcs, visibility/
saved_visibility, pending_moves/exploded, active_sketch, path/disk/read_only. New
`Session::quit_snapshot(&self) -> solidcraft_engine::Result<QuitSessionSnapshot>` and
`matches_quit_snapshot(&self, &QuitSessionSnapshot) -> bool` compare all fields
(Arc::ptr_eq for retained Arcs); reject saturated counters. Public
design_revision/document_stamp is unsuitable for this token: inspect.rs351–365
allows content ABA reuse. General revision conservatively includes view refresh;
do not “fix” false cancellation by dropping that guard. No undo-length/hash/
dirty-only identity. Private saved fields stay behind engine accessors.

Documents stores `quit_intent: Option<QuitIntent>` with bounded entries
`{tab, snapshot, decision: Pending|KeepSaved|Discard}` and
`stage: Prompting|Staged|Committed`, exactly defined in the Spec. Existing
quitting/discard index vectors retain their prior public semantics; the new
entry snapshots are consent, not a second mutable discard-index list.

- [ ] On UI request capture all sessions using existing open_sessions757–762,
  including inactive tabs, with clean entries KeepSaved and dirty ones Pending.
  Capture/approve the actual state when Don't Save is answered. Only successful
  prompted Save of a Pending target may refresh that target to KeepSaved, after
  adopt_save and still-clean verification; never recapture earlier approved
  tabs while advancing prompts. Programmatic accepted quit captures all entries
  at acceptance; force authorizes only then-current dirty states. Validate
  approved entries at quit_step and all entries before Staged.
  During Prompting validate only approved entries, not the Pending Save target;
  pending-work absence is a finish_quit condition. Routine validation does not
  clear the terminal Committed latch because its saved markers changed on purpose.
  Initial capture/allocation failure aborts before installing intent; later
  prompted capture failure cancels the request before commit, with no default token.
- [ ] In existing engine tests verify refresh1299, undo/redo edit149–213,
  replace_design907–927, accept_computed2209–2234 and adopt_save(file633–642)
  cannot accidentally match the old snapshot after relevant change. A real
  Save keeps its written snapshot and newer edits remain dirty, unchanged.
- [ ] Add the exact pending queries `SolveApp::quit_has_pending_design_work`
  and `deferred::DocJobs::blocks_quit` from the Spec. Inspect existing fields
  without polling/consuming workers: active applying/deferred/deferred_queue/
  start_after_roll/delete_check/pending_feature_edit/editing_state, file load/picker/saving,
  control_job/control_held/agent execution, edit-in-place refreshing, plus
  inactive slots' DocJobs and pending feature/editor work. Spec lists the
  four DocJobs fields; no undefined PendingWork abstraction. In-flight work
  cannot be silently dropped just because its result has not been applied.
- [ ] Run `validate_quit_intent(app: &mut SolveApp)
  -> Result<(), QuitIntentError>` when an intent exists after run/control batch/
  deferred/feature-edit and real completion adoption: control_job::finish_
  control_ahead/poll_control_jobs → run_control, dialogs::adopt_dialog →
  accept_commands, file_load::tick → accept_computed/adopt, save_job::tick →
  adopt_save and linked/edit-in-place application. These may mutate while
  Closing; a changed approved snapshot cancels quit, not the successful command/
  completion. Failed/no-op operations need not cancel unchanged state.
  Explicit replacement/close/discard respects the existing tab guard or cancels
  quit before its ordinary operation; it never borrows stale quit consent.
- [ ] Before **any** quit-induced mark_saved/preference/recovery discard/final
  Close, finish_quit must check all sessions and no Pending/pending owner work,
  then commit in one non-reentrant owner-thread section. Any mismatch/busy work
  calls cancel_quit and returns Changed/PendingWork before **any** effects;
  NotReady/Missing cannot grant a receipt. No partial first-tab discard before
  discovering a later-tab mismatch. Preserve actual saved state, dirty/recovery/
  reopen data, pending work and real Save results. Clear snapshot intent and
  both index vectors/closing/saving/quit_requested; desktop sends CancelClose
  and nonmodal status, no new programmatic dialog or asynchronous reply.
  NotReady retains pending state; Missing and AlreadyCommitted have no effects,
  and AlreadyCommitted never clears the terminal latch/resumes CAD.
- [ ] Success changes stage to Committed, marks only approved Discard states
  saved once and produces the private non-Clone `QuitCommit { _private: () }`.
  Move final Close out of main274–275 to **tail of App::ui main293–325**, after
  content_ui/Assistant tick/native-menu/input work. That receipt permits one
  Close in the same section; return without another mutation. Hosts Ready at
  request use this same check. Intercept every direct OS Close until Committed,
  including Ready+staged intent; a second close request cannot bypass validation.
  Keep Committed as a narrow terminal latch:
  `quit_committed(&SolveApp) -> bool` prevents later logic/UI/control execution/
  adoption before teardown; SolveApp::run and the named completion entries
  enforce it even outside the frame callback. Repeat finish returns AlreadyCommitted.
  Owner-only shutdown cancels jobs, not applies results; on_exit340's save tick
  cannot acquire a new save after the pending-work check/latch. No commit via
  quit_requested alone or from NativeClosed.
- [ ] Late native completion after snapshot mismatch/Cancel/timeout may clear
  the native barrier but cannot recreate any consent/quit intent. Fresh explicit
  quit captures anew. Existing closing:Option<usize>, quitting=document workflow
  and exact hostClose.pending remain unchanged.

Planned real-module tests (NOT_RUN; no fake Session/GPU implementation):
`clean_programmatic_quit_then_edit_cancels_before_close`,
`dont_save_then_edit_preserves_new_dirty_state`,
`delayed_command_completion_invalidates_quit_intent`,
`quit_snapshot_rejects_undo_redo_content_aba`,
`quit_snapshot_detects_save_replace_and_inactive_session_changes`,
`cancelled_quit_then_late_native_closed_never_resumes`,
`unchanged_quit_intent_commits_once_after_final_ui_work`,
`committed_quit_never_dispatches_another_mutation`.
Use existing Session commands and Harness/channel-controlled real completion
seams: both initially clean/nonforce and force/Don't Save; edit→undo→same content/
history length; completion applied during Closing and pending at final check;
successful save preserved; later-tab mismatch gives zero mark_saved/discard/
Close; last UI mutation invalidates; unchanged state yields exactly one commit.
Native close/lifetime/teardown proof remains BLOCKED, not replaced by these tests.

**One concrete Outcome refusal path:** programmatic helper preflights, in order:
Quarantined → Err("quit refused: assistant-close-incomplete");
busy picker/file load or pending document close/quit workflow →
Err("quit refused: document-operation-pending");
dirty open sessions with force=false → Err("quit refused: unsaved-documents").
Each Err is before any mutation/dialog. In `control::dispatch`, both quit
endpoints map that Err with the existing `err(...)` helper at control.rs:202:
`Outcome::Done({"ok":false,"error":reason})`, **no result/nested error**.
Do not add a new error wrapper or business IPC error schema.

For Ok, helper records quit intent (and host pending status); it never sends
viewport Close. `app.quit` uses existing `ok(...)` with result
`{"quitting":quit_requested && host_close_state==Ready,
"asking":false,"hostClose":host_close_status(app)}`.
For `ui.documents action=quit`, control dispatch consumes that action, invokes
the same helper/Err mapping and then calls `documents::control(app, &json!({}))`
as a **query** to obtain the normal result. Move only its old Some("quit")
mutation branch into dispatch; preserve `documents::control -> Value`, all
query/non-quit callers and the document-tab `closing: Option<usize>` field.
It must not retain a hidden dialog-producing quit route.

`documents::control` result adds `hostClose` while retaining its existing
`quitting = docs.quitting.is_some()` and every other existing field. `ui.inspect`
adds the same hostClose object beside existing quit_requested and document
quitting/dialog/status. Do not redefine either document quitting field.
Query endpoints remain successful in quarantine; a new quit request returns
the outer Err above. No second asynchronous reply to the original request is
promised. Required immediately after an accepted request reports app quitting
false/hostClose state required/pending true; the later host turn sets Closing.
After timeout inspect shows state quarantined/pending false/blocked reason.
No pending/Ready response is proof that a process has actually exited.
Update only the quit/host-status control documentation and the callers in W7
that poll quitting/cleanup; Agent/MCP business payloads stay unchanged.

### W3 conditional DRAFT startup-failure policy (same unit/owner, not approval)

Read addendum §5's exact user decision before dispatch. Approval of close-timeout
quarantine does **not** approve cancelling startup. One proposed policy only:
initialize CEF at AppCreator entry before SolveApp/services/document/control/
recovery/autostart work; native false cancels only this launch via normal error
return, with no post-false CEF calls; pre-init refusal continues CAD without
Assistant. No new process architecture, forced kill or backend-relaunch fallback.

Source-backed order: main currently builds SolveApp at1269, control at1345,
sample at1353, document opening at1395 and recovery at1409, **before** CEF1443.
Move runtime preparation ahead of font/chooser jobs1261–1268 as well. The
actual cc render state/display handle already exists at closure entry1257.
macOS `open_documents::install` before run_native only queues paths; leave its
take/open after the gate. AI/agent autostarts are in App::logic's second frame;
no App is returned on native false. Do not describe the current late call as safe.

Conditional engineering steps within W3, only after explicit policy approval:

- [ ] Call `prepare_runtime(cc)` first. Pre-init refusal is
  AssistantBootstrap::Unavailable; actual
  `CefRuntime::initialize -> Err(WeldError::InitFailed { code:0 })` is
  StartupCancelled. Use this typed native-false distinction, not string matching.
  Success transfers a single PreparedCefRuntime to the later
  `CefAssistant::from_initialized` attachment, with no second initialize.
- [ ] Set `NativeOptions.run_and_return=true` explicitly. For
  `Error::AppCreation(error)` with
  `error.downcast_ref::<StartupCancelled>().is_some()`, return that error before
  the existing generic created/no_window branch or token cleanup. Do not call
  no_window (which may spawn a retry/process and show a dialog), App on_exit,
  recovery discard, update launch or device_lost::end. Emit the constant
  startup-cancelled diagnostic; never call process::exit on this browser-process
  failure path. Existing subprocess dispatch is a different entrypoint.
- [ ] At owned `vendor/welding/src/runtime.rs`, prepare a non-dropping native
  init-call frame **before** cef_initialize. Conditional DRAFT (b) explicitly
  uses existing **`cef::sys::cef_initialize`** inside that owned native module,
  not generated high-level `cef::initialize`, whose internal clones/conversions
  cannot be retained by an outer guard. Existing cef dependency/reexport only:
  no new dependency, generated-binding edit or CEF binary fork. Prepare raw
  Settings/string storage, App raw pointer and explicit reference transfer,
  raw argv backing, sandbox/loader owners before calling. Do not blindly call
  From<Settings> and assume pointer lifetime/equivalence. Native false must not construct
  CefRuntime or drop/release its Args/App/Settings/callback/loader owners into
  CEF APIs. Retain false-path inputs without native destructors, including at
  thread-local teardown; success transfers ownership to the live-runtime path.
  Current locals on runtime.rs324–343 do not meet that guarantee merely by
  returning InitFailed. Keep runtime process ownership terminal on false.
- [ ] Keep the source/conformance gate explicit: cef's wrapper58325–58358 clones/
  converts Settings and adds an App reference; CefStringUtf16 Drop477–486 and
  RefGuard Drop304–307 can call CEF. Prove the whole return/teardown chain makes
  no CEF call after native false, not only the explicit shutdown callsite.
  Completed INDEX source copies close the excerpt-access question, **not**
  raw ABI/lifetime or immediate sys callee/loader cleanup proof. Source normal
  error return is known; zero post-false CEF entrypoints/ref-release/unload
  callbacks remains **BLOCKED** until verified. If the immediate owned seam
  cannot meet it, report that exact blocker; no more callee-graph extraction
  requests, binding fork, force-exit exemption or new process design.
- [ ] Implement the exact **NON-CEF launch-owned bookkeeping interfaces** in
  addendum §5 within this same conditional W3, not a new subsystem task:
  startup::{StartupTicket,begin_owned}, ticket prior_failed_starts/
  cancel_controlled/mark_window_started/retire_no_window_attempt;
  crash::{StartupSessionTicket,
  startup_session_ticket,cancel_startup_session}. Private non-Clone tickets
  bind unique locked launch entries/current process-local journal generation.
  Replace main's begin/path-only App.starting/second-frame started calls;
  renderer no_window/retry must transfer/retire only its own ticket and is
  bypassed on StartupCancelled. No blind shared starting-file deletion.
- [ ] Use short, nonblocking coordinator File-lock sections and bounded,
  versioned local startup ledger plus unique per-run lifetime-locked entries.
  Import legacy numeric history once, clamped as before; do not delete legacy
  bytes. Count baseline plus proven dead unresolved entries, never active/
  cancelled/started entries or lock-access errors. Preserve historical and
  concurrent generations. cancel_controlled retires only its own entry;
  actual second-frame success and the existing generic no_window retirement
  may retire their observed failed generations as before; controlled CEF
  cancellation never does, and never calls retire_no_window_attempt.
  Durable terminal state survives unlink failure; errors are typed, not zero
  history, forced safe mode or a fake mark_window_started. Mixed legacy-binary/
  migration and IO fault behavior stay explicit test gates.
- [ ] In crash.rs factor a checked owned end-session helper while preserving
  ordinary end_session callers/report repeat-count persistence. Startup token
  validation requires empty pre-document journal, no RUNNING/commands/actual
  panic/native evidence. Sync controlled_startup_cancel before unlock/unlink;
  reader/dead-journal classification skips only valid otherwise empty cancelled
  runs. Genuine native/panic/command evidence overrides cancellation. Do not
  clear prior reports/.seen/dirty/recovery/reopen data or rescan dead journals
  during cancellation. Wrong token cannot remove any journal.
- [ ] Capture startup ticket at existing begin position and journal ticket just
  after install. On actual false retain native inputs, propagate typed error,
  let eframe return, then finalize own journal → own startup ticket → diagnostic/
  normal error return. All abort bookkeeping is Rust/filesystem-only, no CEF,
  App hooks, token/socket cleanup or device_lost::end. Acknowledge both durable
  transitions; surface partial write/lock/permission errors and keep evidence.
  If this proof cannot prevent fake crashes/escalation for controlled abort,
  (b) stays disabled/BLOCKED. No other process's state may be cleared.

Planned tests/gates in existing main/runtime/lifecycle test areas:
`startup_gate_precedes_solve_app_documents_control_and_autostarts` pins actual
production ordering (source check only, no native proof);
`pre_init_refusal_continues_cad_without_calling_initialize` and
`native_false_is_terminal_startup_cancelled_not_quarantine` pin the pure
classification/transition with actual policy code, not a mock CEF runtime;
`startup_cancelled_error_bypasses_no_window_and_relaunch` pins typed outer error
classification; `successful_bootstrap_is_consumed_without_second_initialize`
pins the one-owner contract. Real-type/native tests
`false_return_never_releases_cef_refs_or_calls_shutdown`,
`false_return_opens_no_user_or_recovery_document`,
`cancelled_startup_does_not_touch_other_instance` and platform main-thread/
normal-return traces remain **NOT_RUN/BLOCKED**, requiring real permitted
native failure evidence. No fake native implementation or source-order test
is reported as executing cef_initialize=false.

Bookkeeping RED/GREEN tests belong in the existing startup/main/crash modules:
`controlled_startup_cancels_do_not_increment_failures_or_cpu_safe_mode`,
`controlled_cancel_preserves_prior_failures_reports_and_recovery_bytes`,
`controlled_cancel_never_resets_history_or_uses_no_window_retirement`,
`parallel_launch_cancel_never_clears_another_marker`,
`wrong_or_replayed_journal_ticket_cannot_end_another_session`,
`cancel_terminal_survives_unlink_failure_without_unclean_report`,
`native_crash_evidence_overrides_controlled_cancel`,
`bookkeeping_lock_write_and_migration_errors_are_not_silent_success`.
Use real tempfile/lock/parser code in existing modules, both completion orders,
at least two controlled cancellations plus subsequent real premature death,
legacy/damaged markers, active lock vs permission error and durable-write faults.
No production fake CEF or arbitrary path supplied to cancel. Actual main-thread/
native false/OS return traces remain NOT_RUN/BLOCKED; filesystem policy tests do
not prove CEF safety. Any recovery-policy change beyond excluding this controlled
pre-document abort must return as a new material decision.

- [ ] **RED** pure lifecycle tests:
  `close_requires_matching_completion_not_request`,
  `close_timeout_cancels_quit_and_never_allows_retry`,
  `late_close_finishes_cleanup_without_resuming_quit`,
  `partial_initialization_cannot_create_overlapping_owner`,
  `late_creation_after_close_is_still_owned`.
  Drive exact instants: 4,999ms Pending; 5,000ms Quarantined; repeated polls do
  not extend deadline. Wrong browser id must not complete.
- [ ] **RED** existing real runtime/platform/app/control tests:
  `runtime_is_unique_and_shutdown_requires_no_browser_leases`,
  `browser_references_release_before_single_shutdown`,
  `initial_none_retry_is_consumed_as_restart_required`,
  `terminated_renderer_retry_navigates_fresh_document_in_same_runtime`,
  `programmatic_quit_has_no_dialog_and_obeys_host_barrier`,
  `force_quit_does_not_force_native_shutdown`,
  `ui_documents_quit_cannot_bypass_host_barrier`,
  `quit_refusals_are_outer_outcome_errors_without_mutation`,
  `documents_closing_remains_tab_index_with_additive_host_close`,
  `accepted_quit_and_timeout_report_endpoint_specific_host_close`,
  `cancelled_native_quit_does_not_mark_discarded_documents_saved`,
  `host_close_pending_depends_only_on_required_or_closing_phase`,
  `staged_discard_moves_indices_without_stale_document_query_state`,
  `cancel_or_failed_save_clears_both_quit_stages_preserving_dirty`,
  `tab_mutations_cannot_invalidate_staged_discard_indices`,
  `quarantined_close_does_not_start_update_or_device_loss_replacement`.
  Pure transition tests do not execute CEF; real callback/thread order tests
  remain BLOCKED until trusted native build, with no simulated ABI claim.
  Outcome tests pattern-match `Outcome::Done` from real dispatch: both dirty
  quit methods equal `{"ok":false,"error":"quit refused: unsaved-documents"}`,
  with unchanged dialog/docs/quit state; force still refuses Quarantined.
  Existing documents close_others/close_all tests keep closing=Some(tab)/null,
  never bool. Pending/timeout status tests distinguish app.quit.quitting,
  ui.documents.quitting, ui.inspect.quitting and nested hostClose.
- [ ] Implement init-thread-affine non-cloneable runtime and pending/live creation
  leases. Audit `CefRuntime` users using source search; no duplicate Drop shutdown.
  Explicit shutdown validates empty leases and completes once; Drop cannot call
  native shutdown with unresolved ownership. Keep subprocess entry before threads.
  Apply the separate DRAFT startup policy only if approved, and only after
  the no-post-false-call conformance gate closes. Without that authorization/
  proof, initialize=false remains an explicit blocker, not Running quarantine,
  a forced exit or certified continuation of CAD after native failure.
- [ ] Add matching before-close completion and browser-reference release to Linux/
  macOS, reuse/fix Windows shared completion (never reset completed state on
  repeated close). Include pending/deferred creation cancellation, script/cookie/
  auth/context and callback-held reference release.
- [ ] Implement nonblocking close progression on app logic. Drain/release both
  frame slots, texture registrations, producer/native contexts and server in
  correct order; shutdown only after completion, outside callback, on init thread.
  Failed constructors unwind through this owner, not detached resources.
- [ ] Wire shared host barrier to every final Close producer: OS, menu/documents,
  control `app.quit`, `ui.documents quit`, device-loss restart/update close.
  Preserve document save workflow for UI; programmatic dirty nonforce refuses
  without dialogs. At 5s clear quit_requested, CancelClose, localized nonmodal
  status and retained quarantine; no forced exit or synchronous wait.
  Stage existing Don't Save finalization until `finish_quit` immediately before
  final Close; do not call recovery close/update launch/device_lost::end while
  the owner is unresolved. A lost CAD GPU remains a rendering-loss condition,
  not a promise that quarantine can restore its rendering.
- [ ] Handle Retry in `main`'s None branch and live owner. Renderer retry rotates
  W1 authority and actually navigates existing browser; close/partial init forbids
  reconstruction. Page-ready/theme and frame-admission remain separate.
- [ ] GREEN pure lifecycle plus available real-type checks; update control docs/
  result tests and pending locale keys. Keep native thread/ref/late-callback/
  normal-quit/programmatic-quit/device-loss acceptance BLOCKED until permitted.
- [ ] Local checkpoint: `fix: await CEF close completion before host quit`.

## W4 — Correct captured input and Unicode/IME geometry

**Consumes:** W1 current DocumentId and W3 live-phase rules; existing InputTicket,
Placement, MouseEvent/EventModifiers, ImeComposition and egui IME output types.

**Produces:** same public welding event types, fixed translation; app helper
`fn utf16_selection(text: &str, range: std::ops::Range<usize>)
-> Option<(u32, u32)>` in existing `assistant_cef.rs`; native
`cef_input::send_key` emits all UTF-16 units. No scalar→UTF-16 cast.

- [ ] **RED in existing app/vendor test modules:**
  - `captured_drag_keeps_buttons_modifiers_and_outside_release`: press in current
    page, motion beyond page with ctrl+shift+alt/meta, release; same ticket and
    held-button flags reach real event queue. Uncaptured outside motion does not.
  - `motion_never_crosses_press_release_or_ime`: move/down/move/up/move sequence
    retains ordering; coalescing adjacent motion retains full MouseEvent.
  - `outside_page_popup_is_clickable_only_when_current`: drawn popup outside
    page accepts press/wheel; stale popup/old placement cancels instead.
  - `ime_scalar_range_converts_to_utf16`: `"a😀中"` range `1..2` → `(1,3)`,
    `0..3` → `(0,4)`; `3..2` and `0..4` refuse.
  - `non_bmp_char_emits_surrogate_pair`: `😀` → `[0xD83D,0xDE00]`, BMP `中`
    → `[0x4E2D]`. Test conversion used by real send_key, not a separate encoder.
  - `ime_bounds_use_current_window_points`: physical `[20,40,10,20]`, ppp=2,
    placement point origin `[100,200]` → `[110,220,5,10]`; old ticket, NaN,
    overflow, hidden/blurred state clears output.
  - In `ime.rs` extend `bounds_union` tests with i32 extrema/u32::MAX, excessive
    count >200,000, empty bounds and poisoned mailbox; no unchecked sum/panic.
  - `ime_and_paste_queue_bound_owned_bytes`: 400,001-byte string or total
    >800,000 bytes refuses/cancels, without truncating; 256 event count retained.
- [ ] Run permitted pure conversion/bounds tests on real production helpers;
  app/native-type tests remain BLOCKED rather than replaced with shadow events.
- [ ] Retain entire motion event, ordered capture and all modifiers. Hit-test the
  same current popup rectangle that drawing uses; cancel capture at invalidation.
- [ ] Implement UTF-16 conversion/character emission; bound CEF character bounds
  before allocation and checked union; fix `ImeComposition` units documentation.
- [ ] Poll composition, validate current ticket, set actual egui `IMEOutput`
  fields purpose=Normal, window-point rect/cursor_rect and
  should_interrupt_composition=false plus IMEAllowed; drain stale geometry and stop
  logging typed/preedit/commit content. Verify real egui 0.36 signatures before
  changing this call site; do not invent `ViewportCommand::IMERect`.
- [ ] GREEN available source tests. Native drag selection, popup placement,
  modifiers and **real OS** preedit/commit/cancel/candidate position remain BLOCKED.
- [ ] Local checkpoint: `fix: retain CEF gesture and IME coordinate ownership`.

## W5 — Make texture registration ownership explicit and repair cfg

**Consumes:** W3 release ordering and W4 placement validity; actual
`wgpu::TextureFormat`, ImportedTexture and renderer register/free APIs.

**Produces:** in existing `assistant_cef.rs`, replace PageTarget/id duplication
with `RegisteredPage` owning exactly one TextureId, optional copy texture and
`PageKey { size: [u32; 2], source: wgpu::TextureFormat,
sampled: wgpu::TextureFormat }`; one release path for page and popup.

- [ ] **RED real app tests:** `page_gamma_cache_keys_extent_and_formats`,
  `popup_gamma_cache_keys_extent_and_formats`,
  `srgb_plain_srgb_replaces_registration_exactly_once`,
  `closed_assistant_releases_each_registration_once`.
  At fixed size drive BGRA-sRGB→RGBA-sRGB→RGBA-plain→RGBA-sRGB→popup-hidden.
  Retained target never references freed id; copy formats match import and view.
  Use real types; GPU registration execution remains gated.
- [ ] **RED cfg coverage:** existing `agent_chat/tests.rs`
  `theme_state_is_available_without_desktop_web`; source audit checks module
  predicate and all callers. Actual wasm compile is BLOCKED, not emulated by
  defining fake `target_arch` for a host compile.
- [ ] Implement single registration owner/format cache and common release for
  page/popup, retaining byte-preserving GPU gamma copy and 1:1 draw clip.
- [ ] Restore pure theme_bridge availability + native-only web gate; guard main
  subprocess/configure_interop with the same desktop predicate as the module.
- [ ] Add existing module tests for minor M4 later-frame recovery and M5 stale
  composition cancellation. Retain M1/M2 optimization deferral and M6 behavior;
  test/status evidence must not imply new non-B timeout semantics.
- [ ] GREEN available pure tests/static cfg audit. Queue future real
  `cargo xtask wasm`, app/UI tests and Windows/macOS checks, all BLOCKED now.
- [ ] Local checkpoint: `fix: own CEF texture registrations across format changes`.

## W6 — Make density and ABA receipts independently meaningful

**Consumes:** W3 document lifecycle, W4 placement, existing real ViewMetrics and
CaptureMetadata; no invented effective native DSF field.

**Produces:** `surface.rs` owned `RequestedViewMetrics {
physical: [u32; 2], dip: [i32; 2], scale: f32 }` and trait
`fn requested_view_metrics(&self) -> Result<RequestedViewMetrics, WeldError>`.
Each producer reads its actual metrics lock; app receipts distinguish requested,
callback and independently observed fields. Native effective DSF stays `None`
unless an identified independent observation exists.

- [ ] **RED existing `view.rs`/frame/admission tests:**
  `fractional_requested_metrics_are_not_capture_derived`: 463×1122 at 1.25 gives
  370×898 and expected 463×1123; round then ceil, no ±1 density loophole.
  `same_lifetime_origin_and_size_aba_keeps_input_cancelled`: record A→B→A
  revisions even with no admitted B; delayed old-A geometry is not freshness proof.
- [ ] **RED `native-density.py` selftests:**
  `requested_dip_cannot_be_reconstructed_from_capture`,
  `missing_effective_native_scale_blocks_exact_density`,
  `same_lifetime_placement_aba_is_detected`,
  `later_aba_after_unrelated_prefix_is_detected`,
  `pending_b_transition_is_not_hidden_by_admission_dedup`,
  `new_capture_counter_or_document_nonce_is_not_causal_proof`.
  Fixtures with requested 370×898/1.25 and coded 370×898 must FAIL density when
  independently observed scale is present, not PASS via a reconstructed ratio.
  Unknown effective scale BLOCKED; known geometry mismatch FAIL wins.
- [ ] Run Python RED and available real source-module RED.
- [ ] Implement actual requested-metrics snapshot and placement/clip transition
  receipts before acquire/admission, including pending/refused transitions.
  Dedupe by placement identity, not coded-size+scale. Keep receipt caps.
- [ ] Update parser/schema/fixtures/validator together; distinguish request DSF
  from observation. Scan actual placement triples, preserve causal limitation.
  No ABI extension/debugger experiment/CEF fork to manufacture proof.
- [ ] GREEN source/Python tests. Four-scale native effective DSF, visible/input
  ABA and real GPU observation remain BLOCKED.
- [ ] Local checkpoint: `fix: separate CEF requested geometry from density evidence`.

## W7 — Repair proof tools, hook schema and task-owned automation

**Consumes:** W1 new shim/no secrets, W3 asynchronous close outcomes, W6 receipts;
actual fixed hook schema and task VM/app-control interfaces.

**Produces (Python interfaces in existing scripts):**
`native-density.validate_manifest(manifest, base_dir) -> report`,
`capture_bundle(app, cef_root, pid) -> report` retain call signatures, strengthen
identity; `app-input.result_report(checks) -> dict` and
`result_exit(report) -> int` use 0 PASS, 1 FAIL, 2 BLOCKED/NOT_RUN.
`Control.close()` plus context-manager support owns socket and file.
Scripts accept explicit task token-file/PID/starttime/source/platform arguments;
no shared defaults. VM OS input unavailable → structured BLOCKED.

- [ ] **RED selftests in existing native-density.py:**
  `empty_inventory_and_missing_roles_never_pass`,
  `duplicate_role_or_missing_mandatory_resource_fails`,
  `supplied_app_must_match_pid_executable`,
  `pid_reuse_deleted_executable_or_child_helper_mismatch_fails`,
  `resource_escape_or_origin_mismatch_fails`,
  `missing_authenticated_origin_is_blocked`.
  Fixtures use temporary regular files, synthetic proc-record data passed into
  parsing/validation helpers, and known hashes; they are parser tests, not live
  origin/process attestation. Keep existing text-vs-whole-file mismatch tests.
- [ ] **RED selftests in app-input.py/theme-matrix.py:**
  `hook_input_consumer_rectangles_match_real_producer`,
  `missing_clipboard_or_popup_cannot_exit_zero`,
  `wrong_present_value_outranks_missing_observation`,
  `control_socket_closes_on_exception`,
  `offscreen_exception_writes_report_and_finally_cleans_owned_handles`,
  `cleanup_cannot_signal_unowned_or_reused_pid`,
  `current_theme_set_not_literal_25_controls_matrix`.
  Parse actual `SCOPE_PROBE_JS`, assert consumer-requested rect names exist in
  emitted action; add separate small action to budgets/fixture/consumer, not
  merely raising cap. Retain all existing role/alpha/swatch/window-coordinate tests.
- [ ] Run all three offline selftest suites RED without launching any app.
- [ ] Complete role inventory/authenticated-origin/PID-exe/child-starttime
  verification and report statuses. A user-supplied self-consistent manifest is
  not authenticated origin. Inventory whole bundle and packaging transforms.
- [ ] Split extra hook rects into bounded action; consumer waits on matching
  document/theme/placement/run context. Validate actual clipboard expectations,
  popup change selection and scope results in final aggregation, not logs alone.
- [ ] Replace shared token/name-selected window/global kill with owned launch
  identity and explicit task token directory. Use approved VM source/platform/
  task target and app-control capture. `app-ime.sh` must not set X11 fallback,
  run host niri or issue host input. It records BLOCKED if required VM OS IME
  operation is unavailable; injected composition is a separately labelled test.
- [ ] Own clipboard helpers/sockets/logs with finally; use monotonic bounded waits.
  Preserve report on all exceptions and record timed-out product close as
  quarantine/failure, not clean exit. Only task-owned disposable VM cleanup may
  terminate owned processes, with explicit cleanup status.
- [ ] GREEN Python selftests, AST and shell syntax only. Do **not** run live mode
  during current gate. Native artifact identity, actual OS input and VM capture
  remain BLOCKED.
- [ ] Local checkpoint: `fix: bind CEF acceptance tools to owned artifacts and targets`.

## W8 — Close minor records and provenance without rewriting history

**Consumes:** all changed vendor paths and check receipts, final verdict minor
dispositions, existing valid pending records and source theme enumeration.

**Produces:** accurate vendor/source inventory and documentation; no legal
clearance, historical recertification or new product behavior.

- [ ] Correct T5M3's known wrong fold input: remove **ONLY** the `vendor/welding/`
  CODE row from `attribution/pending/cef-wayland-welding.md`. Verify the existing
  four-column header/separator and `vendor/welding/LICENSE.txt` asset row are
  byte-unchanged and unfolded. Code origin/licence/delta remain in existing
  vendor README/UPSTREAM/NOTICE inventory; this is not acceptance of code in
  generic Files. Canonical ATTRIBUTION **Vendored-table curation is DEFERRED
  release housekeeping**; do not claim the main-table observation is closed.
  No new pending header/parser API, fold routing, pending README edit, direct
  ATTRIBUTION edit/fold or workflow/parser tests.
- [ ] If (a) approved, record faithful HAL30.0.1 input archive checksum, local
  delta/files/tests, original licences and removal criteria in **existing**
  vendor README inventory and NOTICE. Code never enters Files pending; only
  actual licence-text asset paths get existing four-column pending rows.
- [ ] Update welding/egui-wgpu notices/provenance for all modified/new files,
  pinned source hashes, removal criteria and MPL matching-source obligation.
  Preserve the actual upstream licence texts and local change marks.
- [ ] Update docs inline around historical PASS/Cargo headings, not only a remote
  blanket disclaimer. Record current theme source order/count derivation and
  JSON→Tokens→bridge→computed DOM→composited pixels chain. Remove hardcoded25
  validator condition, not the requirement to cover **every** source theme.
- [ ] Retain M1/M2 optimization deferral, M4/M5 native acceptance, M6 reliability
  limitation; retain harness warning/failed rerun receipts (T5M4/M5). No unrelated
  warning suppression, preference/default change or cleanup.
- [ ] Explicitly list unclosed package/helper/sandbox, credits generator/version/
  hash, exact MPL public source location and bundled LGPL human authorization.
  Use normal generator only when authorized; no generated-credit hand edits.
- [ ] Check code row absent, licence asset/header preserved and code inventory
  pointers complete; no parser behavior was added or tested.
  Legal/package/runtime-artifact gates stay BLOCKED. No public action.
- [ ] Local checkpoint: `docs: qualify CEF repair evidence and vendor inventory`.

## W9 — Freeze the consolidated fix and hand off one scoped re-review

**Consumes:** W1–W8 source and evidence. This is integration evidence preparation,
not a second broad review or implementer approval.

- [ ] Reconcile every row of coverage matrix below with actual test names, source
  hashes, exit statuses and remaining native blockers. Do not count duplicated
  geometry tests twice or old unchanged source tests as new repair evidence.
- [ ] Run allowed cheap checks on final inputs. Retain RED and GREEN separately,
  actual compiler/version/linker, command/exit output and input SHA-256. No
  metadata retry spree, shadow types, old binary reuse or hidden failed attempts.
- [ ] Capture `git status`, HEAD, diff hashes and exact frozen diff against
  c355edb97. Check only intended file ownership changed. Checkpoints are local/
  unpushed unless controller separately authorizes otherwise.
- [ ] Controller submits the **complete repair diff and coverage** for one scoped
  independent Sol re-review, referencing existing broad verdict and architecture
  audit. Do not dispatch thirteen reviewers or restart whole-branch review.
  Self-review, source tests and builder-check are not this independent verdict.
- [ ] If implementation discovers a material unapproved choice, mark its extension
  DRAFT and stop; do not spend the single fix wave on a replacement architecture.
- [ ] Delivery state stays source-only/BLOCKED where native/resource/licence
  gates remain open. Report exact blockers and retained worktree/resources.

## Coverage matrix — mandatory at final handoff

Initial status for **every new test** below: **NOT_RUN (planning)**.
Source-traced defect status: OPEN until implementation and independent re-review.
Evidence root E is
`.superpowers/sdd/2026-11-18-cef-wayland-dsf/final-review/repair/`.
Paths name future receipts, not files created or tests executed by this author.

| Finding / minor | Work unit | Required test / observation | Evidence / unresolved proof |
|---|---|---|---|
| F1 capability/navigation | W1/W3 | private bootstrap/all-assets, Host/Origin, consumed permit, old approval revocation | E/W1 + E/W3; native navigation/redirect delivery BLOCKED |
| F2 hostile HTTP | W1 | framing/allocation/deadline/queue/UI batch/stop and visible503 tests | E/W1; live CEF maximum business/attachment transport BLOCKED |
| F3 cfg | W5 | theme state/source gate audit; real wasm and unsupported-target compile | E/W5; Cargo/wasm BLOCKED |
| F4 CAD device | W2 | real descriptor equivalence, selected surface/adapter, coarse-vs-later policy; failed cleanup requires DRAFT dependency gate | E/W2/cleanup-audit.md; at most source CAD-preservation; working accelerated Assistant/F4 delivery BLOCKED pending scoped approval/proof |
| F5 runtime shutdown | W3 | matching close, partial init, reference release, thread/single shutdown; DRAFT direct-sys/early gate, typed return, owned startup/journal cancellation and history/concurrency/fault tests; R1 per-session state-valid quit/atomic last-action Close | E/W3; policy approval, zero-post-false CEF calls, durable bookkeeping/migration/multi-instance, normal platform return and native lifetime BLOCKED |
| F6 capture/popup | W4 | drag order/full flags/outside release/current popup | E/W4; GPU/native input BLOCKED |
| F7 IME/Unicode | W4 | scalar→UTF16, surrogate pair, checked union/current candidate rect | E/W4; OS IME candidates BLOCKED |
| F8 Retry | W3 | actual fresh navigation same runtime vs initialNone/restart-required; no delayed initial CEF call after documents opened | E/W3; native renderer termination/recovery and DRAFT startup split proof BLOCKED |
| F9 gamma ownership | W5 | page+popup BGRA/RGBA/sRGB/plain/release-once | E/W5; actual GPU registration/gamma BLOCKED |
| F10 artifact identity | W7 | nonempty roles, PIDexe/starttime/child/resources/authenticated origin | E/W7; fresh trusted artifact unavailable/BLOCKED |
| F11 density/ABA | W6 | producer rounded DIP, independent scale absent→BLOCKED, actual same-life ABA | E/W6; effective native scale/visible causal freshness BLOCKED |
| F12 schema/aggregation | W7 | real hook/consumer contract, clipboard/popup failure exit, report exception | E/W7; actual app hook/input BLOCKED |
| F13 target ownership | W7 | owned token/PID/starttime/socket/helper cleanup; no global/host operations | E/W7; approved task VM OS input/capture BLOCKED |
| Task3 M1 | W8 | document deferred duplicate state serialization, no speed-up claim | E/W8/disposition.md; DEFERRED nonblocking |
| Task3 M2 | W5/W8 | preserve conservative theme ticket reset; existing admission tests | E/W5 + E/W8; optimization DEFERRED |
| Task3 M3 | W5 | F9 tests for popup and page | E/W5; elevated, not separately deferred |
| Task3 M4 | W5 | later admitted non-B frame clears transient failure | E/W5; native visible recovery BLOCKED |
| Task3 M5 | W4/W5 | move→cancel→refocus→new composition, no old replay | E/W4; native acceptance BLOCKED |
| Task3 M6 | W5/W8 | late/missing non-B paint recorded accurately, existing behavior retained | E/W8; reliability unproven/nonblocking, no new timeout |
| Task4 offscreen report | W7 | exception produces status/report and finally | E/W7; native screenshot not run |
| T5M1 | W7/W8 | dynamic theme enumeration and documented provenance/order | E/W7 + E/W8 |
| T5M2 | W8 | inline historical qualification source audit | E/W8 |
| T5M3 | W8 | remove only welding CODE row; preserve licence asset/header bytes; existing code inventories, including conditional HAL | E/W8/disposition.md; wrong fold input removed, canonical Vendored-table curation DEFERRED release housekeeping; no main-table closure/parser/workflow claim |
| T5M4 | W8/W9 | retain warning with source-harness scope | E/W8; already disclosed, not app compile failure |
| T5M5 | W8/W9 | retain failed early paths and successful receipts/actual exits | E/W9; no historical erasure |

## Cheap commands and evidence protocol (future execution, not run here)

Use the installed toolchain directly; do not enter the older flake dev shell or
install/resolve anything:

```sh
R=/nix/store/53z8sy6irq3kj6nn9n7f3vxnjzff6wwg-rust-default-1.99.1/bin
CC=/nix/store/j1938gsbvbhkyz4kbmvjk56r479mkw3s-gcc-wrapper-16.2.0/bin/cc
W=/home/hyjing/Code/intgral/intcad/worktrees/cef-osr
# Execute in W, after authorization; E is a new task-owned local receipt directory.
# Record command, immediate status, rustc --version and input hashes per RED/GREEN.
# These wrappers are future local evidence, NOT product implementations.
set -eu
: "${E:?set a new task-owned receipt directory}"
mkdir -p "$E"
cat > "$E/direct-source.rs" <<EOF
#[path = "$W/apps/solidcraft/src/assistant_cef_http.rs"]
mod assistant_cef_http;
#[path = "$W/apps/solidcraft/src/assistant_cef_lifecycle.rs"]
mod assistant_cef_lifecycle;
mod assistant_cef {
    #[path = "$W/apps/solidcraft/src/assistant_cef_config.rs"]
    mod config;
}
EOF
"$R/rustc" --version
"$R/rustc" --edition=2024 --test "$E/direct-source.rs" \
  -C linker="$CC" -o "$E/direct-source-tests"
"$E/direct-source-tests" --nocapture --test-threads=1
python3 -B experiments/cef-osr/scripts/native-density.py --self-test
python3 -B experiments/cef-osr/scripts/theme-matrix.py --self-test
python3 -B experiments/cef-osr/scripts/app-input.py --self-test
bash -n experiments/cef-osr/scripts/app-ime.sh
git diff --check
```

The direct-source wrapper imports the **actual** production modules at their
production hierarchy, so pub(super) has a parent and sibling visibility works;
do not compile a pub(super) module file as the rustc crate root. Run the recorded
compiler/test commands separately with their immediate statuses preserved; no
compile failure is GREEN. All names above are proposed until implementation
authorization. The HTTP/lifecycle/config seams remain std-only; wrapper code
contains no replacement logic, types, native/GPU stubs or modified visibility.
The vendor request-policy types alone do not implement Assistant authorization;
their callback integration is verified with the actual native crate later.
For existing pure config/geometry/admission/viewport/identity suites, reuse the real source
imports documented in Task5 report and the installed1.99.1 driver, with **new**
output paths/hashes after changed inputs. A harness may alias actual
`frame_geometry.rs`; it may not define replacement GPU/native/egui types.
AST-parse Python with `ast.parse(path.read_text(), filename=str(path))`, not
py_compile that writes caches. Formatting uses installed rustfmt `--check` on
changed modules without reformatting unrelated main baseline.

Future gated real-type commands, **not authorized/runnable acceptance now**:

```sh
cargo test --profile ci -p solidcraft --bin intcad -- assistant_cef
cargo test --profile ci -p solidcraft --bin intcad -- startup
cargo test --profile ci -p solidcraft-engine --lib -- crash::
cargo test --profile ci -p solidcraft-ui-egui --lib -- agent_chat::tests
cargo test --profile ci -p solidcraft-ui-egui --lib -- control::tests
cargo test --profile ci -p solidcraft-ui-egui --lib -- documents::tests
cargo test --profile ci -p egui-wgpu --lib
cargo test --manifest-path vendor/welding/Cargo.toml --lib --features cef-runtime
cargo clippy -p solidcraft -p solidcraft-ui-egui --all-targets -- -D warnings
cargo xtask wasm
cargo xtask lists
cargo xtask assets
cargo xtask licences --check
cargo xtask runtime-artifact --mode policy
cargo xtask layers
cargo xtask ui-containers
cargo xtask skills
```

`runtime-artifact --mode policy` is the exact registered existing check; it
checks repository distribution policy (currently Mastra), **not** CEF bundle
provenance or legal clearance. CEF identity remains W7's explicit gate.
Windows/macOS shared-Rust checks, native callback and
GPU tests need their real targets/runners. A cached metadata-only typecheck may
be requested separately from the controller only with real dependency identity
and strict attempt limits; this plan does not inherit unlimited previous
metadata exceptions.

## Trusted artifact/native acceptance preparation (BLOCKED)

When resource policy and controller authorization genuinely permit:

1. Recheck nonduplicated aggregate worktree/target/E2E usage, filesystem free and
   one-heavy-build rule. No further cleanup to manufacture permission.
2. Freeze reviewed source/revision/dirty diff/lock/toolchain/vendor hashes.
   Build new app in owned target with authenticated stock CEF origin; retain
   native logs, exit status and complete bundle manifest. Aborted prior build
   produced no artifact; old `target/release/intcad` is ineligible.
3. Record wrapper/app/mapped runtime/subprocess/resources plus pre/post packaging
   transforms, notices/credits/source availability and exact payload. Fail
   mismatches; missing origin/resources stays BLOCKED.
4. Use `/home/hyjing/Code/intgral/intcad/.dev/e2e/bin/intcad-e2e` with stable
   `--source <W> --task cefrepair` (options before subcommand), explicit platform,
   VM name/evidence. Read `report.json` and `REPORT.md`. No host niri fallback.
5. Exercise valid maximum IPC/send/attachments/link; unauthorized asset/bootstrap/
   redirects/stale documents; head/body/slow-reader/backpressure; creation failure/
   renderer Retry/close timeout/late callback/programmatic quit/device-loss;
   current popup/drag/modifier/UTF16 and real OS IME; format transitions.
6. At 1/1.25/1.5/2, every source theme: actual requested DIP/placement/native
   effective DSF/capture/import/clip, resize/scale-only/bursts/ABA/origin moves,
   readiness/font/rail/transparency/gamma/state screenshots. Effective scale and
   causal freshness unknown cannot PASS. llvmpipe refusal is not GPU visual proof.
7. Record FD/handle/resource lifetimes over the real lifecycle including failed
   interop and close, addressing historical399→504 growth; reconcile historical
   whole-file mismatch without relabeling old PASS.
8. Same identified artifact host last through approved task control/manual user
   review. Native/platform/package/legal gates still cannot be substituted by
   source-module tests, synthetic screenshots, manifest self-consistency or
   implementer self-review. No delivery promise until all required gates clear.

## Author self-check and handoff

### B1 seven-gap disposition (author revision, not independent approval)

| Builder gap | Revision / remaining status |
|---|---|
| 1 atomic navigation permit | W1 inert blank→actual bind→host arm/navigate; exact tuple/atomic claim; separate non-consuming resource/HTTP checks; inert-order/timeout/replay and race tests specified. Native callback delivery still BLOCKED, no initial ordering assumption. |
| 2 control collision/refusal | W3 keeps documents closing index/null; additive nested hostClose; one real Outcome::Done outer error mapping for both quit endpoints; endpoint/caller tests specified. |
| 3 unapproved pending workflow | No parser/workflow change. B3 removes only CODE row, preserves licence asset/header; existing code inventory route. Canonical Vendored-table curation DEFERRED, not main-table closure. |
| 4 HAL cleanup | Read pinned early-path copies and INDEX addendum; core/front-end access gap is now closed. Narrow early HAL patch remains DRAFT; later cleanup/fault/ABI gates remain native BLOCKED, not solved by partial RAII. No further callgraph extraction/worker requests. F4 working Assistant remains BLOCKED. |
| 5 URL-log/config/module ownership | W1 owns assistant_cef_config validation and private-host log/argv settings on every desktop backend; main owns real HTTP/lifecycle module declarations. |
| 6 pub(super) harness | Cheap commands compile one crate-root wrapper importing actual production modules with the same hierarchy. No visibility edits/shadow native types. Commands are future/NOT_RUN, not a claimed build. |
| 7 eligibility timing | W2 traces pre-run_native setup through selected factory to later CreationContext/display handle; coarse bool cannot claim Wayland; actual adapter/backend gates and later refusal tests specified. |

### B3 resumed Q1–Q7 disposition (author revision, not independent approval)

| Question | Bounded resolution / remaining gate |
|---|---|
| Q1 attribution | W8 removes only welding CODE row, preserves LICENSE asset/header, uses existing code inventories; canonical Vendored-table release curation deferred, no parser/workflow/main-table closure. |
| Q2 wrapper temporaries | Conditional (b) explicitly proposes direct existing cef::sys::cef_initialize in owned welding runtime, with raw settings/app/args/loader retention and success transfers. Generated code/binary untouched; immediate callee/loader/ABI zero-post-false proof still BLOCKED. |
| Q3 false crashes/escalation | W3 owns startup/journal tickets, checked non-CEF finalization after typed normal return, durable cancellation classification, historical/concurrent ownership and real crash preservation. Fault/migration/multi-instance tests specified, NOT_RUN; no broader recovery-policy authorization. |
| Q4 initial callback order | W1 inert about:blank→actual bind→host arm/navigate; no privilege or permit during inert phase, absolute timeout/replay/resource-first tests; no assumed callback order. Native delivery remains BLOCKED. |
| Q5 dependency scope | W2/W8 exact same-version vendor/Cargo.patch/path-lock exception, input checksum/delta/licence route and human decision (a) in addendum §4; no other churn. |
| Q6 eframe error route | Completed INDEX AppCreator/error/return/conditional save_and_destroy excerpts support the source route, not downstream native destructor safety. Addendum §5 completed pinned receipt gives exact source ranges. |
| Q7 query/discard | pending iff Required/Closing, independent of quit_requested; sole accumulating quitting indices moved to staged vector with Some(empty) guard until commit/cancel; real queries/dirty saves preserved. |

The completed source copies close access gaps; no more extraction/recursive
callee-graph requests. Exact proposed approval sentences are addendum §4
**decision (a)** and §5 **decision (b)**; both remain DRAFT. Conditional direct
sys/init-frame and launch-bookkeeping details are in the owned W3 map, not
implicit generated-code changes or a replacement process architecture.
Native cleanup/ABI/loader/destructor, durable failure handling and platform
normal-return acceptance remain proof gates, not claims of safe normal cleanup.
Accepted transport A, selected adapter hook and successful-runtime close-timeout
quarantine are not reopened.

### R1-only independent follow-up disposition

`integration-builder-check-3.md` accepts the other proposal contracts for
written presentation, with one Important source-traced remaining state-validity
gap, not native/implementation approval. Addendum R1 and W3 R1 now specify the
existing-field engine snapshot/accessors, per-session consent timing, all-session
revalidation, real command/save/undo/redo/replacement/completion ownership, pending
work exclusion at commit and one last-action QuitCommit/terminal close latch.
Mismatch clears staged consent without marking newer work saved; late native
completion cannot resume quit. Named actual-module tests are planned/NOT_RUN.
The two DRAFT human sentences are byte-unchanged; no additional material
decision, dependency/native policy, work unit or reviewer fanout was introduced.

Coverage: all F1–F13 and each final-triaged minor has an owner/test/status or
explicit-deferral row. T5M3 canonical table curation remains deferred while its
wrong fold input is corrected; F4 accelerated operation is
blocked; this matrix is not a closure claim.
Type boundaries use actual wgpu/CEF/egui types where needed, optional native
factory after selection, existing poll_ime_composition and programmatic/UI quit
callers. Policy/transport/lifecycle pure tests are explicitly not ABI proof.
No new material decision is claimed human-approved; numerical engineering
bounds/signatures remain builder-checkable details. The source-backed minimum
HAL dependency-patch scope in addendum §4 and startup-cancellation policy in §5
are **DRAFT / need authorization**, not approved defaults/work or resolved
cleanup/native proof. They remain within the same nine-unit owner/wave if
approved, not new branches/tasks/reviewer fanout.

Next action: controller arranges **one scoped independent Sol R1 document
re-review** of these changes, then presents the two unchanged DRAFT human
decisions and its separate implementation gate. Builder followup is not human
approval, implementation authorization or the final scoped fix re-review.
This author does not delegate, implement,
commit, push, build, clean or independently approve the proposed repair.
