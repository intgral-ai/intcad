# CEF integration repair addendum

**Status: human-approved written design for one consolidated implementation wave.**
Independent builder review and the scoped R1 follow-up passed. The user then
explicitly approved both the specified same-version HAL repair and the
pre-document CEF-initialization failure policy. Historical DRAFT/proposal labels
below preserve that review chronology; those exact scopes are now authorized,
not broader alternatives. Native cleanup, zero-post-false-call, ABI and visual
enablement/acceptance gates remain binding and are not passing by approval.

Worktree: `/home/hyjing/Code/intgral/intcad/worktrees/cef-osr`;
branch: `probe/cef-welding-osr`;
frozen source: `c355edb977cd828f8d6a2f2de7bbdde792f8d1df`.
Companion plan: `docs/superpowers/plans/2026-11-19-cef-integration-repair.md`.

## 1. Authority, scope and evidence

This supplements, not replaces, the approved sections 1–5 of
`2026-11-18-cef-wayland-dsf-design.md`. The native-Wayland stock-CEF policy B,
OSR-only boundary, four scales, semantic fonts, themes, frame ownership and
VM-first/host-last gates remain binding.

The controller records the human's “接受你的推荐” after explaining these choices:

1. **Transport A:** keep private HTTP, with a separate ephemeral random capability
   covering bootstrap, every resource and IPC. Keep Agent/MCP business messages.
2. **CAD owns graphics:** an optional device-creation hook runs after CAD selects
   its adapter, using its complete descriptor. No second Assistant adapter.
3. **Close timeout:** cancel normal quit, quarantine the Assistant and retain
   unresolved native ownership while CAD remains responsive. No forced exit.

These choices are settled, not questions to reopen. Following the independent
written-contract review, the human separately approved the exact HAL dependency
repair and startup-cancellation scopes below. The contracts/bounds are the
implementation details of this reviewed design; new material architecture,
product, dependency or exit-policy extensions still require approval. Do not
substitute an alternative or infer native correctness from scope authorization.

Alternatives considered in the architecture audit: native CEF credential
attachment avoids URL secrets but expands native request/thread interfaces;
controlled exit avoids retaining an unresolved owner but changes document/exit
policy. Neither is selected or authorized. Transport A here is **not** the old,
unselected custom-Chromium “approach A”.

Inputs read: root `AGENTS.md`; ground-rules, crash-safety, commands, UI, UI-text,
tests, performance, packaging, agent-tools and contributing domain skills;
validation policy; approved spec; SDD `progress.md` (especially the corrected
budget ruling and human decision); `final-review/verdict-c355edb97.md` and
`final-review/architecture-repair-audit.md`. Paths prefixed `final-review/` below
are relative to `.superpowers/sdd/2026-11-18-cef-wayland-dsf/`.
Revision B1 responds to `integration-builder-check-1.md` in that SDD directory.
That check is **incomplete / changes required**, not builder approval. The
present B3 revision addresses Q1–Q7 of `integration-builder-check-2.md` under
the controller's bounded rulings; neither revision is independent approval.
The R1-only follow-up below addresses independent
`integration-builder-check-3.md`; its other contracts and both DRAFT approval
sentences are unchanged. This is a document revision, not approval of the fix.
The original verdict exists at
`/home/hyjing/Code/intgral/intcad/worktrees/cef-osr/.superpowers/sdd/2026-11-18-cef-wayland-dsf/final-review/verdict-c355edb97.md`;
its failed relative read was an access/cwd issue, not missing evidence.

All F1–F13 are **source-traced findings**, not reproduced exploits/crashes.
Some lifecycle problems predate the local welding patch. Do not call all findings
new density-policy regressions. The repair is one integrated L3 change owned by
one implementer, with sequential checkpoints and one scoped independent Sol
re-review against the completed broad verdict. No thirteen-fixer/reviewer fanout.

## 2. Existing seams that constrain the repair

| Source at frozen base | Actual contract / consequence |
|---|---|
| `apps/solidcraft/src/assistant_cef.rs:356–520,1607–1708` | `new -> Option<Self>` starts a detached server returning only a port; unbounded `channel`, request lines, body allocation and UI drain. Replace those ownership boundaries, not chat business logic. |
| `crates/ui-egui/src/agent_chat/web.rs:593–625` | `act(&mut SolveApp, &egui::Context, &str) -> Result<(), String>` allows **400,000 serialized UTF-8 bytes** (`MAX_MESSAGE * 4`); send text has **100,000 Unicode scalar values**. JSON overhead counts toward the byte limit. |
| Same file, `pictures`, `add_file`, `paste_picture`, `open_link`; `attach.rs` | Pictures use `Arc<Vec<u8>>`; normalized attachment picture limit is 3,750,000 bytes, eight files; existing attachment checks and web-address-only external opening stay authoritative. HTTP adds no lower attachment limit. |
| `assistant_page.rs` | Six embedded page files through `page_file`; host additionally serves `ipc.js`, Plex font and pictures. Built page references and `host.ts` picture URLs are relative. Root-absolute host-injected URLs must change together. |
| `vendor/egui-wgpu/src/lib.rs:417–468`, `setup.rs:173–209,261–283` | Adapter selection already receives compatible surface; descriptor callback returns `wgpu::DeviceDescriptor<'static>`. CAD needs limits, format features, plus `main.rs:1522–1527` float depth/stencil. Existing descriptor callback cannot add Vulkan native extensions. |
| `vendor/welding/src/native_frame/vulkan_dmabuf.rs:85–92` | `build_dmabuf_capable_device(&wgpu::Adapter, &wgpu::DeviceDescriptor<'_>) -> Result<(wgpu::Device, wgpu::Queue), ImportError>` delegates to grafting 0.6.2. No adapter selection belongs here. |
| `vendor/welding/src/runtime.rs:154–159,323–366,389–396` | Runtime is currently cloneable with unconditional shutdown in each Drop; Linux/macOS external pump, Windows dedicated CEF loop. Close request is not close completion. |
| `vendor/welding/src/{linux_cef,macos_cef}/mod.rs` and `cef_backed.rs` | Producer retains browser; `on_before_close` currently fails requests but does not expose completion. Windows already has shared optional browser, close-requested/closed flags and `is_closed`; repeated `close` resets its flag. Preserve useful plumbing, unify completion semantics. |
| `vendor/welding/src/surface.rs:586–592`, `view.rs` | `poll_ime_composition` already exists; returned bounds are **physical view pixels**, ranges come from CEF UTF-16 despite “characters” documentation. Physical input is converted to rounded DIP inside welding, not twice in app. |
| `main.rs:232–240,285–287`; `control.rs:1459–1469` | UI and control can send Close before native teardown; `app.quit` currently may open unsaved dialogs and reports quitting before completion. Both paths need the same host barrier. |
| `agent_chat/mod.rs:58–60,3405`; `main.rs:1063,1532` | Pure bridge has displaced the desktop-only `web` cfg; repair both, plus unconditional unsupported-target app references. |

Also inspected real pinned CEF 154.5.0+154.0.34 bindings:
`ImplRequestHandler::on_before_browse(..., request, user_gesture, is_redirect)
-> c_int`, `resource_request_handler(...) -> Option<ResourceRequestHandler>`,
and `ImplResourceRequestHandler::{on_before_resource_load -> ReturnValue,
on_resource_redirect(..., new_url: Option<&mut CefString>)}`. Redirect callback
does not itself return a cancellation boolean. Use pre-load cancellation too.
Do not invent a CEF hook or assert after-load enforcement is prevention.

## 3. Private HTTP and document authority — F1/F2

### 3.1 Credentials and routes

Use `getrandom::fill` as `control_server::new_token` does, but generate a **new,
independent 32-byte** capability, hex encoded. Never call `env_token`, read a
control/MCP token, use an environment override, or publish a discovery file.
Entropy failure refuses the Assistant without affecting CAD.

Canonical namespace is
`http://127.0.0.1:<owned-port>/__assistant/<64-lowercase-hex>/`.
There is no public landing page, redirect, bootstrap or asset outside it.
Only current namespace routes are valid:

* GET `index.html`, six whitelisted embedded files, `ipc.js`,
  `intcad-plex.ttf`, and `picture/<single encoded key>`;
* POST `ipc` with the **unchanged JSON body** used by `window.ipc.postMessage`.

The shim resolves `ipc` against the document directory, not `/ipc`; DOM probe,
scope probe and bridge injection use that same shim. Serve shim before module
execution. Keep relative module/style/font/picture loads within the namespace;
rewrite only the host's root-absolute injections. No product bundle rebuild or
new Node runtime is needed for this repair.

Require exact `Host: 127.0.0.1:<port>`. POST also requires the exact expected
`Origin` (scheme/host/port, no path). GET may omit Origin, as normal navigation
does; reject a present foreign Origin. Host/Origin are supplementary, **not**
credentials. No CORS, permissive OPTIONS, cookies or Referer-based authority.
Unknown, missing, expired or malformed capability fails before body allocation
and before pictures/static files are exposed; return a generic 404.

Use `Cache-Control: no-store`, `Referrer-Policy: no-referrer`, `nosniff`, and the
existing restrictive CSP, adding explicit no frames/objects/workers and retaining
same-origin scripts/styles/fonts/connect plus existing data/blob image support.
No capability in external URLs, page telemetry, public receipts or control state.
Do not log raw request lines, bodies, authorization data, page text, IME content,
navigation URLs or error strings containing URLs. Production native CEF logging
must not persist capability-bearing URLs: configure disabled native file logging,
retain sanitized typed host diagnostics, and reject diagnostic overrides that
bypass this containment. Structured scope logs remain bounded; redact private
namespace URLs and known current/revoked credentials before any host log sink.
Tests include credentials deliberately embedded in error/log text.

Containment has explicit owners: `assistant_cef_config.rs` adds
`validate_private_http_overrides(&[Switch]) -> Result<(), PolicyError>`, called
from `assistant_cef.rs` **on every desktop backend**, after parsing overrides
but before `build_policy`/native initialization. Compare ASCII-lowercased names;
reject those containing `log`, starting with `trace`, or equal to `v`/`vmodule`;
this deliberately covers
enable-logging, log-file, log-level/log-severity, log-net-log,
net-log-capture-mode, trace-startup and trace-to-file. Refuse the Assistant, do
not silently strip a rejected override. Keep unrelated non-B feature/Ozone
semantics unchanged. Existing generic parser acceptance of enable-logging is not
permission for this private-page host.
Also retain the existing production bans on remote-debugging* and
auto-open-devtools-for-tabs for this host on every desktop backend, not only B;
an alternate debug/log sink is not a transport-authentication exception.

The private-HTTP runtime constructor sets `CefRuntimeConfig.log_severity` to
`CefLogSeverity::Disable` and `command_line_args_disabled=true` for all its
desktop instances, preventing inherited browser argv from undoing containment.
This does not change the general welding default or CEF subprocess argument
dispatch. Preserve the existing B-specific DSF policy method/tests; add
private-host containment tests rather than falsely asserting that its old
B-only method already protects Windows/macOS. Main owns the desktop-gated
`assistant_cef_http` and `assistant_cef_lifecycle` module declarations; neither
module is an unregistered source file.

### 3.2 Replacement is host-owned, not `ready`-owned

Separate `DocumentId` from frame/input/theme tickets. Before any host reload,
renderer recovery or full-document replacement: synchronously revoke old
capability and queue authority, cancel input/composition/copy requests, stop
privileged script/state delivery, then generate a fresh namespace and arm a
single main-document navigation permit on an already bound browser. Initial
producer creation first uses the inert sequence below; neither creation nor
HTTP requests claim or consume a private navigation permit.

The authority record binds **DocumentId, BrowserOwnerId, actual CEF browser
identifier, exact canonical bootstrap URL and permit state** under one lock.
BrowserOwnerId is a checked host-issued creation/lifetime counter captured by
the producer's policy closure, not a CEF field or renderer-supplied value.
DocumentId is resolved from the stored namespace/capability and URL; CEF supplies
no document-authority id. Do **not** assume `on_after_created` precedes the first
`on_before_browse`: the supplied pinned contract has not established that order.
Use explicit `CreatingInert → BoundInert → Armed → Claimed`:

1. Reserve BrowserOwnerId and create the producer with literal `about:blank`,
   never the capability URL. CreatingInert admits only that exact, non-redirect,
   main-frame inert bootstrap for that captured owner (even before id binding).
   It supplies no capability, private resources, page shim, privileged scripts,
   host state, ready acceptance or business-message authority. It spends no
   private permit. No iframe, popup browser, external URL or blank variant.
2. The real life-span `on_after_created` passes the actual browser id directly
   into the synchronized policy, binding once. A bounded owner-only notification
   tells the host it can proceed; it does not itself arm or call navigation.
   Duplicate same-id delivery is idempotent; another id is rejected.
3. Only after observing BoundInert does the host generate a fresh document/
   namespace, arm its exact URL for that bound tuple, release the policy lock,
   then call existing `navigate_to_url`. It never waits for blank-page ready/
   load completion or runs privileged bootstrap code there. No polling loop
   blocks the UI. A late inert callback is cancelled once Armed/Claimed; it
   cannot spend/revoke/renew the private permit. Discard blank-page paint/input.
4. CreatingInert/BoundInert share a **5-second absolute** creation-to-private-
   navigation deadline. Missing binding, navigation error or deadline revokes
   authority and enters W3's owned close path, including pending/late creation.
   It does not retry initial creation into an overlapping owner.

This sequence owns both possible callback orders without inventing a CEF hook.
For later host reloads the browser is already bound: revoke, generate fresh URL,
arm, then navigate. Binding another id or rearming a claimed/revoked document
fails. No wildcard id ever grants private authority. Real callback/thread and
host-navigation ordering on each platform remains a native validation gate.

`claim_main_navigation(identity, main_frame, is_redirect)` atomically compares
the entire bound tuple and changes **Armed → Claimed exactly once**. Two matching
concurrent callbacks cannot both succeed. Revocation uses the same exclusive
lock, invalidating armed and claimed tuples; a claim racing revocation either
linearizes first and is subsequently revoked, or fails. Redirects and non-main
frames never consume it. A same-URL reload after consumption must request host
renewal into a fresh DocumentId/capability, not rearm the old permit.

Resource/IPC authorization is a **separate, non-consuming** check requiring the
current document to be Claimed. An asset/IPC arriving before main claim is
refused without changing Armed state; assets arriving after claim cannot spend
or renew the permit. The HTTP server can validate current capability/Host/Origin,
but has no CEF browser identity and cannot bind/claim one.

An optional, Assistant-specific request policy in `CefSurfaceConfig` shares the
authority state with the server. It is installed in **all three** platform
request handlers. Outside the explicitly inert creation phase,
`on_before_browse` accepts only the armed exact main document;
no untrusted iframe/document URL. A reload/back/full navigation reusing a consumed
permit is cancelled before commit. A bounded renewal event can ask the host to
perform the revoke→fresh-URL navigation sequence, never authorize the old URL.
Same-document fragments do not grant a new document authority.
The adapter obtains browser id from the callback's Browser, main-frame status
from `Frame::is_main()`, target URL from the Request, and redirect flag from
`on_before_browse`'s existing argument. Do not use `Frame::url()` as the pending
target or invent a renderer-document id. Resource/redirect callbacks supply their
own request/new URL and browser identity, and only perform non-consuming checks.
The same captured policy closure is passed to the life-span and request handlers
for that producer; a queued BrowserCreated event in the app is too late to bind
the synchronous private-navigation check. The bounded host notification above
is safe only because no private navigation starts until the host sees binding.

At resource pre-load, allow only the current private assets/IPC destination and
existing data/blob **image** use; deny external network, unauthorized frames,
scripts and downloads. Enforce redirects at resource redirection and again at
pre-load; the server never redirects. Fail closed if CEF identity/frame/request
information is absent. Callbacks consult shared bounded state, not synchronous
round trips to the window thread. Do not hold a policy lock across CEF calls.
Keep popup-browser denial, native DevTools refusal and genuine OSR `<select>`/
fullscreen widgets. `web::act` attachment selection and external `open_link`
behavior are not navigation exceptions and must still work.

Each accepted IPC entry carries its **validated DocumentId** outside its business
JSON. Recheck that id and current phase at UI dispatch. Revocation removes queued
old entries and marks any racing enqueue stale. No old ready/theme/log/action may
restore authority. A `ready` message only marks the already permitted current
document page-ready; it no longer increments/replaces document authority itself.
Dispatch and revocation must have a defined synchronization point, including
Windows callback concurrency, so revocation cannot race a privileged action.
Use a current-document dispatch lease, obtained after dequeue and held through
the bounded `receive`/`web::act` decision; revocation takes the exclusive side.
No socket write or CEF/native call occurs while holding that lease. Split
housekeeping that requests native repaint/navigation into deferred host work
after releasing it. A boolean `permits` check followed by an unlocked action is
not this synchronization contract.

A capability/nonce identifies authority, **not renderer-commit or pixel
freshness**. A static nonce echoed by a delayed document or paint proves neither.
Native frame freshness remains the bounded original contract in section 7.

### 3.3 Fixed engineering budgets (author-selected, not new business limits)

| Boundary | Limit / action |
|---|---|
| Request line / individual header | 2,048 / 8,192 bytes including delimiter |
| Total head / header count | 16,384 bytes / 64 headers; cap while reading, not afterwards |
| IPC body | 400,000 bytes inclusive; UTF-8 must decode strictly; never lossy conversion |
| Framing | HTTP/1.1, origin-form canonical target, GET or POST by route; exactly one decimal Content-Length for POST; no TE, duplicate CL, signed/overflow length, folded headers, Upgrade, Expect, embedded CR/LF or pipelining |
| GET body | No body; absent CL or CL=0 only |
| Absolute deadlines | 5 seconds for the entire head+body from accept; separate 5-second response deadline; timeouts recomputed from remaining time, not reset per byte |
| Concurrency | Four fixed workers, at most four active connections, no unbounded accepted-socket backlog or thread-per-request |
| Inbox | 32 entries **and** 3,200,000 serialized bytes total, including housekeeping |
| UI drain | At most eight entries **and** 800,000 serialized bytes per logic turn; retain FIFO for live entries; request another turn if work remains |
| Stop | Revoke first; stop accepting, interrupt owned sockets, observe worker completion asynchronously; no UI-thread join/wait loop |

The listener's OS backlog is not application authorization or queue capacity.
No allocation from attacker CL before head, route, credential, framing and length
validation. Use checked arithmetic and `try_reserve` at request-derived buffers
and queue storage. Incremental head parsing uses bounded storage. Reject invalid
UTF-8/ambiguous paths (`..`, encoded separators, double encoding, absolute URLs,
userinfo, query secrets, NUL) rather than normalize to an authorized resource.

Share embedded asset bytes and picture `Arc`s instead of `to_vec` per request.
The four response leases bound concurrent picture references to four; retain the
existing picture bytes/attachment policy, not a second accumulating cache.
Use short lock scopes; never write a network response while holding pictures or
authority locks. Request and response deadlines also cover slow readers.

Only successful queue insertion returns 200 `ok` (receipt of enqueue, not action
completion). Full/stopped queue returns 503; framing/length failures use bounded
400/413/431 responses. Shim checks non-2xx/rejected fetch and exposes a bounded
transport failure on the existing host/failure path; the server also records a
non-secret failure flag for UI visibility. **Never auto-replay** approvals,
permission changes, sends or other non-idempotent actions. Existing `ready`,
`theme-ready`, `focus`, `log` and fixed scope reports use the same queue budget;
the 600-character log cap is not the business-message cap.

The returned `PageServer` owns port, cancellation, active sockets, workers,
completion and inbox. Failed creation after server start must stop/reap it.
No pictures/listener survives a failed initialization or completed close.
Server stop timeout is reported as incomplete teardown, not hidden by Drop.

## 4. CAD-selected device construction — F4

Remove `interop_setup`'s independent instance/adapter and the unconditional
`WgpuSetup::Existing` replacement in `wgpu_options`. Keep all existing preference,
environment, compatible-surface, software-recovery and device-loss logic.

Add exactly one optional native factory field to `WgpuSetupCreateNew`:

```rust
pub type NativeDeviceCreator = Arc<
    dyn Fn(&wgpu::Adapter, &wgpu::DeviceDescriptor<'static>)
        -> Result<Option<(wgpu::Device, wgpu::Queue)>, String> + Send + Sync
>;
// native_device_creator: Option<NativeDeviceCreator>; default None
```

This is a proposed API, not a claim it exists. Clone/Debug/default/destructuring
must include it. Native `RenderState::create` calls the existing descriptor
callback **once after adapter selection**, then invokes the factory with that
same adapter and descriptor. No hook on wasm; `Existing` remains caller-owned.
`None` means ordinary `adapter.request_device(&base_descriptor)`. `Some` must
have used that descriptor with only supported additive interop features.
Clean failure is diagnosed and uses ordinary creation on the **same** adapter.

`assistant_cef::configure_interop(&mut WgpuSetupCreateNew, eligible: bool)` installs
the Linux factory only when **coarse** prerequisites are available, interop is
enabled, and this is not CAD software-recovery mode. `main.rs:1168` calls
`wgpu_options` before `eframe::run_native` at 1254; no CreationContext/display
handle exists there. The early bool means only runtime-path availability,
explicit interop enablement and non-software eligibility. It proves no display
backend or loaded CEF identity and performs no CEF initialization.

At selected-device creation (`RenderState::create`) the callback can check the
real selected adapter/backend/features and cleanup eligibility, but still cannot
claim the later app display handle. The existing `CefAssistant::new(...,
host_backend(cc))` at 1443–1458 decides actual policy too late for the DRAFT
startup guarantee. If §5 is approved, move only runtime preparation/initialization
to AppCreator entry, using the same actual CreationContext display handle and
selected render state; retain browser attachment later without reinitializing.
Actual native-Wayland policy/runtime identity still precedes initialization.
Never infer that policy from WAYLAND_DISPLAY, DISPLAY, XDG_SESSION_TYPE or the
early eligibility bool. A coarse candidate later refused by actual host policy
may leave supported additive interop on the same CAD device; it creates no
Assistant browser/runtime and does not rewrite CAD configuration to recover.
The callback
checks the **selected** backend/features; GL/non-Vulkan, unsupported hardware,
missing CEF and `NO_INTEROP` leave ordinary CAD intact and refuse Assistant OSR.
Do not override an existing foreign factory silently.

The augmented descriptor preserves label, every CAD required feature, every
required limit, memory hints, experimental features and trace settings. It adds
`VULKAN_EXTERNAL_MEMORY_DMA_BUF` only when supported. It retains
`TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` and `DEPTH32FLOAT_STENCIL8` where CAD
requested them. Surface/MSAA selection and `DeviceLost::watch` still use the
selected adapter/device. No “fix” by lowering limits or switching adapter/backend.

**Failure cleanup prerequisite:** grafting's actual helper checks four Vulkan
extensions, calls HAL `open_with_callback`, then consumes `OpenDevice` through
`create_device_from_hal`. The extension list includes queue-family-foreign. An
Err is not evidence all intermediate native ownership was released. Trace and
record the exact locked HAL/core failure paths (raw-device construction,
wrapper creation, memory/queue allocation, registration) before enabling fallback
after an attempted interop creation. All temporary native owners must release
before ordinary request_device; no second live logical device hidden in an Err.
If the current helper cannot establish that contract, leave that attempted path
disabled/refuse Assistant and report the precise blocker; do not claim a native
cleanup pass, mutate the registry, add unsafe in app, or silently expand to a new
graphics framework/dependency fork. Native fault/cleanup validation remains gated.

### Pinned-source result and DRAFT minimum dependency repair

Evidence is now readable in SDD `builder-evidence/INDEX.md`. Its receipt binds
archives to Cargo.lock and excerpt files to the exact archived source; reuse
those copies, do not duplicate extraction. Source result (original line numbers):

* grafting 0.6.2 `src/vulkan_dmabuf.rs:664–725` checks extensions, then calls
  HAL `open_with_callback` and front-end `create_device_from_hal`. On failure it
  returns an error string, not the raw device or a cleanup owner.
* HAL30.0.1 `src/vulkan/adapter.rs:2991–3067` creates the raw device and passes
  **None** as drop_callback to `device_from_raw`.
* Whole `device_from_raw:2562–2985` creates
  `DropGuard::from_option(drop_callback)` at 2897; that is **None** on this
  path (`src/lib.rs:342–371`). The layout allocation `?` at 2902 precedes
  DeviceShared construction. The excerpt contains no owned-device destruction
  on that early return. A DropGuard declaration by itself is therefore not
  cleanup for the None path.
* Later `?` at 2940/2963 occurs after DeviceShared exists. Its Drop
  (`src/vulkan/mod.rs:505–573`) destroys the layout and, for None drop_guard,
  device. Those later shared-owner paths do not cover the earlier return.
* The INDEX addendum now supplies core Device::new/Drop, Queue::new/Drop,
  late-init, acquire_encoder and front-end wgpu30.0.1 conversion/Drop bodies.
  These are the exact locked versions, not nearby 30.0.2 or an assumed 30.0.0.
  The front-end error propagates before CoreDevice/CoreQueue handles exist;
  Device::new has fallible partial construction, and Queue::Drop can wait/panic.
  Seeing boxed/Arc ownership alone does not prove bounded native cleanup.

This establishes a concrete **source cleanup gap on the helper's early HAL
error path**, not an executed fault/leak count or a proof of all later native
cleanup. Neither the app nor owned welding receives the lost raw-device owner
on Err, so a safe post-error cleanup cannot be added at those boundaries by
guessing a handle. Adding a catch/drop around welding's returned String does
not fix it. Refusing to enter the helper preserves CAD but does not meet the
working accelerated Assistant goal.

**DRAFT — minimum source-backed scope proposal, not approved work:**

1. Make a repository-owned patch of the **same** wgpu-hal30.0.1 package, authenticated
   from the Cargo.lock archive checksum
   `b6b7fb58561a792bc237628ba0792e332de418fefe145f13b5ed8201e6d52f58`.
   Proposed files: faithful crate input under `vendor/wgpu-hal/` with original
   licences, a narrow local delta in `src/vulkan/adapter.rs`, root Cargo.toml
    `[patch.crates-io] wgpu-hal = { path = "vendor/wgpu-hal" }` registration
    (merge into the existing patch table) and resulting Cargo.lock path-source
    change; existing `vendor/README.md` inventory and notices. W2 owns the
    faithful crate input, adapter delta and Cargo files; W8 owns provenance/
    notices and licence-text asset rows in the existing four-column pending
    format. No CODE row in generic Files pending. No registry edit, version upgrade, core/grafting
   fork, feature/limit change or new attribution workflow is included.
2. At the existing layout-allocation error boundary in `device_from_raw`,
   explicitly release the **owned** raw device when drop_callback/guard is None,
   before returning the same mapped error. Preserve the Some(callback) contract:
   existing callback runs once and no new independent destroy is issued.
   Success transfers to the existing DeviceShared owner; later failures retain
   existing layout/device destruction order. No double destruction, and no
   destruction of a caller-managed borrowed device.
3. Add regression tests at the real HAL Vulkan adapter initialization seam
   (within `src/vulkan/adapter.rs`'s test area, not a per-bug file): fail layout
   allocation with None → exactly one destroy before Err; Some callback →
   callback once/no HAL destroy; success → no early destroy; later semaphore/
   allocator failure → existing shared release exactly once. Native tests use
   actual HAL/native types and an authorized fault-injection seam, not a fake GPU
   implementation or source harness claimed as cleanup proof.
4. The supplied additional core/front-end bodies close the requested access
   gap, **not** fallback-safety proof. Bound the investigation here: no recursive
   full-wgpu callgraph extraction or further worker requests. Preserve explicit
   native proof gates for partial Device::new allocations, queue/late-init
   failures, disposal/idle-wait behavior and final raw-device release before a
   fallback request. Tests remain BLOCKED until authorized native validation.
   Do not enable a path whose cleanup remains unproven; do not infer safety
   from partial RAII or silently add core/grafting patches.

The dependency input and native-error-path delta affect shared Vulkan device
construction, so controller authorization and independent builder review are
required before adding them to the unified wave. No dependency/product edit or
fault test is performed by this document author. Actual GPU/cleanup/ABI tests
remain BLOCKED by resource policy even after source design approval.
Until this scope and proof gate clear, leave the interop-attempt path disabled:
F4 is at most **source CAD-preservation only; accelerated Assistant/F4 working
goal and delivery BLOCKED**. No binary-delivery or full-repair-ready claim.

**Exact proposed user decision (a):**
> Approve faithfully vendoring the locked wgpu-hal 30.0.1 archive with its
> provenance/original licences and only the specified early layout-failure
> cleanup delta/tests in vendor/wgpu-hal/src/vulkan/adapter.rs, including root
> Cargo.toml patch/Cargo.lock same-version path-source changes owned by W2 and
> existing vendor inventory/notices/licence-text asset attribution owned by W8,
> without version upgrade, core/grafting fork or other dependency churn, subject
> to native cleanup/ABI proof before interop enablement or acceptance.

## 5. Runtime, close, quit and Retry — F5/F8

One non-cloneable, initialization-thread-affine `CefRuntime` owns process state.
Replace unconditional shutdown-on-Drop with explicit checked shutdown.
Track pending browser creation as well as live browser owners before issuing
native creation. Never create a second owner after partial initialization.
An immutable policy claim is not sufficient runtime ownership.

Lifecycle:

`Unavailable → Initializing → Running → Closing → Closed → Shutdown`

`Initializing/Running/Closing → Quarantined` on unresolved ownership/failure.
The DRAFT native-initialize=false startup branch below is separate:
`Initializing → StartupCancelled → normal error return`; it is not Running
quarantine and is not authorized by the approved close-timeout policy.

1. Begin close **while the event loop runs**, before sending viewport Close.
   Revoke authority, cancel input/IME/copy and work, stop server, request browser
   close exactly once. Keep browser identity until matching completion.
2. Linux/macOS: pump on CEF's initialization/main thread, one bounded turn per
   app logic turn. Windows: dedicated CEF UI loop delivers callbacks into shared
   state; do not call an external pump there. Request/create/close API use must
   match pinned CEF's allowed threads; callback receipt is not permission to run
   shutdown from that callback thread.
3. `on_before_close` records the matching browser/creation owner, clears native
   browser references, fails outstanding requests and publishes completion. The
   consumer advances only after callback return, not from inside the callback.
   Late creation after a close request must still close and complete. Cancelled
   deferred creation must account for its pending owner without inventing a
   before-close callback for a browser that never existed.
   Pinned callback docs also permit final frame/popup and IO resource callbacks
   after on_before_close. Retain inert shared policy state to cancel those;
   do not infer all callbacks are gone from the browser-close receipt or call
   browser operations from them. Native callback/shutdown ordering stays gated.
4. Observe completion, release producer/browser/frame/request-context/native
   graphics and page/popup registrations, stop/reap server and pending tasks.
   Only then call shutdown **once on the initialization thread**. Shared
   callback state must not retain CEF references. No native calls after shutdown.
5. Elapsed close budget is **5 seconds** (author-selected finite diagnostic
   threshold, measured from close start, not reset by callbacks). At timeout:
   cancel requested quit, send CancelClose, set host barrier Quarantined, show a
   localized non-modal diagnostic, retain unresolved owner, reject Retry/new
   runtime. Do not block the window, spin, join indefinitely or force-exit.
   Late completion may safely finish cleanup; it must not silently resume the
   cancelled quit or recreate Assistant. A subsequent explicit quit is eligible
   only after the ownership barrier is genuinely clear.

Drop and partial-init paths must fail closed: no unconditional native shutdown
with outstanding browser creation/references, no field-order-only fix. If the
app object is dropped abnormally while ownership is unresolved, a same-thread
quarantine holder retains the entire owner bundle, not just the runtime field.
This containment is explicitly **not** clean shutdown or leak closure.
Retain the process-global unique-owner claim as well; a leaked bundle must not
allow a second initialization. The quarantine holder must not automatically
destroy its native bundle at thread-local teardown; retain it without a native
destructor unless explicit safe completion releases it on the owner thread.
No unconditional Drop shutdown/pumping, field reordering or
process-exit side effect can turn quarantine into successful teardown.

### DRAFT — cancel startup on native initialize=false, before CAD documents

**One policy proposal, not approval or a new default.** If native browser-process
`cef_initialize` returns false, cancel **this new launch** through the normal
AppCreator error → run_native return → main error-return path. Report “Intcad
startup cancelled: CEF did not initialize; no design was opened.” This does not
misclassify a native early-exit request as a crash. No forced
kill/process::exit shortcut, signal, restart, fallback process or action against
another Intcad instance. Do not open/restore/modify any user or recovery design
before this decision. Pre-initialization missing CEF/policy/interop refusal may
still start CAD without Assistant, as before.

**Actual ordering audit (frozen main.rs):**

| Existing source | Consequence for this proposal |
|---|---|
| `main:1060–1065` | Existing CEF subprocess dispatch remains first; its separate subprocess exit is not the browser-process startup-failure policy and is not expanded. |
| `main:1081–1254` | Settings validation/adoption, startup counter/crash setup, graphics/window options and `run_native` precede AppCreator. There is no user-design open or recovery restoration here. Do not claim zero startup filesystem bookkeeping. |
| `main:1257–1269` | AppCreator already receives actual display handle/render state. It currently starts font loading and desktop chooser before `SolveApp::new(Session::default(), services(...))`. Put the proposed gate at its **entry**, before those calls or SolveApp construction. |
| `ui-egui/lib.rs:825–836` | SolveApp construction creates a render job host and cleans old slicer files; it is not an inert container to construct before a “safe” startup gate. |
| `main:1345–1352,1353–1367` | Control endpoint, agent control starter, sample modeling and document-lock infrastructure currently precede CEF. All must remain after the proposed early gate. |
| `main:1369–1417` | Finder queue consumption, reopen planning, `open_path_async`, new-design preferences and recovery/autosave start currently precede CEF. The existing CEF call at 1443–1458 is therefore **not** a safe startup-cancellation seam. |
| `open_documents.rs:26–58,89–96` | macOS pre-loop installation only queues paths/wakes a UI; actual `take`/open remains after the gate. Do not consume those paths on cancelled startup. |
| `App::logic:206–225` | CLI-link/AI-connect/agent autostarts run on the second app frame. No App returned means those paths must never run on native false. |

Thus **a source-level pre-document ordering seam exists**, not proof of a
fully compliant native abort with the present implementation. It is AppCreator
entry on the application/event-loop thread, after the selected CAD device and
actual display handle exist, **not** early `wgpu_options` or an environment
guess. Linux/macOS external pumping and Windows dedicated browser loop do not
change CEF's requirement to initialize on the main application thread.
Native thread/return behavior on all platforms remains unvalidated.

**Smallest same-process sequencing change, conditional on approval:**

1. At that entry, run current private-host config/identity/actual-adapter checks
   and browser-process runtime initialization only. No page server, producer,
   browser, SolveApp, control endpoint, recovery or agent work exists yet.
   An ineligible/pre-init refusal returns “Assistant unavailable” and continues
   ordinary CAD startup without ever calling cef_initialize.
2. Success returns one thread-affine prepared runtime owner. Carry it through
   the same closure and consume it at later Assistant browser attachment;
   never call initialize again at the old 1443–1458 location or on Retry.
   Subsequent browser/page failures use the already specified live-runtime
   close/quarantine rules; actual native false cannot become a late document
   shutdown problem.
3. On native false, return a dedicated `StartupCancelled` error containing only
   a constant diagnostic (no CEF references). Optional GetExitCode is allowed
   by pinned CEF, but this minimal proposal **does not call it**. No pump,
   shutdown, retry, browser creation or CEF-object release is allowed afterwards.
   Report via the normal error/diagnostic sink, not a programmatic dialog.
4. Explicitly use eframe `run_and_return=true`. Its 0.36.2 source
   `wgpu_integration.rs:318–334,379–384` maps the AppCreator error before storing
   a running App; `run.rs:161–183,385–395,450–470` records it, exits the event loop
    and returns it on that path. The event-loop exiting callback still calls
    save_and_destroy, but its wgpu implementation425–429 acts only if a running
    App exists. This gives a source error-return route, not a native execution
   receipt or permission to use eframe's run-and-exit branch.
5. Classify typed `Error::AppCreation(StartupCancelled)` **before** main's
   generic `created/no_window` fallback at 1482–1485. Otherwise a reordered
   created flag can relaunch another backend/process or show the no_window
   dialog (`1540–1581`). Cancelled startup must do neither. No `App::on_exit`,
   App Drop update/device-loss restart, recovery discard or document-save flow
   is invoked because App/SolveApp was never built.
6. After run_native returns the typed cancellation, perform the NON-CEF
   launch-ticket/journal finalization below, then return the startup error.
   Do not call `startup::started`, generic no_window or normal App close hooks.
   No other PID, token, socket, recovery entry or settings cleanup is permitted.

**Launch-owned bookkeeping contract (conditional part of (b), not fake success):**

Current `startup::begin:103–109` reads numeric `starting`, writes before+1 and
returns before; `started:113–115` deletes that shared file without ownership.
Main stores only its directory in `App.starting` and clears it at frame two
(`main:91–92,224–226`); no_window also calls started1545. Meanwhile
`crash::install:300–341` scans earlier dead journals and opens this process's
journal before CEF (`main:1178`). `Journal::open:661–680` uses a locked per-run
path; STATE is process-local. `end_session:414–427` removes its current journal
and persists known counts, but silently ignores lock/unlink errors. Neither
deleting shared `starting` nor just returning an error meets this proposal.

Replace path-only startup accounting with the following narrow owned interfaces
in existing startup/main/crash modules (no recovery-document format change):

```rust
// startup.rs; fields private; non-Clone; holds lifetime lock, never PID-only
pub struct StartupTicket { /* unique launch id, own entry, observed history */ }
pub fn begin_owned(dir: &Path) -> Result<StartupTicket, StartupError>;
impl StartupTicket {
    pub fn prior_failed_starts(&self) -> u32;
    pub fn cancel_controlled(&mut self) -> Result<(), StartupError>;
    pub fn mark_window_started(&mut self) -> Result<(), StartupError>;
    pub fn retire_no_window_attempt(&mut self) -> Result<(), StartupError>;
}
// engine/crash.rs; opaque token bound to current process-local journal generation
pub struct StartupSessionTicket { /* no CEF refs, no arbitrary path input */ }
pub fn startup_session_ticket() -> Result<Option<StartupSessionTicket>, CrashError>;
pub fn cancel_startup_session(ticket: &mut StartupSessionTicket)
    -> Result<(), CrashError>;
```

StartupError distinguishes Busy/Io/InvalidRecord/Ownership/HistoryCapacity;
the proposed checked crash API distinguishes Io/Ownership/NotPreDocument/
Poisoned. Errors do not expose private paths/CEF data. No startup-ticket Drop
calls CEF. Native initialization cannot proceed when required ownership or
durable bookkeeping prerequisites are unavailable.

* `begin_owned` runs at the existing pre-graphics begin position. Use a small
  versioned startup ledger in the settings directory, a stable coordinator
  File lock for short read/modify/write sections, and unique create-new per-run
  entries with lifetime File locks. No UI blocking lock waits; contention/errors
  return typed errors. At first migration, import the clamped legacy numeric
  count once as historical baseline under the coordinator; leave original
  `starting` bytes untouched and record the import in the ledger. New calls no
  longer use legacy begin/started. This is local crash bookkeeping, not a new
  design file or recovery migration.
* `prior_failed_starts` counts baseline plus provably dead unresolved tickets,
  saturating at the existing 1000 bound; active/Cancelled/Started entries add
  zero. A busy lifetime lock is active, an access/IO error is **not** proof of
  death. Shared ledger access and owner transitions use the same coordinator.
  Bound record sizes/counts and parsing; inability to account safely is an
  error, not invented zero history, automatic safe-mode/CPU escalation or
  permission to delete entries. Mixed legacy-binary concurrent writes cannot
  be proven owned; migration compatibility remains a validation gate, not a
  claim to clear an older running launch.
* `cancel_controlled` durably marks/removes **only its own** unresolved entry;
  it never resets baseline or another ticket, never pretends a window opened,
  and is idempotent. Keep an acknowledged terminal record if unlink fails.
  `mark_window_started` is the distinct real second-frame success operation:
  retire this entry and only baseline/dead failure generations observed by
  this ticket, preserving later/active launches. Move the ticket, not a PathBuf,
  into App on success. Audit no_window/retry transfer: normal renderer fallback
  retains the existing no_window retirement/retry decision through distinct
  `retire_no_window_attempt`: retire its own entry and observed historical
  snapshot as the old no_window→started path did, never later/active launches
  and never claim a window opened; the controlled CEF branch bypasses it entirely
  and must **not** retire any historical snapshot.
  Dropping an unresolved ticket releases its lock but never marks success/
  cancellation; a genuine premature death still counts.
* `startup_session_ticket` is captured immediately after the existing install,
  before CEF; None means no persisted journal exists for this launch, not a
  guessed path or permission to ignore an ownership/IO error on an existing
  journal. A token mismatch returns error without mutation. Cancellation
  verifies this journal has no user design/command, RUNNING phase or actual
  panic/native-crash evidence. Under its existing mutex, write and sync a
  terminal `controlled_startup_cancel` record before releasing the file lock,
  then retire this journal through a checked shared end-session helper.
  Reuse the existing persistence of genuine report repeat counts; do not erase
  reports, `.seen`, autosaves/reopen guards, prior native evidence or recovery
  data. Do not rerun install/dead-journal scan as cleanup.
* The journal reader/dead-journal classifier recognizes that terminal record
  only for an otherwise empty pre-document journal. If unlink fails after the
  durable terminal record, a subsequent install must not generate an unclean
  report/CPU escalation from it. Actual panic/native evidence or commands
  override cancellation; no general suppression of real crashes is permitted.
  Refactor existing `end_session()` through the common owned helper, preserving
  its ordinary callers; the checked startup API must not inherit silent errors.
* Ordering: obtain both tickets pre-gate → call CEF → retain false-path native
  inputs → typed AppCreator error → eframe returns without an App → cancel own
  journal session → cancel own startup ticket → emit diagnostic/return error.
  These final operations use Rust/filesystem/journal APIs only, never CEF,
  control endpoint cleanup, App hooks or device_lost::end. Both completions
  must be acknowledged before claiming clean controlled cancellation.
  Partial writes/lock poison/permission/disk errors surface a bookkeeping error
  and leave owned evidence intact; never clear history to force success.
  Unresolved persistence/fault behavior is an enabling BLOCKER, not a native
  PASS. If no durable cancellation contract can be met, do not enable (b).

Tests in existing startup/main/crash test modules must show two or more
controlled aborts add **zero** failed starts/crash reports and no CPU/safe-mode
change; prior genuine failures/reports remain; real subsequent premature death
still counts; controlled cancellation never performs the historical reset of
real second-frame success or the existing generic no_window branch;
two live launches cancelling/starting in either order never erase each other's
entry; wrong/replayed tickets, busy locks, damaged/legacy input, denied writes,
unlink failure after terminal sync and native-evidence-overrides-cancel are
handled explicitly. Assert existing dirty/recovery/reopen bytes unchanged.
Bookkeeping tests are not CEF execution. Broader recovery behavior changes need
a new material decision; only ignoring this controlled, pre-document abort is
proposed here.

**Runtime ownership is a real blocking detail, not “return Err and drop”:**
`vendor/welding/runtime.rs:324–343` creates local Args/App/Settings, calls
initialize and returns `InitFailed { code:0 }` without constructing CefRuntime.
Its existing scopes still drop those locals. Pinned cef154.5.0 source shows
`Settings` holds CefStrings (`bindings/...:556–588`),
`CefStringUtf16::drop` calls `cef_string_utf16_clear` (`string.rs:477–486`), and
`RefGuard::drop` calls release (`rc.rs:304–307`); App wraps RefGuard.
Moving the call earlier alone does **not** establish “no CEF API after false”.

The owned vendor init boundary must retain its complete native init-call inputs/
callback refs without automatic destruction on the false branch, including at
thread-local/process teardown; no CefRuntime shutdown owner is constructed.
A non-dropping init frame (owned in the existing runtime module, not a new
framework) must be prepared **before** the call. On success, ownership transfers
to the ordinary runtime; on false, retain without CEF destruction until the
process naturally ends. This is intentional failure-only retention, **not**
clean CEF shutdown, leak closure or a claim that releasing refs is harmless.
The generated `cef::initialize` wrapper at binding lines58325–58358 clones/
converts Settings and adds an App reference **inside** the call. Keeping outer
welding locals cannot retain these temporaries; a guard around the Result is
too late. DRAFT (b) therefore explicitly proposes bypassing only this generated
high-level wrapper: call existing `cef::sys::cef_initialize` (cef-dll-sys reexport)
inside owned `vendor/welding/src/runtime.rs` native code, with equivalent raw
arguments/callback add-ref and conversion ownership prepared in the non-dropping
frame **before** the call. No new Cargo dependency, generated binding edit,
CEF binary fork, new transport or app unsafe is included. If the pinned sys
reexport/callee cannot support this exact seam, stop with that precise blocker.
Raw settings strings, app/ref-count callbacks, raw argv buffers, sandbox info
and dynamic loader owners all stay valid and retained on false. Success must
balance transfers/refcounts using the actual ABI, not guess wrapper equivalence.
The direct sys function's own loader/callee return and platform destructors
remain in the zero-post-false-call proof gate. A bypass proposal is not proof.

**Precise unanswered conformance gate:** can the existing owned runtime seam,
with the proposed direct-sys/non-dropping call-input frame, return through the
immediate sys callee/loader and each
platform's AppCreator/normal-return teardown after native false with **zero**
post-false CEF entrypoints/ref releases/unload callbacks, while no App/document/
control/autostart has existed? The ordering and eframe branch are source-backed;
the complete destructor/loader/platform return contract is **not established**.
If it cannot be established within that seam, report this exact blocker and
leave startup cancellation unimplemented/unaccepted. No new process architecture,
force-exit exemption or guessed ref-release safety follows.

**Completed pinned source receipt (B3; no more extraction requests):**
Evidence root is
`/home/hyjing/Code/intgral/intcad/worktrees/cef-osr/.superpowers/sdd/2026-11-18-cef-wayland-dsf/builder-evidence/`.
INDEX's final CEF/eframe addendum records all three locked archive checksums,
full-source archive equality, and the 23 new excerpt hashes. Names below use
its `package__source_path__Lstart-end.txt` convention; line numbers are original
source, not the excerpt's renumbered display.

| Pinned source / excerpt | Established contract, not native proof |
|---|---|
| cef154.5.0 binding L58324–58358, L554–674; string L239–284/L473–559; rc L219–308 and App L33665–33712/L33880–33911 | Wrapper clones/converts Settings and add-refs App; UTF16 Clear Drop calls native clear, RefGuard Drop calls release. Conversion/storage lifetime cannot be inferred from holding only outer settings. |
| cef `src/lib.rs` L1–33; cef-dll-sys binding L30478–30494 | `pub use cef_dll_sys as sys`; existing direct `cef_initialize` declaration, main-thread requirement and false-return restriction (“no further calls” except optional exit-code query). Reexport exists, but raw init/callee/platform teardown safety is not established. |
| cef-dll-sys binding L25373–25398/L27281–27296 | Async creation is nonblocking/UI-thread actual creation; sync creation UI-thread-only; on_after_created says it is now safe to act on browser. No general create-return/browse ordering guarantee; use inert sequencing, not an inference. |
| eframe0.36.2 `epi.rs` L46–51; `wgpu_integration.rs` L208–387 | AppCreator is fallible; map_err(AppCreation)? occurs before self.running insertion. Error locals follow Rust Drop semantics; painter/window/integration native cleanup is not certified. |
| eframe `run.rs` L89–185/L386–407/L450–471, `wgpu_integration.rs` L417–429/L583–597 | Error stored and event loop exited in run-and-return, returned as app.return_result; run-and-exit can process::exit and is forbidden here. Exiting/save_and_destroy may be invoked, but running cleanup/on_exit only runs for Some(running), absent on AppCreator error. No claim of zero destructors or native clean teardown. |

Missing-source Q2/Q6 access gaps are closed by these copies. Native immediate
sys callee/loader behavior, all-platform destructor/ABI conformance, actual
controlled-false return and durable bookkeeping/multi-instance fault tests stay
BLOCKED/NOT_RUN. Do not start another callee graph, invent an exit exemption or
present retained native inputs as clean shutdown.

**Exact proposed user decision (b):**
> Approve cancelling this new Intcad launch by its normal error/return path if
> native CEF initialization returns false, with initialization moved before any
> user/recovery design is opened or modified, including the narrow direct
> existing cef::sys::cef_initialize call inside owned welding native runtime
> code, retained failure-path call inputs/loader owners, and launch-owned
> non-CEF journal/startup-ticket cancellation that preserves genuine failure
> history without fake crashes or CPU/safe-mode escalation, with no
> generated-binding/CEF-binary fork, forced kill or action against another
> launch, subject to zero-post-false CEF-call and durable bookkeeping proof before
> enablement, while pre-init refusal may still start CAD without Assistant and
> approved responsive close-timeout quarantine remains only for initialized runtimes.

This decision is additional to the accepted close-timeout recommendation and
needs human approval plus resumed independent builder review. No native failure
has been executed and no startup behavior/default is changed by these documents.

### Shared quit barrier and programmatic behavior

Add a small host-close status to `SolveApp`, default Ready for hosts without CEF.
The desktop host publishes Required/Closing/Quarantined/Ready. UI quit keeps the
existing save-confirmation workflow; after documents permit quit, host close must
complete before viewport Close. Intercept direct OS Close every time the barrier
is not Ready, even if `quit_requested` was already true.
Stage Don't Save decisions until actual host-close commit: `quit_step` currently
marks those sessions saved before the host can cancel quit. Move that existing
finalization into a host-called `documents::finish_quit`, keeping document
content/dirty/recovery state intact on close timeout. Actual saves already made
remain saved; do not roll them back. No save/close dialog is run while quarantined.

Use the existing `docs.quitting: Option<Vec<usize>>` as the sole accumulating
Don't Save list during prompts, not a competing copy. When all prompts finish,
**move** that vector to `pending_quit_discard`, retain `docs.quitting=Some(vec![])`
as the existing workflow/tab-mutation guard, clear closing/saving, and set quit
intent. Programmatic accepted quit takes the same staged form; force stages
dirty tab indices without marking them saved. No prompts are revisited while
staged. All tab mutations (including control/open-completion paths) must honor
the existing quitting guard; no index shift or document replacement is permitted
between staging and commit. Host Ready with current intent calls `finish_quit`
only after the R1 snapshot validation below, marks only staged sessions saved,
updates reopen preferences, and clears
both vectors/closing/saving before final Close. Hosts already Ready use the same
commit in that turn. Timeout/Cancel/failed or cancelled Save clears both vectors,
closing/saving and quit intent, but never marks a staged session saved or erases
recovery. Real saves remain real. Late host completion does not resurrect intent.
Thus `ui.documents.quitting` stays true during prompting/staged close, false
after cancel/commit; closing remains the actual prompt tab/null, never a stale
host phase. Cancel during prompts also discards accumulated Don't Save choices.

### R1 — quit approval belongs to session state, not just tab indices

`integration-builder-check-3.md` identifies a source-traced data-loss gap in the
index-only staging above. This revision closes that **design** gap, not an
implemented/native proof. No new human decision or change to either DRAFT scope.

**Actual seams at c355edb97:** engine `Session` has per-open `uid`508,
`document_id`536, general refresh `revision`527, private `saved`523 and
`saved_visibility`641. `is_dirty:984–1001` compares document/visibility state;
`mark_saved:1011–1015` blesses the **current** document/visibility. Public
`document_stamp` is not sufficient: `cmd/inspect.rs:351–365` explicitly permits
`design_revision` reuse after edit→undo ABA. Undo/redo's `cmd/edit.rs:149–162,
189–213` restores document Arcs and refreshes revision; `refresh`1292–1299,
`refresh_display`1558–1561 and `accept_computed`2209–2234 advance the general
revision. `replace_design`907–927 replaces identity; view invalidation2189–2191
also advances revision. These are not renderer-commit epochs.

`SolveApp::run`1235–1303 executes existing-session commands without a quitting
guard; control's immediate/batch and `control_job::{finish_control_ahead,
poll_control_jobs}` re-enter that path. Other owner-thread adoption paths include
`dialogs::adopt_dialog`613–625 (`accept_commands`), `file_load::tick`1058–1083
(`accept_computed`/`documents::adopt`), `save_job::tick`49–70 (`adopt_save`),
deferred/feature-edit work, and linked/edit-in-place refresh. `adopt_save`
(`engine/cmd/file.rs:633–642`) correctly saves the worker's snapshot, not newer
edits. Preserve that behavior. `SolveApp::logic`2986–3033 polls file/save/
edit-in-place/control work; desktop main currently sends Close274–275 before
the later `App::ui`/content UI. A validation early in logic alone is too early.

**Minimal proposed types/accessors, in existing modules, not a new framework:**

```rust
// engine/src/lib.rs; opaque to UI, no native types, not serialized
pub struct QuitSessionSnapshot {
    uid: u64,
    document_id: String,
    replacements: u64,
    revision: u64, // Session::revision, NEVER design_revision()
    doc: Arc<Document>,
    saved: Arc<Document>,
    visibility: Visibility,
    saved_visibility: Visibility,
    pending_moves: std::collections::BTreeMap<u64, solidcraft_doc::Mat>,
    exploded: std::collections::BTreeMap<u64, solidcraft_doc::Mat>,
    active_sketch: Option<u64>,
    path: Option<String>,
    disk: Option<solidcraft_io::design_lock::FileStamp>,
    read_only: bool,
}
impl Session {
    pub fn quit_snapshot(&self) -> Result<QuitSessionSnapshot>; // engine Result
    pub fn matches_quit_snapshot(&self, expected: &QuitSessionSnapshot) -> bool;
}
// ui-egui/documents.rs; private fields; Vec size <= existing MAX_OPEN (32)
enum QuitDecision { Pending, KeepSaved, Discard }
struct QuitEntry {
    tab: usize,
    snapshot: solidcraft_engine::QuitSessionSnapshot,
    decision: QuitDecision,
}
enum QuitStage { Prompting, Staged, Committed }
struct QuitIntent { entries: Vec<QuitEntry>, stage: QuitStage }
// Documents::quit_intent: Option<QuitIntent>, initially None
pub struct QuitCommit { _private: () } // non-Clone; construct only on commit
pub enum QuitIntentError {
    Changed, PendingWork, Missing, NotReady, AlreadyCommitted, Allocation,
}
pub fn validate_quit_intent(app: &mut SolveApp) -> Result<(), QuitIntentError>;
pub fn finish_quit(app: &mut SolveApp) -> Result<QuitCommit, QuitIntentError>;
pub fn quit_committed(app: &SolveApp) -> bool;
// SolveApp/lib.rs + deferred.rs; inspect existing fields, never poll/consume work
impl SolveApp {
    pub(crate) fn quit_has_pending_design_work(&self) -> bool;
}
impl crate::deferred::DocJobs {
    pub(crate) fn blocks_quit(&self) -> bool;
}
```

`quit_snapshot` copies the listed fields, retains the two real document Arcs
(no deep Document clone), and clones existing visibility/move state.
`matches_quit_snapshot` compares every scalar/value field exactly and both Arcs
with `Arc::ptr_eq`; retained strong refs prevent pointer-allocation reuse/COW
in-place edits. Revision inequality rejects undo/edit/redo ABA even when content,
dirty flag and history lengths return to their old values. Never use only
tab/index, undo length, `is_dirty`, content hash or `document_stamp`. General
revision/view-only changes may conservatively cancel; do not weaken the check
by inventing a semantic/renderer epoch. Capture/check refuses a saturated
revision/replacement/identity counter rather than treating wrap/reuse as valid.
No new engine-wide mutation counter, save format or command schema.

1. **Capture before consent can age.** UI request creates bounded entries for
   all `open_sessions`, in tab order: initially clean entries are KeepSaved;
   dirty entries are Pending. A real Don't Save answer captures that tab's
   **then-current** snapshot and sets Discard as it appends the existing
   quitting index. Successful prompted Save may update only its Pending entry
   to KeepSaved, after real save adoption and `is_dirty=false`; failure/cancel
   or newer unsaved state cancels quit. Previously approved entries are never
   silently recaptured while later prompts run. Validate approved entries at
   each quit_step; after all answers, require all entries approved/unchanged,
   then move discard indices and set Staged. Programmatic clean/nonforce or
   force captures all sessions at acceptance, respectively KeepSaved or Discard
   for dirty ones; force grants no future-edit authority.
2. **Revalidate the whole intent, not just discarded tabs.** Require the same
   session count, tab→uid/document_id mapping and matching per-session snapshot
   for every approved entry (including initially clean sessions). This uses
   `documents::open_sessions:757–762`, not only the active session. Before
   committing, also require no Pending decision and no outstanding owner work
   that can change/save/replace a session. `quit_has_pending_design_work`
   conservatively includes active applying/deferred/deferred_queue/
   start_after_roll/delete_check/pending_feature_edit/editing_state, file load/picker/save,
   control_job/control_held or agent execution, and edit-in-place refreshing;
   `DocJobs::blocks_quit` tests its four fields at deferred.rs70–75, and the
   document owner checks inactive slots' jobs/pending feature/editor work too.
   No background snapshot worker mutates live sessions; acceptance is on the
   owner thread. Read-only completed answers/cache warming alone cannot grant
   or refresh quit consent.
3. **Edits/completions are not suppressed while Closing.** Existing commands,
   undo/redo and genuine saves may proceed normally; after owner-thread execution/
   adoption, if quit_intent exists run validate_quit_intent and cancel if an
   approved entry changed.
   A failed/no-op command with unchanged snapshots need not cancel. A delayed
   completion applies under its existing admission checks, then invalidates
   stale quit; never drop it or mark its newer design saved to preserve quit.
   Pending work found at final validation cancels quit rather than waiting/
   discarding the job. Replacement/open/close/tab-discard operations retain the
   existing document guard: refuse while quitting, or explicitly cancel_quit
   **before** their own ordinary operation; they never inherit discard consent.
   Direct existing-session changes bypassing run are still caught by the final
   per-session snapshot check. Explicit non-prompt Save/adopt_save changes the
   saved/path snapshot and cancels stale intent; it does not reauthorize quit.
4. **One synchronous final commit.** `finish_quit` must validate Ready, Staged,
   every snapshot and pending-work absence immediately **before the first**
   quit-induced mark_saved/reopen-preference/recovery-discard effect, checking
   all sessions before touching any. No await, worker polling, command, tab
   switch, UI callback or partial per-tab commit between validation and effects.
   Mark only approved Discard snapshots saved; KeepSaved entries must still be
   unchanged. Change stage to Committed and return one non-Clone QuitCommit;
   clear the existing quitting/discard/closing/saving fields as specified.
   The desktop uses that receipt for the single final viewport Close, in the
   same owner-thread section **after all app logic, control/adoption, UI and
   native-menu/input work**; then returns without further mutable app work.
   Move the current main274–275 Close out of early logic. Hosts already Ready
   obey the same last-action check; they cannot bypass it with quit_requested.
   The concrete desktop seam is the tail of `App::ui` (currently main293–325),
   after content_ui, Assistant tick and native-menu work, not a new eframe hook.
   Direct OS Close is always intercepted with CancelClose until this commit,
   including Ready+staged intent; a second OS request is not an alternate
   snapshot-free commit. Only the terminal Committed path may allow final close.
5. **No check-to-close gap.** Retain Committed only as a terminal close latch,
   not a fresh intent: no further command/adoption/UI mutation is run between
   commit/Close and teardown. App logic/UI, SolveApp::run, control dispatch
   and the named completion/adoption entries check quit_committed before
   executing/polling/applying mutating work; queued callbacks only
   queue inert data, never apply it. Repeated finish/Close attempts return
   AlreadyCommitted/no second receipt or mark_saved. This narrow terminal latch
   is not a general write-lock framework during asynchronous Closing.
   On_exit's existing save_job::tick(true) at main340 cannot adopt a late save:
   commit requires saving=None and no owner can start one after the latch.
   Shutdown may cancel workers, not apply their results; recovery_ui::close
   runs only for the already validated committed state. An abnormal exit with
   no QuitCommit is not permitted to manufacture consent/mark_saved.
6. **Any mismatch cancels, never partially discards.** Changed/PendingWork on
   an uncommitted intent calls existing cancel_quit: clear quit_intent, both index vectors, closing/
   saving and quit_requested; emit CancelClose/non-modal status through the
   desktop owner, with no new programmatic prompt/response. Keep current
   document, saved baseline, real-save results, dirty/recovery/reopen data and
   pending work. No mark_saved, discard, preference commit or viewport Close.
   Native close may continue to Ready, but that receipt only changes host
   status; it cannot recreate snapshots, quit intent or auto-resume old quit.
   A fresh explicit quit must capture fresh consent. Existing query closing/
   quitting fields and exact hostClose.pending semantics remain unchanged.
   Missing is a side-effect-free no-intent result; NotReady leaves pending
   close pending, with no commit effects. AlreadyCommitted is side-effect-free
   and **retains** the terminal latch, never calls cancel_quit or resumes CAD.

`validate_quit_intent` validates only already approved entries while Prompting
(the current Pending target may still be edited/saved), and all entries while
Staged. Pending-work absence is the additional **finish_quit** check, not a ban
on the prompted Save itself. None/Missing grants no commit. A Committed latch
is not cleared by routine validation of its intentionally changed saved markers.
Initial capture/allocation failure aborts before installing a new intent;
a later prompted capture failure cancels that existing request before staging/
commit. Never manufacture a zero/default snapshot or partially discard.

Required tests in existing real engine/session and ui-egui documents/control/
save-job/computed-command test areas (all **planned/NOT_RUN**):
`clean_programmatic_quit_then_edit_cancels_before_close`,
`dont_save_then_edit_preserves_new_dirty_state`,
`delayed_command_completion_invalidates_quit_intent`,
`quit_snapshot_rejects_undo_redo_content_aba`,
`quit_snapshot_detects_save_replace_and_inactive_session_changes`,
`cancelled_quit_then_late_native_closed_never_resumes`,
`unchanged_quit_intent_commits_once_after_final_ui_work`,
`committed_quit_never_dispatches_another_mutation`.
Drive actual Session commands and existing Harness/completion channels, not
shadow Session/GPU implementations or fake native callbacks claimed as CEF.
Check saved baseline/dirty and recovery bytes, no partial commit on a later-tab
mismatch, both force and initially clean/nonforce paths, pending work not yet
applied, real saves retained, index/query compatibility and zero second receipt.
Native ordering/Close/teardown acceptance remains BLOCKED.

`control::dispatch`'s `app.quit` and `ui.documents action=quit` must not bypass the
barrier or open programmatic dialogs. Existing force means “skip unsaved
confirmation”, **not** “kill live CEF”. Use the existing control
`Outcome::Done` wrapper with **outer refusal**, not a nested result error:
`{"ok":false,"error":"quit refused: <reason>"}`. Reasons, in preflight order:
`assistant-close-incomplete` for quarantine, `document-operation-pending` for a
busy picker/file load/pending document-close workflow, then `unsaved-documents` for dirty
sessions without force. Both quit endpoints return that same refusal shape,
with no dialog, quit intent, document or host-close state mutation.

Add one noncolliding `hostClose` object to **successful result objects only**:
`{"state":"ready|required|closing|quarantined","pending":bool,
"blocked":null|"assistant-close-incomplete"}`.
`pending = matches!(host_close_state, Required | Closing)` **exactly**,
independent of `quit_requested` (including host cleanup with no quit intent).
Blocked describes Quarantined, never a
prompt. Endpoint contracts:

| Endpoint | Existing fields preserved | Additive / pending semantics |
|---|---|---|
| `app.quit` success | `quitting: bool`, `asking: bool` | `quitting = quit_requested && host state Ready`; `asking=false` for this no-dialog programmatic path; `hostClose` reports the barrier. Required immediately after request means quitting=false, state=required, pending=true; a later host logic turn changes state to closing. Ready means eligible quit intent, not proof the window already closed. |
| `ui.documents` success/query | `closing: Option<usize>` remains a document-tab index/null; `quitting` remains `docs.quitting.is_some()`; all other document fields unchanged | Add `hostClose` only. Dispatch consumes action=quit through the programmatic helper, then obtains the ordinary document query result; never replace closing with a bool or redefine quitting as host state. |
| `ui.inspect` success/query | Existing `quit_requested`, `quitting` (document workflow), dialog/status and other fields | Add `hostClose`; after asynchronous timeout it reports state=quarantined, pending=false, blocked=assistant-close-incomplete. No second response is invented for the original request. |

A subsequent quit during quarantine returns the outer refusal above; query
endpoints still succeed and expose status. Move the quit action handling out of
the `documents::control` value-returning mutation branch into control dispatch;
keep that helper's query/non-quit signature and existing closing-index callers
unchanged. No competing embedded refusal format. See plan W3 for precise
Outcome/helper tests and caller updates. No Agent/MCP business JSON changes.
Do not consume update-launch intent, start a replacement process or call
`device_lost::end` from App Drop while CEF ownership remains unresolved. Existing
CAD software-recovery selection is preserved when a safe restart becomes
possible; it is not permission to create an overlapping CEF runtime. A lost CAD
device cannot be promised to draw: preserve event-loop/control/document recovery
responsiveness and report rendering loss honestly, rather than claiming the
timeout policy repairs a failed GPU.

Retry belongs to this owner, not the render tick alone:

* Renderer terminated with live runtime/browser: revoke old document, cancel
  placement/input, then navigate to a fresh authorized bootstrap URL on that
  existing browser; reinitialize page/theme independently of frame admission.
* A recoverable page/load/admission failure uses the same runtime and fresh
  document path when reload is necessary, otherwise a bounded repaint.
* Initial construction returned no usable owner, partial init is uncertain, or
  close quarantined: consume Retry in the host's **None path** too, report
  restart-required accurately and do not create an overlapping runtime.
* Conversation/attachments are retained. No “clear failed” success without an
  actual navigation/readiness transition. No new backend or CPU Assistant fallback.

## 6. Input, Unicode, registration and cfg — F3/F6/F7/F9

Keep one physical→DIP conversion in welding. The app captures only gestures that
began in the admitted current page or **drawn, current** popup rectangle. Retain
held button and ctrl/shift/alt/meta flags in coalesced motion and wheel events.
Captured motion/releases continue outside page/popup until release; unrelated
outside presses still return focus to CAD. Preserve event order: coalesce only
adjacent motion with the same ticket/modifiers/buttons, never move motion across
a press/release/key/IME event. Cancel capture on placement/lifetime/theme
invalidation, blur or queue overflow; never replay into a new layout.

Convert egui `active_range_chars` from scalar indices to **UTF-16 code-unit**
offsets before `ime_set_composition`. Reject reversed/out-of-range ranges and
checked-conversion overflow. Encode non-BMP `KeyEventKind::Char` as its two CEF
UTF-16 units, not `as u16`. Preserve existing public scalar-character KeyEvent;
translation is at `cef_input::send_key`. Correct the surface range documentation.
Retain the existing 256 critical-event count and bound owned queued IME/paste
text as well: 400,000 UTF-8 bytes per string and 800,000 bytes total. Refusal
cancels the gesture/composition with visible failure, not truncation or replay.

Consume existing `poll_ime_composition` only under current document/placement/
input ticket; drain/cancel stale bounds on transitions. Vendor callbacks must
bound character count, use fallible/checked bounds union and handle poisoned
locks without panic. Union directly over at most 200,000 bounds (the UTF-16
upper bound for 100,000 scalar values), without a temporary unbounded Vec; reject
excess with no current candidate geometry. Returned physical local bounds become egui window points
by dividing once by current ppp and adding current placement origin in points;
set `PlatformOutput::ime`/`egui::output::IMEOutput` (egui 0.36's
`egui::IMEOutput { purpose: egui::IMEPurpose::Normal, rect, cursor_rect,
should_interrupt_composition: false }`) for egui-winit's existing candidate-area
path, alongside IMEAllowed. No OS screen origin addition. Validate bounds and
clear candidate geometry on blur/hide/reload/placement change. This source path
does not prove a real OS candidate window works.

For page **and popup**, make one object own registration plus optional gamma
copy texture. Cache key contains extent, source format and sampled/view format.
Switching BGRA↔RGBA or sRGB→plain→sRGB destroys the old registration once and
replaces/clears its copy target. Never keep `PageTarget.id` after freeing it;
metadata/placement cannot select a stale registration. Keep byte-preserving GPU
gamma copy, transparency and current source/import admission; no CPU readback.

Make pure `agent_chat::theme_bridge` available on wasm; restore native-only cfg
on `agent_chat::web`, including all callers and field references. Use identical
desktop cfg predicates for main's subprocess and interop calls. New native
factory/policy/runtime imports must not leak into wasm or unsupported native
targets. Do not remove state/theme from wasm just to hide a compile error.

## 7. Honest density, scope and artifact receipts — F10/F11/F12/F13

### Density and placement

Record independently sourced fields: requested physical placement origin/extent,
clip rectangle, **producer's actual rounded requested DIP** from `ViewMetrics`,
requested DSF, callback coded/visible/content/source geometry+validity, imported
extent/generation, document/lifetime and placement revision. Never reconstruct
requested DIP from the received allocation. Keep `round(P/s)` and
`ceil(DIP*s)` rules: 463×1122 placement at 1.25 requests 370×898 DIP and expects
463×1123 native pixels, clipped 1:1 to placement.

`GetScreenInfo`'s supplied DSF is a request, not the independently effective
Chromium getter. Pinned accelerated metadata has no effective-DSF field.
Represent independently observed effective native scale as optional with
observation provenance. Missing observation is **BLOCKED** density proof, never
filled with requested scale, CSS DPR, or a product-derived ratio. Observational
native proof, if later permitted, must bind artifact/PID/starttime/callback and
avoid inferior mutation; no native experiment is authorized by these documents.

Identify actual placement transitions `(document, placement_revision,
origin, extent, scale, clip)` including same-lifetime A→B→A, not alternating page
lifetimes. Emit every transition, including refused/pending placements, so
receipt deduplication cannot hide B. Scan all triples, not only the first three.
Drain old frames/input and keep themed backing as already specified. Late
same-geometry captures with newer counters remain causally ambiguous: report
freshness NOT_RUN/BLOCKED; a visible stale frame/click **FAILS** acceptance.
No static document nonce, local epoch, capture counter or BeginFrame label
upgrades that claim. A stronger native fence would be a separately reviewed
extension, not an invented ABI patch in this wave.

### Hook and consumer

Keep fixed opt-in app scope probe, not arbitrary script/URL input. Publish the
missing dialog/devtools/fullscreen rectangles in a **separate bounded action**
instead of overflowing the existing rects message. Source contract tests parse
the real producer and consumers; fixtures mirror it. Each scope text remains
≤480 JS units and ≤600 Rust characters, with a bounded oversize failure.

Input report must aggregate typed/copy output, paste/copy output, popup open and
selection, scope, process ownership and requested movement checks. A present
wrong result FAILS; unavailable required observation BLOCKED, optional unrequested
coverage NOT_RUN. Missing clipboard/popup evidence cannot exit 0. Never require a
composer field absent from the real control state. JS DevTools URL popup denial
is not proof of every native DevTools entrypoint; retain native request-boundary
tests. The test controls are not fabricated supported product features.

### Provenance and safe automation

`native-density.py::validate_manifest` must reject empty/duplicate-role/incomplete
inventories. Require app, launcher/wrapper if used, actually mapped CEF, CEF
subprocess executables and platform-mandatory resources. Linux CEF 154 requires
at least ICU data, resource/scale paks, locales, the selected V8 snapshot blob(s),
and any sandbox/GPU binaries required by the authenticated distribution. Inventory
the **whole** bundle, not only those minima. Bind origin archive/revision/checksum
record and all packaging/strip/RPATH transforms, effective policy and notices.
Unavailable authenticated origin is BLOCKED; a known mismatch is FAIL.

Live capture compares supplied app to `/proc/<pid>/exe` through stable
PID+starttime and file identity/whole hash; inventory child/zygote descendants
with their own starttimes/executables/runtime origin. Detect deleted/replaced
executables, outside-root resource/symlink escapes, stale loader paths, mismatched
subprocess helpers and PID reuse. Windows/macOS need their platform identity
receipts later; Linux `/proc` logic cannot certify them.

All tools use explicit task source/platform/name, private token directory and
PID+starttime. Close control sockets/files with context managers/finally.
No `/tmp/intcad-shot-token`, name-selected windows, `pkill`, host niri capture or
global clipboard helpers. Input uses app-control for app-level tests and approved
task VM control for OS input/IME. Real OS IME is BLOCKED where VM capability is
unavailable; do not substitute direct composition injection. Any xclip or helper
used inside an authorized isolated VM is a retained owned PID with bounded wait,
not a shell-background orphan. Cleanup cannot terminate another Intcad process.
Own-process termination in a disposable test VM must be recorded as harness
cleanup, never a product clean-close PASS; no new forced-exit product policy.

On every tool exception (including offscreen surface sampling), write the report
with accumulated statuses and run owned-resource finally cleanup. FAIL outranks
BLOCKED, which outranks NOT_RUN, which outranks PASS. Do not turn missing
measurement into a successful or empty inventory.

## 8. Minor disposition without default/scope creep

| Item from final verdict | Disposition in this wave |
|---|---|
| Task3 M1 duplicate state JSON | Defer performance optimization; retain semantics. Record no speed-up claim. |
| Task3 M2 origin-only B redoes theme ack | Retain conservative ticket reset; no optimization or old-ticket reuse. |
| Task3 M3 | Elevated F9; planned repair/tests for both surfaces, not executed or closed here. |
| Task3 M4 transient non-B repaint error | Pin recovery on later admitted frame; test no permanent error latch. Native visible recovery remains gated. |
| Task3 M5 origin move cancels focus/composition | Keep safe cancellation; explicitly exercise move→refocus→new composition, no stale replay. User-visible native acceptance outstanding. |
| Task3 M6 non-B theme timeout reliability | Retain existing behavior; add coverage/status for missing/late theme paint. No new non-B timeout/default in this wave; do not claim reliability proof. |
| Task4 offscreen exception/report cleanup | Repair report/finally in theme tool, including app launch/control failures. |
| T5 M1 theme provenance/order/count | Document theme JSON→Tokens→bridge→computed roles→pixels chain; enumerate current source set, not literal 25. Four-scale required coverage unchanged. |
| T5 M2 historical inline PASS/Cargo headings | Label historical lines inline in cutover/visual/readiness docs; never rewrite raw old evidence. |
| T5 M3 code row in Files pending | W8 removes **only** the `vendor/welding/` CODE row from `attribution/pending/cef-wayland-welding.md`; preserves `vendor/welding/LICENSE.txt` asset row and existing four-column header unchanged/unfolded. Code origin/licence/delta stay in existing vendor README/UPSTREAM/NOTICE inventory, including any approved HAL code through the existing README inventory. Licence texts alone may get four-column asset rows. This eliminates the known wrong Files fold, not accepts it. Canonical main ATTRIBUTION Vendored-table curation is explicitly DEFERRED release housekeeping, not closure here. No direct ATTRIBUTION edit/fold, pending README/format/parser/workflow change. |
| T5 M4 harness warning | Already disclosed; no app-compile-failure claim or unrelated warning suppression. |
| T5 M5 early failed paths / reruns | Retain all receipts and exact exit evidence; no new blocker or rerun of unchanged inputs. |

## 9. Verification, delivery and stop rules

The plan maps every F/minor to a work unit and test/evidence or explicit deferral.
All newly named tests
are **planned/NOT_RUN**. Pure Rust source-module tests and Python selftests may
establish only their actual bounded contracts. No fake `wgpu`, CEF, egui native
types or shadow application implementations to make a cheap harness compile.
Tests requiring actual dependency types stay in existing real module tests and
remain BLOCKED until permitted compilation. No per-bug test files.

Use installed Rust **1.99.1**, not the older dev-shell compiler; no installation,
Cargo/dependency rebuild or new shared target authorized here. Full Cargo,
Clippy, native FFI/link/ownership, wasm, Windows/macOS and all GUI/GPU/OS IME gates
remain BLOCKED under current resources. Current historical source counts
(24 unique Rust, 20 theme and 23 density selftests) do not validate this repair.

The corrected developer resource ruling is binding: no new heavy builds while
aggregate ≥180/200 GiB constraints apply; ≥150 GiB filesystem free still required.
Last recorded aggregate exceeded 200 GiB. No second cleanup, CI/public push,
new artifact service, provider/billing, system change or historical-binary reuse.
The aborted heavy attempt yielded **no artifact**. No binary delivery is promised.

When permitted, build a new fixed-source trusted artifact in an owned target,
record full bundle provenance, then VM first and same identified artifact host
last/manual approval. Required native coverage includes all current themes,
1/1.25/1.5/2, same-size and ABA placement changes, page/popup gamma, drag and
modifiers, real IME candidates, renderer recovery, partial init, timeout/late
close and device loss. Reconcile FD 399→504 growth and whole-file historical hash
mismatch with new evidence, never retroactive recertification.

MPL exact modified source location, package CEF staging/helper/sandbox closure,
generated CEF credits version/generator/hash and bundled LGPL authorization
remain delivery blockers. No new LGPL exception. Do not read prohibited code,
edit generated `THIRD-PARTY-LICENSES.txt`/credits by hand or fold attribution in a
PR. New diagnostics use locale pending files. No changed CAD/UI default,
preference migration, fallback browser or kernel change is authorized.

Rollback is a matched reviewed app/runtime/resource bundle with old authority
revoked; never restore unauthenticated IPC, hot-swap a live runtime, use another
adapter, XWayland, WebView or CPU Assistant rendering. If any material contract
cannot be met, return its exact source/evidence blocker to the controller.
Source review can conclude separately from native delivery, but neither an
implementer's self-review nor these documents independently approves the repair.
