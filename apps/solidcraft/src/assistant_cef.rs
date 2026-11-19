//! The Assistant's page as a CEF accelerated-OSR texture in the app's own window.
//!
//! Experimental replacement for the WRY web view (see `assistant_web.rs`): the page is
//! painted by CEF into a DMA-BUF, imported into the app's existing wgpu device, and drawn
//! with egui in the panel's rectangle. There is no child window and no GTK on Linux.
//!
//! Enabled with `SOLIDCRAFT_ASSISTANT_CEF=1` while the migration is under test. The
//! production path stays `assistant_web.rs` until that switch is removed in the cutover.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Context;
#[cfg(target_os = "linux")]
use welding::linux_cef::{LinuxCefConfig, LinuxCefProducer};
#[cfg(target_os = "macos")]
use welding::macos_cef::{MacosCefConfig, MacosCefProducer};
use welding::wgpu;
#[cfg(windows)]
use welding::windows_cef::{WindowsCefConfig, WindowsCefProducer};
use welding::{
    CefRuntime, CefRuntimeConfig, CefSandboxMode, CefSurfaceConfig, CefSurfaceEvent, CefSurfaceProducer, EventModifiers, FocusDirection,
    HostWgpuContext, ImportedTexture, KeyEvent, KeyEventKind, MouseAction, MouseButton, MouseEvent, RuntimeLease, WebRequestId,
};

use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::agent_chat::web;
use crate::assistant_cef_http::{BrowserOwnerId, DocumentAuthority, DocumentId, NavigationIdentity, PageAssets, PageServer, Pictures, ServerProgress};
use crate::assistant_cef_lifecycle::{CloseProgress, Lifecycle, Phase};

#[path = "assistant_cef_frame.rs"]
mod admission;
#[path = "assistant_cef_config.rs"]
mod config;
#[path = "assistant_cef_viewport.rs"]
mod viewport;

use admission::{FrameAdmission, FrameReject, InputTicket, Placement, ThemeTicket};
pub use config::HostBackend;
use config::Switch;
use welding::frame_geometry::{CaptureMetadata, FrameRect, FrameWatermarks};

/// The actual window-system connection, not the desktop session's environment.
pub fn host_backend(cc: &eframe::CreationContext<'_>) -> HostBackend {
    use welding::wgpu::rwh::{HasDisplayHandle, RawDisplayHandle};
    match cc.display_handle().map(|display| display.as_raw()) {
        Ok(RawDisplayHandle::Wayland(_)) => HostBackend::LinuxWayland,
        Ok(RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)) => HostBackend::LinuxX11,
        Ok(RawDisplayHandle::Windows(_)) => HostBackend::Windows,
        Ok(RawDisplayHandle::AppKit(_)) => HostBackend::Macos,
        _ => HostBackend::Unknown,
    }
}

static WAYLAND_RUNTIME: std::sync::OnceLock<config::EffectivePolicy> = std::sync::OnceLock::new();

fn configure_private_runtime(runtime: &mut CefRuntimeConfig, policy: &config::EffectivePolicy) {
    // Keep the B policy's predicate authoritative there; every other private
    // HTTP host must suppress inherited browser argv as well.
    runtime.command_line_args_disabled = policy.command_line_args_disabled() || policy.backend != HostBackend::LinuxWayland;
    runtime.log_severity = welding::CefLogSeverity::Disable;
}

/// The page's origin while CEF runs: a loopback server this module owns, because CEF has no
/// custom-scheme handler here and the page's ES modules need a real origin.
static FAILED_START: AtomicBool = AtomicBool::new(false);
thread_local! {
    // Failure-only retention, never a clean shutdown receipt. W3 owns explicit
    // native completion/shutdown; TLS teardown must not drop live CEF inputs.
    static FAILED_OWNERS: std::cell::RefCell<Option<std::mem::ManuallyDrop<FailedOwners>>> = const { std::cell::RefCell::new(None) };
}
struct FailedOwners {
    _runtime: CefRuntime,
    _server: Option<PageServer>,
    _host: Option<HostWgpuContext>,
}
fn retain_failed_start(runtime: CefRuntime, mut server: Option<PageServer>, host: Option<HostWgpuContext>) {
    FAILED_START.store(true, Ordering::Release);
    if let Some(server) = &mut server { server.request_stop(); }
    let owners = std::mem::ManuallyDrop::new(FailedOwners { _runtime: runtime, _server: server, _host: host });
    FAILED_OWNERS.with(|slot| {
        if let Ok(mut slot) = slot.try_borrow_mut() {
            if slot.is_none() { *slot = Some(owners); }
            // A second bundle also remains non-dropping; never replace/release
            // an older owner if an impossible re-entrant failure is observed.
        }
    });
}

thread_local! {
    // A quarantined bundle never runs a native destructor at TLS teardown. The
    // runtime itself also refuses to shut CEF down without observed completion.
    static QUARANTINED: std::cell::RefCell<Option<std::mem::ManuallyDrop<QuarantinedOwners>>> = const { std::cell::RefCell::new(None) };
}
struct QuarantinedOwners {
    _runtime: CefRuntime,
    _producer: Option<Producer>,
}
fn quarantine(runtime: CefRuntime, producer: Option<Producer>) {
    let owners = std::mem::ManuallyDrop::new(QuarantinedOwners { _runtime: runtime, _producer: producer });
    QUARANTINED.with(|slot| match slot.try_borrow_mut() {
        Ok(mut slot) if slot.is_none() => *slot = Some(owners),
        _ => {
            // A second bundle cannot replace the first: retain it without a
            // native destructor rather than releasing owners into CEF.
            log::warn!("CEF quarantine already holds a bundle; retaining this one without native teardown");
        }
    });
}

fn private_request_policy(
    authority: Arc<DocumentAuthority>, owner: BrowserOwnerId, renewal: Arc<AtomicBool>,
) -> welding::request_policy::RequestPolicy {
    use welding::request_policy::{RequestCheck, RequestDecision};
    Arc::new(move |check| match check {
        RequestCheck::BrowserCreated { browser_id } => {
            if authority.bind_browser(owner, browser_id).is_ok() { RequestDecision::Allow } else { RequestDecision::Cancel }
        }
        RequestCheck::BeforeBrowse { browser_id, main_frame, target_url, is_redirect } => {
            if authority.check_inert_navigation(owner, browser_id, target_url, main_frame, is_redirect).is_ok() {
                return RequestDecision::Allow;
            }
            let result = authority.navigation_identity(owner, browser_id, target_url)
                .and_then(|identity| authority.claim_main_navigation(&identity, main_frame, is_redirect));
            match result {
                Ok(_) => RequestDecision::Allow,
                Err(crate::assistant_cef_http::TransportError::PermitConsumed) if main_frame && !is_redirect => {
                    renewal.store(true, Ordering::Release);
                    RequestDecision::RenewDocument
                }
                _ => RequestDecision::Cancel,
            }
        }
        RequestCheck::Resource { browser_id, target_url, is_image } |
        RequestCheck::Redirect { browser_id, target_url, is_image } => {
            let authorized = authority.check_resource(owner, browser_id, target_url, is_image).is_ok();
            let asset = (is_image && (target_url.starts_with("data:image/") || target_url.starts_with("blob:")))
                || target_url.contains("/picture/")
                || target_url.rsplit('/').next().is_some_and(|name| {
                    matches!(name, "ipc" | "ipc.js" | "intcad-plex.ttf") || crate::assistant_page::page_file(name).is_some()
                });
            if authorized && asset { RequestDecision::Allow } else { RequestDecision::Cancel }
        }
    })
}

/// CEF re-executes this binary for its renderer, GPU and utility processes. Must run before
/// any thread is started (winit, wgpu, the app's pools).
pub fn subprocess_entry() -> Option<i32> {
    let cef_path = std::env::var("CEF_PATH").ok()?;
    match CefRuntime::execute_process_from(std::path::Path::new(&cef_path), sandbox()) {
        Ok(code) => code,
        Err(error) => {
            note(&format!("CEF subprocess probe failed: {error}"));
            None
        }
    }
}

fn sandbox() -> CefSandboxMode {
    // The sandbox is the production setting; the unsandboxed mode is only for a run that asks
    // for it explicitly while the migration is being brought up.
    //
    // Windows is the exception that packaging has to close: CEF's Windows sandbox runs its
    // subprocesses through `bootstrap.exe`, which creates the sandbox context and calls an
    // exported entry point in the app; `execute_process_from` cannot manufacture that context
    // and refuses `Sandboxed` there. Until the MSI ships that shape, a Windows run reports the
    // page as unavailable rather than starting Chromium unsandboxed.
    match std::env::var("SOLIDCRAFT_ASSISTANT_CEF_SANDBOX").ok().as_deref() {
        Some("off") => CefSandboxMode::UnsandboxedTrustedContent,
        _ => CefSandboxMode::Sandboxed,
    }
}

/// Preserve the full selected CAD descriptor; add only supported interop.
#[cfg(any(target_os = "linux", test))]
fn interop_descriptor(
    base: &wgpu::DeviceDescriptor<'static>, backend: wgpu::Backend, supported: wgpu::Features,
) -> Option<wgpu::DeviceDescriptor<'static>> {
    if backend != wgpu::Backend::Vulkan || !supported.contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF) {
        return None;
    }
    let mut descriptor = base.clone();
    descriptor.required_features |= wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
    Some(descriptor)
}

/// Source repair is not native cleanup/ABI proof. No runtime/env switch may
/// bypass this gate. W2's cleanup audit records the still-unexecuted cases.
#[cfg(target_os = "linux")]
const NATIVE_INTEROP_CLEANUP_VERIFIED: bool = false;

/// Later policy decision on the device CAD already created. It reads only the
/// announced features: it never selects an adapter, opens a device or mutates
/// the retained CAD setup. False leaves the selected CAD device untouched.
#[cfg(target_os = "linux")]
fn interop_device_admissible(features: wgpu::Features) -> bool {
    NATIVE_INTEROP_CLEANUP_VERIFIED
        && features.contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
}

/// Install only on CAD's existing setup, after all its descriptor/adapter policy.
/// Coarse eligibility asserts no display backend and initializes no CEF runtime.
pub fn configure_interop(setup: &mut eframe::egui_wgpu::WgpuSetupCreateNew, eligible: bool) -> Result<(), String> {
    if !eligible { return Ok(()); }
    if setup.native_device_creator.is_some() {
        return Err("CEF interop refused: a native device factory is already installed".into());
    }
    #[cfg(target_os = "linux")]
    {
        setup.native_device_creator = Some(Arc::new(|adapter, base| {
            let Some(descriptor) = interop_descriptor(base, adapter.get_info().backend, adapter.features()) else {
                emit_accelerated_limit(false, "selected CAD adapter does not support Vulkan DMA-BUF");
                return Ok(None);
            };
            if !NATIVE_INTEROP_CLEANUP_VERIFIED {
                emit_accelerated_limit(false, "native interop cleanup and ABI validation blocked");
                return Ok(None);
            }
            // This call remains disabled until every failure path has native
            // proof of release before returning to ordinary CAD request_device.
            welding::build_dmabuf_capable_device(adapter, &descriptor)
                .map(Some)
                .map_err(|_| "CEF interop device creation refused".into())
        }));
    }
    Ok(())
}

/// The platform's producer: every platform paints into its own kind of texture, and
/// `WgpuTextureImporter` imports that kind into the device the app already has.
#[cfg(target_os = "linux")]
type Producer = LinuxCefProducer;
#[cfg(windows)]
type Producer = WindowsCefProducer;
#[cfg(target_os = "macos")]
type Producer = MacosCefProducer;

/// Create the browser on this platform. Windows needs the host device at creation to make its
/// D3D11 copy of CEF's shared texture; Linux and macOS take it at import time.
fn create_producer(runtime: &CefRuntime, surface: CefSurfaceConfig, host: &HostWgpuContext) -> Result<Producer, welding::WeldError> {
    #[cfg(target_os = "linux")]
    let _ = host;
    #[cfg(target_os = "linux")]
    {
        LinuxCefProducer::new(runtime, LinuxCefConfig { surface })
    }
    #[cfg(windows)]
    {
        WindowsCefProducer::new(runtime, WindowsCefConfig { surface }, host)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = host;
        MacosCefProducer::new(runtime, MacosCefConfig { surface })
    }
}

/// Bounded input queue: CEF drops input until the browser has painted its first frame.
struct InputQueue {
    critical: VecDeque<Queued>,
    motion: Option<(i32, i32)>,
    capacity: usize,
    ready: bool,
    ticket: Option<InputTicket>,
    buttons: Vec<MouseButton>,
    keys: Vec<i32>,
    overflowed: bool,
    pub held: u64,
    pub refused: u64,
}

enum Queued {
    Mouse(MouseEvent),
    Key(KeyEvent),
    ImeSet(String, (u32, u32)),
    ImeCommit(String),
    ImeCancel,
}

impl InputQueue {
    fn new(capacity: usize) -> Self {
        Self {
            critical: VecDeque::new(),
            motion: None,
            capacity,
            ready: false,
            ticket: None,
            buttons: Vec::new(),
            keys: Vec::new(),
            overflowed: false,
            held: 0,
            refused: 0,
        }
    }

    fn push(&mut self, event: Queued) {
        if let Queued::Mouse(MouseEvent { action: MouseAction::Moved, x, y, .. }) = event {
            self.motion = Some((x, y));
            if !self.ready {
                self.held = self.held.saturating_add(1);
            }
            return;
        }
        if self.critical.len() >= self.capacity {
            self.refused = self.refused.saturating_add(1);
            self.overflowed = true;
            return;
        }
        // Releases whose press belonged to an invalidated placement must not
        // become a click/key-up in the newly admitted page.
        match &event {
            Queued::Mouse(mouse) => match mouse.action {
                MouseAction::Pressed if !self.buttons.contains(&mouse.button) => self.buttons.push(mouse.button),
                MouseAction::Released => {
                    if !self.buttons.contains(&mouse.button) {
                        return;
                    }
                    self.buttons.retain(|button| *button != mouse.button);
                }
                _ => {}
            },
            Queued::Key(key) => match key.kind {
                KeyEventKind::RawKeyDown | KeyEventKind::KeyDown if !self.keys.contains(&key.windows_key_code) => {
                    if self.keys.len() >= self.capacity {
                        self.refused = self.refused.saturating_add(1);
                        self.overflowed = true;
                        return;
                    }
                    self.keys.push(key.windows_key_code);
                }
                KeyEventKind::KeyUp => {
                    if !self.keys.contains(&key.windows_key_code) {
                        return;
                    }
                    self.keys.retain(|code| *code != key.windows_key_code);
                }
                _ => {}
            },
            _ => {}
        }
        if !self.ready {
            self.held = self.held.saturating_add(1);
        }
        self.critical.push_back(event);
    }

    fn clear(&mut self) {
        self.critical.clear();
        self.motion = None;
        self.buttons.clear();
        self.keys.clear();
        self.overflowed = false;
    }

    fn invalidate(&mut self) {
        self.clear();
        self.ready = false;
        self.ticket = None;
    }
}

pub struct CefAssistant {
    runtime: CefRuntime,
    /// The app's live claim on the runtime; released only before checked shutdown.
    lease: RuntimeLease,
    /// Pure transition/ownership policy; native completion stays platform-owned.
    lifecycle: Lifecycle,
    producer: Producer,
    host: HostWgpuContext,
    queue: InputQueue,
    texture: Option<(egui::TextureId, [u32; 2])>,
    /// The page's own texture: same bytes, a format egui can take as gamma values.
    page: Option<PageTarget>,
    popup: Option<(egui::TextureId, [i32; 2], [u32; 2])>,
    popup_page: Option<PageTarget>,
    told: String,
    told_theme: String,
    focused: bool,
    shown: bool,
    viewport: viewport::ViewportState,
    /// Only B (actual native Linux Wayland) requires owned capture metadata.
    admission: Option<FrameAdmission>,
    placement: Option<Placement>,
    lifetime: u64,
    theme_revision: u64,
    pending_theme: Option<ThemeTicket>,
    requested_theme: String,
    page_ready: bool,
    native_ready: bool,
    last_rect: Option<egui::Rect>,
    placed_this_frame: bool,
    popup_current: bool,
    popup_generation: u64,
    input_quarantined: bool,
    input_failed: bool,
    identity_exhausted: bool,
    last_reject: Option<FrameReject>,
    pictures: Pictures,
    server: PageServer,
    authority: Arc<DocumentAuthority>,
    browser_owner: BrowserOwnerId,
    document: Option<DocumentId>,
    renewal: Arc<AtomicBool>,
    deferred_theme_ack: Option<ThemeTicket>,
    copy_request: Option<(u64, DocumentId)>,
    script_serial: u64,
    transport_probe: Option<(u64, DocumentId)>,
    probe_due: Instant,
    last_frame: Instant,
    failed: bool,
    osr_refused: bool,
    ticks: u64,
    pushes: u64,
    frames: u64,
    last_stats: Instant,
    /// Test-only: a native `<select>` was added to the real page to exercise the popup path.
    test_select: bool,
    /// Test-only: the fixed scope probe was installed once for this page lifetime.
    scope_injected: bool,
    /// Last identified windowless browser, for popup-denial receipts.
    browser_id: i32,
    browser_windowless: bool,
    /// One explicit accelerated-import receipt per Assistant lifetime; absence is never success.
    accelerated_recorded: bool,
    /// One admitted receipt per view geometry/requested scale keeps logs bounded.
    last_view_receipt: Option<([i32; 2], u32)>,
    last_frame_reason: Option<String>,
    /// The window's input method is on because the page has the keyboard.
    ime_on: bool,
    started: Instant,
    test_reloaded: bool,
}

/// The page as egui reads it.
///
/// CEF's frame is a GPU texture in an sRGB format, and egui's shader treats a sampled texture
/// as gamma-encoded: it multiplies by the vertex colour and converts to linear itself before
/// the sRGB surface encodes on write. Sampling an sRGB view decodes twice and the page comes out
/// darker than its own CSS. WebGPU will not let a view pick the non-sRGB format unless the
/// texture was created with it in `view_formats`, and CEF's texture is not ours to create, so
/// the frame is copied on the GPU once per painted frame into this texture and read through a
/// non-sRGB view. No readback, no CPU work, and the page's bytes reach egui unchanged.
struct PageTarget {
    id: egui::TextureId,
    texture: wgpu::Texture,
    size: [u32; 2],
}

impl CefAssistant {
    /// Make the browser and its page. `None` when CEF cannot start: the panel then says why
    /// instead of drawing a second conversation.
    pub fn new(render_state: &eframe::egui_wgpu::RenderState, backend: HostBackend) -> Option<Self> {
        if FAILED_START.load(Ordering::Acquire) { return None; }
        #[cfg(target_os = "linux")]
        if !interop_device_admissible(render_state.device.features()) {
            emit_accelerated_limit(false, "selected CAD device interop is unavailable or native validation is blocked");
            return None;
        }
        let cef_path = std::env::var("CEF_PATH").ok()?;
        let wayland_osr = backend == HostBackend::LinuxWayland;
        #[cfg(not(target_os = "linux"))]
        let identity = None;
        #[cfg(target_os = "linux")]
        let identity = if wayland_osr {
            match CefRuntime::inspect_identity(std::path::Path::new(&cef_path)) {
                Ok(id) => Some(config::RuntimeIdentity {
                    cef: id.cef,
                    chromium: id.chromium,
                    api_version: id.api_version,
                    platform_api_hash: id.platform_api_hash,
                    cef_commit: id.cef_commit,
                }),
                Err(_) => {
                    note("CEF: Wayland runtime identity unavailable; Assistant refused");
                    return None;
                }
            }
        } else {
            None
        };
        let raw = std::env::var("SOLIDCRAFT_ASSISTANT_CEF_SWITCHES").unwrap_or_default();
        let overrides = if wayland_osr {
            match config::parse_overrides(&raw) {
                Ok(overrides) => overrides,
                Err(error) => {
                    note(&format!("CEF: Wayland override policy refused: {error:?}"));
                    return None;
                }
            }
        } else {
            // Preserve the old parser, including permissive legacy switch
            // spelling, on Windows/macOS and Linux X11.
            raw.split(',')
                .filter(|part| !part.is_empty())
                .map(|part| {
                    let (name, value) = match part.split_once('=') {
                        Some((name, value)) => (name, Some(value.to_owned())),
                        None => (part, None),
                    };
                    Switch { name: name.to_owned(), value }
                })
                .collect()
        };
        if config::validate_private_http_overrides(&overrides).is_err() {
            note("CEF: private-page diagnostic override refused");
            return None;
        }
        let policy = match config::build_policy(backend, true, identity.as_ref(), &overrides) {
            Ok(policy) => policy,
            Err(error) => {
                note(&format!("CEF: runtime policy refused: {error:?}"));
                return None;
            }
        };
        if wayland_osr {
            if let Err(error) = config::claim_runtime(&WAYLAND_RUNTIME, &policy) {
                note(&format!("CEF: runtime claim refused: {error:?}"));
                return None;
            }
            // A failed initialization keeps the claim: CEF may have partially
            // initialized global state. Never retry or shut down another owner.
        }
        emit_runtime_receipt(&policy);
        let mut runtime_config = CefRuntimeConfig::new(&cef_path, sandbox());
        runtime_config.cache_path = Some(crate::settings_dir().ok().flatten().unwrap_or_else(std::env::temp_dir).join("cef-assistant"));
        // The validated environment is not the only CEF ingress: ignore the
        // browser's inherited argv on every private HTTP host, then apply only policy.switches through
        // WeldApp's existing callback. Subprocess dispatch above is unchanged.
        configure_private_runtime(&mut runtime_config, &policy);
        runtime_config.command_line_switches = policy.switches.into_iter().map(|entry| (entry.name, entry.value)).collect();
        let mut lifecycle = Lifecycle::new();
        if lifecycle.begin_initialization().is_err() {
            note("CEF: lifecycle refused initialization");
            return None;
        }
        let runtime = match CefRuntime::initialize(runtime_config) {
            Ok(runtime) => runtime,
            Err(error) => {
                note(&format!("CEF initialize failed: {error}"));
                return None;
            }
        };
        let lease = runtime.lease();

        let host = HostWgpuContext::new(render_state.device.clone(), render_state.queue.clone());
        #[cfg(target_os = "linux")]
        if !host.device.features().contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF) {
            note("CEF: the app's device has no VULKAN_EXTERNAL_MEMORY_DMA_BUF; the page cannot be shown");
            emit_accelerated_limit(false, "app device has no VULKAN_EXTERNAL_MEMORY_DMA_BUF");
            retain_failed_start(runtime, None, Some(host));
            return None;
        }
        let pictures: Pictures = Arc::new(Mutex::new(HashMap::new()));
        let authority = Arc::new(DocumentAuthority::new());
        let browser_owner = match authority.reserve_browser_owner() {
            Ok(owner) => owner,
            Err(_) => {
                retain_failed_start(runtime, None, Some(host));
                return None;
            }
        };
        let mut server = match PageServer::start(
            PageAssets { resolve: crate::assistant_page::page_file,
                font: include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf") },
            pictures.clone(), authority.clone(),
        ) {
            Ok(server) => server,
            Err(_) => {
                note("CEF: private page server unavailable");
                retain_failed_start(runtime, None, Some(host));
                return None;
            }
        };
        if server.take_failure().is_some() || server.poll_stop() != ServerProgress::Running {
            retain_failed_start(runtime, Some(server), Some(host));
            return None;
        }
        let renewal = Arc::new(AtomicBool::new(false));
        let surface = CefSurfaceConfig {
            initial_url: "about:blank".into(),
            request_policy: Some(private_request_policy(authority.clone(), browser_owner, renewal.clone())),
            initial_size: dpi::PhysicalSize::new(900, 600),
            scale_factor: 1.0,
            prefer_accelerated: true,
            // The page's own CSS is see-through outside its rounded surface: CEF must paint
            // transparent there, or every transparent pixel becomes its default white and the
            // panel's corners show as white squares.
            background_color: None,
            ..Default::default()
        };
        // Creation is owned before the native request; the real browser id is
        // observed from BrowserCreated, not from a fabricated return value.
        if lifecycle.browser_requested().is_err() {
            note("CEF: lifecycle refused browser creation");
            retain_failed_start(runtime, Some(server), Some(host));
            return None;
        }
        let producer = match create_producer(&runtime, surface, &host) {
            Ok(producer) => producer,
            Err(_) => {
                note("CEF: the private page could not be created");
                retain_failed_start(runtime, Some(server), Some(host));
                return None;
            }
        };
        let max_edge = host.device.limits().max_texture_dimension_2d;
        Some(Self {
            runtime,
            lease,
            lifecycle,
            producer,
            host,
            queue: InputQueue::new(256),
            texture: None,
            page: None,
            popup: None,
            popup_page: None,
            told: String::new(),
            told_theme: String::new(),
            focused: false,
            shown: false,
            viewport: viewport::ViewportState::new(Instant::now()),
            admission: wayland_osr.then(|| FrameAdmission::new(max_edge)),
            placement: None,
            lifetime: 0,
            theme_revision: 0,
            pending_theme: None,
            requested_theme: String::new(),
            page_ready: false,
            native_ready: false,
            last_rect: None,
            placed_this_frame: false,
            popup_current: false,
            popup_generation: 0,
            input_quarantined: false,
            input_failed: false,
            identity_exhausted: false,
            last_reject: None,
            pictures,
            server,
            authority,
            browser_owner,
            document: None,
            renewal,
            deferred_theme_ack: None,
            copy_request: None,
            script_serial: 16,
            transport_probe: None,
            probe_due: Instant::now(),
            last_frame: Instant::now(),
            failed: false,
            osr_refused: false,
            ticks: 0,
            pushes: 0,
            frames: 0,
            last_stats: Instant::now(),
            test_select: false,
            scope_injected: false,
            browser_id: 0,
            browser_windowless: false,
            accelerated_recorded: false,
            last_view_receipt: None,
            last_frame_reason: None,
            ime_on: false,
            started: Instant::now(),
            test_reloaded: false,
        })
    }

    /// The page's messages and the app's state, whether or not the window is painted.
    pub fn logic(&mut self, solve: &mut SolveApp, ctx: &Context) {
        self.pump(ctx);
        if !self.failed && (self.document.is_none() || self.renewal.swap(false, Ordering::AcqRel)) {
            match self.authority.bound_browser(self.browser_owner) {
                Ok(Some(_)) => {
                    if self.navigate_private(ctx).is_err() {
                        self.failed = true;
                        self.close();
                    }
                }
                Ok(None) => {}
                Err(_) => {
                    self.failed = true;
                    self.close();
                }
            }
        }
        if self.osr_refused {
            web::set_host(solve, false);
            web::set_problem(solve, web::Problem::Failed);
            web::advance(solve, ctx);
            return;
        }
        match self.server.take_batch() {
            Ok(batch) => {
                if !batch.is_empty() { ctx.request_repaint(); }
                for entry in batch {
                let admitted = self.can_input();
                let authority = self.authority.clone();
                // Only the bounded business decision runs under this lease.
                // CEF/native housekeeping is deliberately deferred below.
                if let Ok(_lease) = authority.dispatch_lease(entry.document) {
                    self.receive(solve, ctx, &entry.body, admitted);
                }
                self.apply_theme_ack();
                }
            }
            Err(_) => self.failed = true,
        }
        if self.server.take_failure().is_some() {
            self.failed = true;
        }
        if self.failed {
            web::set_problem(solve, web::Problem::Failed);
        }
        // The conversation goes on while the window is behind another one.
        web::advance(solve, ctx);
        // Page readiness/theme delivery do not depend on a panel placement or
        // on any native texture having been admitted.
        self.sync_page(solve, ctx);
        if let Some(text) = COPY_OUT.with(|slot| slot.borrow_mut().take()) {
            ctx.copy_text(text);
        }
    }

    /// What the page says: the same contract as the WRY host (`assistant_web::receive`).
    fn receive(&mut self, solve: &mut SolveApp, ctx: &Context, message: &str, admitted: bool) {
        let value = serde_json::from_str::<serde_json::Value>(message).unwrap_or_default();
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("ready") => {
                // A reloaded page has neither the theme function nor its CSS variables: put the
                // bridge back and send the theme again, not only when the tokens change.
                self.page_ready = true;
            }
            Some("theme-ready") => {
                let ticket = value.get("ticket").and_then(serde_json::Value::as_str).and_then(ThemeTicket::parse);
                if ticket.is_some() && self.pending_theme == ticket { self.deferred_theme_ack = ticket; }
            }
            Some("focus") => {
                self.focused = !self.input_quarantined && admitted && value.get("on").and_then(serde_json::Value::as_bool).unwrap_or(false);
            }
            Some("log") => {
                let text = value.get("text").and_then(serde_json::Value::as_str).unwrap_or_default();
                let scope = text.starts_with("SCOPE ") && std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE").is_some();
                let dom = text.starts_with("DOM ") && std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_PROBE_DOM").is_some();
                if scope || dom { note(&bounded_log_text(&crate::assistant_cef_http::sanitized_diagnostic(text))); }
            }
            _ => {
                if self.admission.is_some() && !admitted {
                    return;
                }
                if web::act(solve, ctx, message).is_err() {
                    note("CEF: private action refused");
                }
            }
        }
    }

    fn apply_theme_ack(&mut self) {
        let Some(ticket) = self.deferred_theme_ack.take() else { return };
        if self.document.is_none_or(|id| !self.authority.permits(id)) || self.pending_theme != Some(ticket) { return; }
        let Some(floors) = self.drain_frames() else { return };
        if self.admission.as_mut().is_none_or(|a| a.theme_applied(ticket, floors, Instant::now())) {
            self.pending_theme = None;
            let due = self.admission.as_mut().is_none_or(|a| a.retry_due(Instant::now()));
            if due && self.producer.request_repaint().is_err() { note("CEF: themed repaint failed"); }
        }
    }

    fn navigate_private(&mut self, ctx: &Context) -> Result<(), crate::assistant_cef_http::TransportError> {
        use crate::assistant_cef_http::TransportError;
        if self.document.take().is_some() { self.authority.revoke(); }
        self.new_page(ctx);
        let mut random = [0; 32];
        getrandom::fill(&mut random).map_err(|_| TransportError::Io)?;
        let document = self.authority.begin_document(self.browser_owner, random)?;
        let url = self.authority.bootstrap_url(document, self.server.port())?;
        let browser_id = self.authority.bound_browser(self.browser_owner)?.ok_or(TransportError::BrowserMismatch)?;
        self.authority.arm_main_navigation(NavigationIdentity { document, owner: self.browser_owner, browser_id, url: url.clone() })?;
        self.document = Some(document);
        self.producer.navigate_to_url(&url).map_err(|_| TransportError::Io)
    }

    /// Once a frame, after the app's UI: place, resize, take the texture, draw it.
    pub fn tick(&mut self, solve: &mut SolveApp, ctx: &Context, frame: &mut eframe::Frame) {
        self.placed_this_frame = false;
        if self.osr_refused {
            return;
        }
        if web::take_retry(solve) {
            if !self.lifecycle.permits_retry() {
                // Closing, quarantined or not-yet-created: Retry must not make a
                // second runtime owner or navigate a browser that is going away.
                note("CEF: Retry refused: no live browser in an open runtime");
                web::set_problem(solve, web::Problem::Failed);
                return;
            }
            self.failed = false;
            self.input_failed = false;
            self.viewport.reset(Instant::now());
            self.told.clear();
            self.told_theme.clear();
            self.pending_theme = None;
            self.reset_admission(ctx);
            if self.navigate_private(ctx).is_err() {
                self.failed = true;
                self.close();
                web::set_problem(solve, web::Problem::Failed);
            }
            web::set_host(solve, true);
        }
        if self.document.is_none_or(|id| !self.authority.permits(id)) {
            self.popup_current = false;
            self.queue.invalidate();
            return;
        }
        let placed = web::placement(solve, ctx);
        self.placed_this_frame = placed.is_some();
        // While the existing failure UI offers Retry, keep processing a late
        // compatible capture at the last target. A cached target never permits
        // drawing or input: only a placement actually supplied by this UI does.
        let Some(rect) = placed.or_else(|| self.failed.then_some(self.last_rect).flatten()) else {
            if self.shown {
                self.shown = false;
                let _ = self.producer.set_visible(false);
                self.placement = None;
                self.reset_admission(ctx);
            }
            return;
        };
        self.last_rect = Some(rect);
        // Align texel centres with the window framebuffer, not a fractional panel origin.
        let ppp = ctx.pixels_per_point();
        if !ppp.is_finite() || !(0.25..=8.0).contains(&ppp) || !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            if self.placement.take().is_some() {
                self.reset_admission(ctx);
            }
            return;
        }
        let pixels = [rect.min.x * ppp, rect.min.y * ppp, rect.max.x * ppp, rect.max.y * ppp].map(f32::round);
        if pixels.into_iter().any(|edge| !edge.is_finite() || f64::from(edge) < f64::from(i32::MIN) || f64::from(edge) > f64::from(i32::MAX)) {
            self.placement = None;
            self.reset_admission(ctx);
            self.failed = true;
            web::set_problem(solve, web::Problem::Failed);
            return;
        }
        let rect = egui::Rect::from_min_max(
            egui::pos2((rect.min.x * ppp).round() / ppp, (rect.min.y * ppp).round() / ppp),
            egui::pos2((rect.max.x * ppp).round() / ppp, (rect.max.y * ppp).round() / ppp),
        );
        if !self.shown {
            self.shown = true;
            let _ = self.producer.set_visible(true);
        }
        let physical = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
        let placement = Placement { origin_px: [(rect.min.x * ppp).round() as i32, (rect.min.y * ppp).round() as i32], physical, scale: ppp };
        if self.placement != Some(placement) {
            self.placement = Some(placement);
            self.reset_admission(ctx);
        }
        if physical.into_iter().any(|edge| edge == 0 || edge > self.host.device.limits().max_texture_dimension_2d) {
            self.failed = true;
            web::set_problem(solve, web::Problem::Failed);
            return;
        }
        let target = (physical, ppp);
        if self.viewport.needs_update(target, Instant::now()) {
            self.native_ready = false;
            self.viewport.begin(target);
            // Native setters configure layout/screen density. No CDP emulation,
            // CSS zoom or second physical→DIP input conversion.
            let result = self.producer.resize(dpi::PhysicalSize::new(physical[0], physical[1])).and_then(|()| self.producer.set_scale_factor(ppp));
            self.viewport.finish(target, result.is_ok(), Instant::now());
            self.native_ready = result.is_ok();
            if let Err(error) = result {
                note(&format!("CEF: viewport configuration failed: {error}"));
            }
            note(&format!("CEF: viewport physical={}x{} scale={ppp}", physical[0], physical[1]));
        }
        self.ticks = self.ticks.saturating_add(1);
        if self.last_stats.elapsed() >= Duration::from_secs(2) {
            note(&format!("stats over 2 s: ticks={} state_pushes={} cef_frames={}", self.ticks, self.pushes, self.frames));
            self.ticks = 0;
            self.pushes = 0;
            self.frames = 0;
            self.last_stats = Instant::now();
        }
        self.pump(ctx);
        self.sync_page(solve, ctx);

        if let Some(native) = self.producer.acquire_native_frame()
            && self.native_ready
        {
            // The real kind of this callback, read before the frame is imported: a CPU bitmap
            // (feature `cpu-paint-fallback`) must never produce a positive accelerated receipt.
            let native_kind = native.kind();
            let metadata = native.capture_metadata();
            let native_size = [native.size().width, native.size().height];
            let geometry = admission::check_transport_view(self.admission.as_ref(), metadata.as_ref());
            let checked = self.admission.as_ref().map_or(Ok(()), |_| {
                geometry?;
                let metadata = metadata.as_ref().ok_or(FrameReject::MissingMetadata)?;
                if native.generation() != metadata.generation
                    || i32::try_from(native.size().width).ok() != Some(metadata.geometry.coded_size[0])
                    || i32::try_from(native.size().height).ok() != Some(metadata.geometry.coded_size[1])
                {
                    return Err(FrameReject::ImportMismatch);
                }
                Ok(())
            });
            if let Err(error) = checked {
                if self.last_reject != Some(error) {
                    self.record_frame_refusal("view", metadata.as_ref(), Some(native_size), None, None, &format!("{error:?}"));
                    note(&format!("CEF: native view refused: {error:?}"));
                }
                self.last_reject = Some(error);
            } else {
                self.last_reject = None;
            }
            if checked.is_ok() {
                match welding::WgpuTextureImporter::import(native, &self.host) {
                    Ok(texture) => {
                        if render_state(frame).is_none() {
                            self.failed = true;
                            return;
                        }
                        let imported_size = [texture.texture.width(), texture.texture.height()];
                        let imported_generation = texture.generation;
                        let admitted = self.admission.as_mut().map_or(Ok(None), |admission| {
                            let metadata = metadata.as_ref().ok_or(FrameReject::MissingMetadata)?;
                            if texture.capture_metadata != Some(*metadata)
                                || texture.texture.width() != texture.size.width
                                || texture.texture.height() != texture.size.height
                                || texture.texture.depth_or_array_layers() != 1
                            {
                                return Err(FrameReject::ImportMismatch);
                            }
                            admission.admit_view(metadata, imported_size, imported_generation).map(Some)
                        });
                        match admitted {
                            Ok(ticket) if !self.input_failed => {
                                self.queue.ticket = ticket;
                                if let Some(metadata) = metadata.as_ref() {
                                    self.record_view_admitted(metadata, native_size, imported_size, imported_generation);
                                }
                                self.publish(render_state(frame), texture);
                                if !self.accelerated_recorded && native_kind == welding::native_frame::NativeFrameKind::DmaBufImage {
                                    // Only the DMA-BUF frame CEF actually delivered counts:
                                    // admission alone would report success for a CPU fallback.
                                    self.accelerated_recorded = true;
                                    emit_accelerated_limit(true, "DMA-BUF accelerated texture import admitted");
                                }
                                self.failed = false;
                            }
                            Ok(_) => {}
                            Err(error) => {
                                self.record_frame_refusal(
                                    "view",
                                    metadata.as_ref(),
                                    Some(native_size),
                                    Some(imported_size),
                                    Some(imported_generation),
                                    &format!("{error:?}"),
                                );
                                note(&format!("CEF: imported view refused: {error:?}"));
                            }
                        }
                    }
                    Err(error) => {
                        self.record_frame_refusal("view", metadata.as_ref(), Some(native_size), None, None, &format!("{error}"));
                        note(&format!("CEF: the page's frame could not be imported: {error}"));
                        self.failed = true;
                    }
                }
            }
        }
        self.popup = self.take_popup(frame);
        if let Some(admission) = &mut self.admission {
            if admission.retry_due(Instant::now()) {
                let _ = self.producer.request_repaint();
            }
            if let Some(error) = admission.pending_error(Instant::now()) {
                if !self.failed {
                    note(&format!("CEF: waiting for compatible native view: {error:?}"));
                }
                self.failed = true;
            }
        }
        if self.failed || self.input_failed {
            web::set_host(solve, false);
            web::set_problem(solve, web::Problem::Failed);
        } else {
            web::set_host(solve, true);
        }

        if self.can_input()
            && let Some((id, size)) = self.texture
        {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("assistant-cef-page")));
            // One imported texel per framebuffer pixel. A stale resize frame is clipped,
            // never stretched; the native themed frame backs any newly exposed area.
            let draw = egui::Rect::from_min_size(rect.min, egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp));
            painter.with_clip_rect(rect).image(id, draw, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            if self.popup_current
                && let Some((popup, at, size)) = self.popup
            {
                // CEF reports the popup's own rectangle in physical pixels, relative to the
                // view: a native <select> list belongs under its control, not at a fixed spot.
                let popup_rect = egui::Rect::from_min_size(
                    draw.min + egui::vec2(at[0] as f32 / ppp, at[1] as f32 / ppp),
                    egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp),
                );
                painter.image(popup, popup_rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
        }

        // Test only: four known colours, to measure what the texture path does to the page's
        // own values. The page is untouched otherwise.
        if !self.test_select && self.can_input() && std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_PROBE_SWATCH").is_some() {
            self.test_select = true; // one shot, the same flag
            let _ = self.producer.execute_script(
                "(() => { const d = document.createElement('div'); \
                 d.id='__intcad_swatch'; d.style.cssText='position:fixed;left:8px;top:8px;z-index:2147483647;display:flex'; \
                 for (const c of ['#000000','#808080','#ffffff','#ff0000','#3a88ee']) { \
                   const s=document.createElement('div'); s.style.cssText='width:32px;height:32px;background:'+c; d.appendChild(s); } \
                 document.body.appendChild(d); })()",
                "",
            );
        }
        // Test only (SOLIDCRAFT_ASSISTANT_CEF_TEST_SELECT): a native select in the real page,
        // so the popup surface path is exercised in the app rather than only in the probe.
        if !self.test_select && self.can_input() && std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_TEST_SELECT").is_some() {
            self.test_select = true;
            let _ = self.producer.execute_script(
                "(() => { const s = document.createElement('select'); s.id = '__cef_test_select'; \
                 s.style.cssText = 'position:fixed;left:8px;top:8px;width:120px;height:26px;z-index:2147483647'; \
                 for (const v of ['alpha','beta','gamma']) { const o = document.createElement('option'); o.textContent = v; s.appendChild(o); } \
                 document.body.appendChild(s); })()",
                "",
            );
        }
        // egui only tells winit to open an input method while one of its own text widgets has
        // the keyboard. The page's editor is inside CEF, so the host asks for it itself, and
        // without this every IME key arrived as plain Latin text.
        self.set_ime(ctx, self.focused && self.can_input());
        // Test only: a real browser reload, to prove the theme survives one.
        if !self.test_reloaded
            && self.can_input()
            && let Some(after) = std::env::var("SOLIDCRAFT_ASSISTANT_CEF_TEST_RELOAD_AFTER").ok().and_then(|value| value.parse::<f32>().ok())
            && self.started.elapsed().as_secs_f32() >= after
        {
            self.test_reloaded = true;
            note("CEF: test reload");
            if self.navigate_private(ctx).is_err() { self.failed = true; self.close(); }
        }
        // Test only: a fixed, in-source scope probe, once per page lifetime.
        if !self.scope_injected && self.page_ready && self.shown && std::env::var("SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE").ok().as_deref() == Some("1")
        {
            self.scope_injected = true;
            if let Err(error) = self.inject_scope_probe() {
                note(&format!("CEF: scope probe injection failed: {error}"));
            }
        }
        self.input(solve, ctx, rect, ppp);
        // The page paints on change; the host only has to ask soon while it is working.
        let working = self.last_frame.elapsed() < Duration::from_millis(400);
        ctx.request_repaint_after(if working { Duration::from_millis(16) } else { Duration::from_millis(200) });
    }

    fn sync_page(&mut self, solve: &mut SolveApp, ctx: &Context) {
        if !self.page_ready || self.document.is_none_or(|id| !self.authority.permits(id)) { return; }
        self.check_transport();
        // State is pushed with the app's own logic; the page is told when it changed.
        let state = web::state(solve);
        // The palette first, every frame the tokens differ, whether or not the conversation
        // changed: a theme switch must reach the page on its own.
        if std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_THEME_TRACE").is_some() {
            note(&format!(
                "theme trace: texture={} state={} bytes has_theme={} told={}",
                self.texture.is_some(),
                state.len(),
                state.contains("\"theme\""),
                self.told_theme.len()
            ));
        }
        if self.page_ready {
            let theme = serde_json::from_str::<serde_json::Value>(&state)
                .ok()
                .and_then(|value| value.get("theme").cloned())
                .map(|theme| theme.to_string())
                .unwrap_or_default();
            if !theme.is_empty() && (theme != self.told_theme || theme != self.requested_theme) {
                if theme != self.requested_theme {
                    let Some(revision) = self.theme_revision.checked_add(1) else {
                        self.failed = true;
                        self.input_failed = true;
                        self.identity_exhausted = true;
                        self.cancel_input(ctx);
                        web::set_problem(solve, web::Problem::Failed);
                        return;
                    };
                    self.theme_revision = revision;
                    let ticket = ThemeTicket { lifetime: self.lifetime, revision };
                    self.cancel_input(ctx);
                    let _ = self.drain_frames();
                    self.popup_current = false;
                    if let Some(admission) = &mut self.admission
                        && let Err(error) = admission.theme_pending(ticket, Instant::now())
                    {
                        note(&format!("CEF: theme admission failed: {error:?}"));
                        self.failed = true;
                        return;
                    }
                    self.pending_theme = Some(ticket);
                    self.requested_theme = theme.clone();
                }
                let Some(ticket) = self.pending_theme else { return };
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&theme) {
                    note(&format!(
                        "theme {} dark={} surface={} text={} font={}",
                        value["id"].as_str().unwrap_or("?"),
                        value["dark"].as_bool().unwrap_or(false),
                        value["colors"]["surface"].as_str().unwrap_or("?"),
                        value["colors"]["text"].as_str().unwrap_or("?"),
                        value["metrics"]["fontBody"].as_f64().unwrap_or(0.0)
                    ));
                }
                let script = format!("window.__intcadTheme&&window.__intcadTheme({theme},'{}:{}')", ticket.lifetime, ticket.revision);
                if self.producer.execute_script(THEME_BRIDGE_JS, "").is_ok() && self.producer.execute_script(&script, "").is_ok() {
                    self.told_theme = theme;
                    // Test hook: report what the page actually lays out, to compare with the
                    // control height and font size the app draws its own widgets with.
                    if std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_PROBE_DOM").is_some() {
                        let _ = self.producer.execute_script(
                            "(() => { const cs = getComputedStyle(document.documentElement); \
                             const h = e => Math.round(e.getBoundingClientRect().height); \
                             const buttons = [...document.querySelectorAll('button')].slice(0,6).map(h); \
                             const inputs = [...document.querySelectorAll('input,textarea')].slice(0,4).map(h); \
                             const ta = document.querySelector('textarea'); \
                             const r = ta ? ta.getBoundingClientRect() : null; const composer = r ? { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) } : null; \
                             window.ipc.postMessage(JSON.stringify({ type:'log', text: 'DOM ' + JSON.stringify({ font: cs.fontSize, iw: innerWidth, ih: innerHeight, dpr: devicePixelRatio, buttons, inputs, composer }) })); })()",
                            "",
                        );
                    }
                }
            }
        }
        if state != self.told {
            if let Ok(mut held) = self.pictures.lock() {
                *held = web::pictures(solve).into_iter().collect();
            }
            if self.producer.execute_script(&format!("window.__assistantHost&&window.__assistantHost.state({state})"), "").is_ok() {
                self.told = state;
                self.pushes = self.pushes.saturating_add(1);
            }
        }
    }

    /// Ask CEF to close exactly once and report owned progress. Completion is
    /// the matching platform callback, never this request.
    pub fn begin_close(&mut self) -> CloseProgress {
        if !matches!(self.lifecycle.phase(), Phase::Closing | Phase::Quarantined | Phase::Closed | Phase::Shutdown) {
            self.lifecycle.begin_close(Instant::now());
            self.authority.revoke();
            self.document = None;
            self.server.request_stop();
            self.queue.invalidate();
            let _ = self.producer.ime_cancel_composition();
            #[cfg(target_os = "linux")]
            let _ = self.producer.set_page_focus(false);
            self.focused = false;
            let _ = self.producer.close();
        }
        self.poll_close()
    }

    /// Advance close state from the platform's owned completion only. A late
    /// callback after quarantine may finish cleanup, but it cannot resume quit.
    pub fn poll_close(&mut self) -> CloseProgress {
        if matches!(self.lifecycle.phase(), Phase::Closing | Phase::Quarantined) {
            match self.producer.close_progress() {
                Ok(welding::surface::BrowserCloseProgress::Complete) => {
                    let _ = self.lifecycle.observe_closed(self.browser_id);
                }
                Ok(welding::surface::BrowserCloseProgress::Pending) | Err(_) => {}
            }
        }
        // Shutdown is not marked here: the native runtime is still live until
        // CefAssistant::shutdown performs its one checked call.
        self.lifecycle.poll(Instant::now())
    }

    pub fn close(&mut self) {
        let progress = self.begin_close();
        log::debug!("CEF close progress: {progress:?}");
    }

    /// Consume the app's owners and shut CEF down only after the platform
    /// observed matching close completion. Anything else is retained, never
    /// dropped into an implicit native shutdown.
    pub fn shutdown(self) -> Result<(), welding::WeldError> {
        let Self { runtime, lease, mut lifecycle, producer, mut server, host, .. } = self;
        if lifecycle.phase() != Phase::Closed {
            // Unresolved ownership is retained whole: no CEF refcount release and
            // no native shutdown. Only the Rust-side lease and the GPU/server
            // helpers (not CEF owners) are released.
            server.request_stop();
            drop(host);
            drop(lease);
            quarantine(runtime, Some(producer));
            return Err(welding::WeldError::BrowserOp(
                "CEF close completion was not observed".into(),
            ));
        }
        // Browser/producer, frame/request-context and server owners go before the
        // single native shutdown.
        drop(producer);
        server.request_stop();
        drop(server);
        drop(host);
        drop(lease);
        runtime.shutdown().map_err(|(runtime, error)| {
            quarantine(runtime, None);
            error
        })?;
        // Policy bookkeeping only after the single native shutdown returned.
        let _ = lifecycle.mark_shutdown();
        Ok(())
    }

    pub fn focused(&self) -> bool {
        self.focused && self.shown && self.can_input()
    }

    pub fn edit_text(&mut self, redo: bool) {
        if !self.can_input() || !self.focused {
            return;
        }
        let _ = self.producer.execute_script(if redo { "document.execCommand('redo')" } else { "document.execCommand('undo')" }, "");
    }

    fn can_input(&self) -> bool {
        self.document.is_some_and(|id| self.authority.permits(id))
            && self.shown
            && self.placed_this_frame
            && self.native_ready
            && self.queue.ready
            && !self.input_failed
            && !self.identity_exhausted
            && !self.failed
            && self.admission.as_ref().is_none_or(|admission| self.queue.ticket.is_some_and(|ticket| admission.permits_input(ticket)))
    }

    fn drain_frames(&mut self) -> Option<FrameWatermarks> {
        #[cfg(target_os = "linux")]
        if self.admission.is_some() {
            return match self.producer.discard_pending_frames() {
                Ok(floors) => Some(floors),
                Err(error) => {
                    note(&format!("CEF: frame drain failed: {error}"));
                    self.input_failed = true;
                    None
                }
            };
        }
        Some(FrameWatermarks::default())
    }

    fn cancel_input(&mut self, ctx: &Context) {
        self.queue.invalidate();
        // Even a down+up entirely inside this egui frame predates the reset.
        // Discard that frame, then wait for held physical input to become neutral.
        self.input_quarantined = true;
        self.focused = false;
        self.copy_request = None;
        self.set_ime(ctx, false);
        if let Err(error) = self.producer.ime_cancel_composition() {
            note(&format!("CEF: IME cancellation failed: {error}"));
            self.input_failed |= self.admission.is_some();
        }
        #[cfg(target_os = "linux")]
        if let Err(error) = self.producer.set_page_focus(false) {
            note(&format!("CEF: native blur failed: {error}"));
            self.input_failed |= self.admission.is_some();
        }
    }

    fn reset_admission(&mut self, ctx: &Context) {
        self.cancel_input(ctx);
        self.popup_current = false;
        self.last_reject = None;
        self.told_theme.clear();
        self.requested_theme.clear();
        self.pending_theme = None;
        let floors = self.drain_frames().unwrap_or_default();
        match admission::reset_transport(self.admission.as_mut(), self.lifetime, self.placement, floors, Instant::now()) {
            Ok(true) => {
                if let Err(error) = self.producer.request_repaint() {
                    note(&format!("CEF: reset repaint failed: {error}"));
                    self.failed = true;
                }
            }
            Ok(false) => {}
            Err(error) => {
                note(&format!("CEF: admission reset failed: {error:?}"));
                self.failed = true;
            }
        }
    }

    fn new_page(&mut self, ctx: &Context) {
        self.page_ready = false;
        self.deferred_theme_ack = None;
        self.transport_probe = None;
        self.probe_due = Instant::now();
        COPY_OUT.with(|slot| *slot.borrow_mut() = None);
        self.scope_injected = false;
        self.last_view_receipt = None;
        self.last_frame_reason = None;
        self.told.clear();
        self.native_ready = false;
        self.viewport.reset(Instant::now());
        match self.lifetime.checked_add(1) {
            Some(lifetime) => self.lifetime = lifetime,
            None => {
                note("CEF: page identity exhausted");
                self.input_failed = true;
                self.identity_exhausted = true;
                self.failed = true;
            }
        }
        self.reset_admission(ctx);
    }

    fn publish(&mut self, render_state: Option<&eframe::egui_wgpu::RenderState>, frame: ImportedTexture) {
        let Some(render_state) = render_state else { return };
        let size = [frame.size.width, frame.size.height];
        let previous_size = self.texture.map(|(_, size)| size);
        let plain = match frame.format {
            wgpu::TextureFormat::Bgra8UnormSrgb => Some(wgpu::TextureFormat::Bgra8Unorm),
            wgpu::TextureFormat::Rgba8UnormSrgb => Some(wgpu::TextureFormat::Rgba8Unorm),
            _ => None,
        };
        let mut renderer = render_state.renderer.write();
        match plain {
            Some(plain) => {
                let stale = self.page.as_ref().is_none_or(|page| page.size != size);
                if stale {
                    let formats = [plain];
                    let texture = self.host.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("assistant-page"),
                        size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: frame.format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &formats,
                    });
                    let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(plain), ..Default::default() });
                    let id = renderer.register_native_texture(&self.host.device, &view, wgpu::FilterMode::Linear);
                    if let Some(old) = self.page.take() {
                        renderer.free_texture(&old.id);
                    }
                    if let Some((old, _)) = self.texture.take() {
                        if old != id {
                            renderer.free_texture(&old);
                        }
                    }
                    self.page = Some(PageTarget { id, texture, size });
                }
                if let Some(page) = &self.page {
                    let mut encoder = self.host.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("assistant-page-copy") });
                    encoder.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &frame.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture: &page.texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                    );
                    self.host.queue.submit([encoder.finish()]);
                    self.texture = Some((page.id, size));
                }
            }
            None => {
                // A non-sRGB frame needs no copy: egui reads it as it is.
                if let Some((id, _)) = self.texture.take() {
                    renderer.free_texture(&id);
                }
                let id = renderer.register_native_texture(&self.host.device, &frame.view, wgpu::FilterMode::Linear);
                self.texture = Some((id, size));
            }
        }
        drop(renderer);
        if previous_size != Some(size) {
            note(&format!("CEF: raster {}x{}", size[0], size[1]));
        }
        self.frames = self.frames.saturating_add(1);
        if !self.queue.ready {
            self.queue.ready = true;
            note("CEF: native view admitted");
        }
        self.last_frame = Instant::now();
    }

    fn take_popup(&mut self, frame: &eframe::Frame) -> Option<(egui::TextureId, [i32; 2], [u32; 2])> {
        if !self.can_input() {
            self.popup_current = false;
            return self.popup;
        }
        let render_state = frame.wgpu_render_state()?;
        match self.producer.acquire_popup(&self.host) {
            Ok(Some(popup)) => {
                let size = [popup.texture.size.width, popup.texture.size.height];
                let checked = if let Some(admission) = &self.admission {
                    popup.texture.capture_metadata.as_ref().ok_or(FrameReject::MissingMetadata).and_then(|metadata| {
                        let width = i32::try_from(popup.rect.width).map_err(|_| FrameReject::WrongSize)?;
                        let height = i32::try_from(popup.rect.height).map_err(|_| FrameReject::WrongSize)?;
                        if metadata.generation <= self.popup_generation {
                            return Err(FrameReject::OldGeneration);
                        }
                        if popup.texture.texture.width() != size[0]
                            || popup.texture.texture.height() != size[1]
                            || popup.texture.texture.depth_or_array_layers() != 1
                        {
                            return Err(FrameReject::ImportMismatch);
                        }
                        admission.check_popup(metadata, FrameRect { x: popup.rect.x, y: popup.rect.y, width, height }, size, popup.texture.generation)
                    })
                } else {
                    Ok(())
                };
                let popup_physical = popup.texture.capture_metadata.as_ref().map(|metadata| {
                    [u32::try_from(metadata.geometry.coded_size[0]).unwrap_or(0), u32::try_from(metadata.geometry.coded_size[1]).unwrap_or(0)]
                });
                if let Err(error) = checked {
                    self.record_frame_refusal(
                        "popup",
                        popup.texture.capture_metadata.as_ref(),
                        popup_physical,
                        Some(size),
                        Some(popup.texture.generation),
                        &format!("{error:?}"),
                    );
                    note(&format!("CEF: imported popup refused: {error:?}"));
                    self.popup_current = false;
                    return self.popup;
                }
                self.popup_generation = popup.texture.generation;
                self.frame_receipt(
                    "popup",
                    popup.texture.capture_metadata.as_ref(),
                    popup_physical,
                    Some(size),
                    Some(popup.texture.generation),
                    "admitted",
                    "",
                );
                self.popup_current = true;
                note(&format!("popup surface {}x{} at {},{}", size[0], size[1], popup.rect.x, popup.rect.y));
                let plain = match popup.texture.format {
                    wgpu::TextureFormat::Bgra8UnormSrgb => Some(wgpu::TextureFormat::Bgra8Unorm),
                    wgpu::TextureFormat::Rgba8UnormSrgb => Some(wgpu::TextureFormat::Rgba8Unorm),
                    _ => None,
                };
                let mut renderer = render_state.renderer.write();
                match plain {
                    Some(plain) => {
                        // The same gamma contract as the page: egui reads gamma values.
                        let stale = self.popup_page.as_ref().is_none_or(|page| page.size != size);
                        if stale {
                            let formats = [plain];
                            let texture = self.host.device.create_texture(&wgpu::TextureDescriptor {
                                label: Some("assistant-popup"),
                                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: popup.texture.format,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                                view_formats: &formats,
                            });
                            let view = texture.create_view(&wgpu::TextureViewDescriptor { format: Some(plain), ..Default::default() });
                            let id = renderer.register_native_texture(&self.host.device, &view, wgpu::FilterMode::Linear);
                            if let Some(old) = self.popup_page.take() {
                                renderer.free_texture(&old.id);
                            }
                            if let Some((old, _, _)) = self.popup.take() {
                                if old != id {
                                    renderer.free_texture(&old);
                                }
                            }
                            self.popup_page = Some(PageTarget { id, texture, size });
                        }
                        if let Some(page) = &self.popup_page {
                            let mut encoder =
                                self.host.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("assistant-popup-copy") });
                            encoder.copy_texture_to_texture(
                                wgpu::TexelCopyTextureInfo {
                                    texture: &popup.texture.texture,
                                    mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                },
                                wgpu::TexelCopyTextureInfo {
                                    texture: &page.texture,
                                    mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                },
                                wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                            );
                            self.host.queue.submit([encoder.finish()]);
                            self.popup = Some((page.id, [popup.rect.x, popup.rect.y], size));
                        }
                    }
                    None => {
                        if let Some((id, _, _)) = self.popup.take() {
                            renderer.free_texture(&id);
                        }
                        self.popup = Some((
                            renderer.register_native_texture(&self.host.device, &popup.texture.view, wgpu::FilterMode::Linear),
                            [popup.rect.x, popup.rect.y],
                            size,
                        ));
                    }
                }
                self.popup
            }
            Ok(None) => {
                if self.producer.popup_rect().is_none() {
                    self.popup_current = false;
                    // Gamma-copy registrations belong to popup_page and can be
                    // reused; don't free that id while retaining its PageTarget.
                    if self.popup_page.is_none()
                        && let Some((id, _, _)) = self.popup.take()
                    {
                        render_state.renderer.write().free_texture(&id);
                    }
                }
                self.popup
            }
            Err(_) => self.popup,
        }
    }

    fn pump(&mut self, ctx: &Context) {
        self.runtime.do_message_loop_work();
        while let Some(event) = self.producer.poll_web_event() {
            match &event {
                CefSurfaceEvent::Navigation(welding::NavigationEvent::BrowserCreated { browser_id, windowless }) => {
                    self.browser_id = *browser_id;
                    if self.lifecycle.observe_created(*browser_id).is_err() {
                        // A creation we do not own cannot become a second runtime owner.
                        note("CEF: browser creation did not match the owned request");
                    }
                    self.browser_windowless = *windowless;
                    note(&format!("CEF_BROWSER {}", serde_json::json!({"browser_id": browser_id, "windowless": windowless, "action": "created"})));
                    if !windowless {
                        self.osr_refused = true;
                        self.cancel_input(ctx);
                    }
                }
                // LoadStart/ready are observations, never authority renewal.
                CefSurfaceEvent::Navigation(welding::NavigationEvent::LoadStart { .. }) => {}
                CefSurfaceEvent::Navigation(welding::NavigationEvent::ContentProcessTerminated { .. }) => {
                    self.authority.revoke();
                    self.document = None;
                    self.new_page(ctx);
                    self.failed = true;
                }
                CefSurfaceEvent::Navigation(welding::NavigationEvent::NewWindowRequested { .. }) => note(&format!(
                    "CEF_BROWSER {}",
                    serde_json::json!({"browser_id": self.browser_id, "windowless": self.browser_windowless, "action": "popup_denied"})
                )),
                _ => {}
            }
            if let CefSurfaceEvent::ScriptCompleted { id, result } = &event {
                if self.transport_probe.is_some_and(|(pending, _)| pending == id.get()) {
                    let current = self.transport_probe.take().is_some_and(|(_, document)| {
                        self.document == Some(document) && self.authority.permits(document)
                    });
                    if current && result.as_ref().is_ok_and(|json| json == "true") { self.failed = true; }
                    self.probe_due = Instant::now() + Duration::from_millis(500);
                }
                if self.copy_request.is_some_and(|(pending, _)| pending == id.get()) {
                    let current = self.copy_request.is_some_and(|(_, document)| {
                        self.document == Some(document) && self.authority.permits(document)
                    });
                    self.copy_request = None;
                    if current && let Ok(json) = result {
                        let text = json_string(json);
                        if !text.is_empty() {
                            note(&format!("CEF: copied {} byte(s) out of the page", text.len()));
                            // The window's clipboard, not Chromium's: CEF OSR has none (cef#4341).
                            COPY_OUT.with(|slot| *slot.borrow_mut() = Some(text));
                        }
                    }
                }
            }
        }
    }

    fn input(&mut self, solve: &mut SolveApp, ctx: &Context, rect: egui::Rect, ppp: f32) {
        if self.admission.is_some() && !self.can_input() {
            self.input_quarantined |= ctx.input(|input| input.pointer.any_down() || input.modifiers.any() || !input.keys_down.is_empty());
            self.queue.clear();
            self.focused = false;
            self.set_ime(ctx, false);
            return;
        }
        if self.input_quarantined {
            self.input_quarantined = ctx.input(|input| input.pointer.any_down() || input.modifiers.any() || !input.keys_down.is_empty());
            // Also discard the neutralizing release frame itself.
            self.queue.clear();
            return;
        }
        let events = ctx.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::PointerMoved(pos) if rect.contains(pos) => {
                    let local = (pos - rect.min) * ppp;
                    self.queue.push(Queued::Mouse(MouseEvent {
                        x: local.x.round() as i32,
                        y: local.y.round() as i32,
                        button: MouseButton::Left,
                        action: MouseAction::Moved,
                        modifiers: EventModifiers::default(),
                    }));
                }
                egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                    if pressed && !rect.contains(pos) {
                        // The CAD side takes the keyboard back.
                        self.cancel_input(ctx);
                        // Blurring does not invalidate the admitted picture.
                        self.queue.ready = true;
                        self.queue.ticket = self.admission.as_ref().and_then(FrameAdmission::input_ticket);
                        return;
                    }
                    let cef_button = match button {
                        egui::PointerButton::Primary => MouseButton::Left,
                        egui::PointerButton::Secondary => MouseButton::Right,
                        egui::PointerButton::Middle => MouseButton::Middle,
                        _ => continue,
                    };
                    if pressed {
                        if !self.can_input() {
                            continue;
                        }
                        self.focused = true;
                        self.set_ime(ctx, true);
                        if let Err(error) = self.producer.move_focus(FocusDirection::Forward) {
                            note(&format!("CEF: native focus failed: {error}"));
                            self.input_failed |= self.admission.is_some();
                            self.cancel_input(ctx);
                            return;
                        }
                    }
                    let local = (pos - rect.min) * ppp;
                    self.queue.push(Queued::Mouse(MouseEvent {
                        x: local.x.round() as i32,
                        y: local.y.round() as i32,
                        button: cef_button,
                        action: if pressed { MouseAction::Pressed } else { MouseAction::Released },
                        modifiers: EventModifiers { ctrl: modifiers.ctrl, shift: modifiers.shift, ..Default::default() },
                    }));
                }
                egui::Event::MouseWheel { unit, delta, .. } if rect.contains(ctx.pointer_latest_pos().unwrap_or(egui::Pos2::ZERO)) => {
                    let scale = match unit {
                        egui::MouseWheelUnit::Line => 20.0,
                        egui::MouseWheelUnit::Page => 120.0,
                        egui::MouseWheelUnit::Point => 1.0,
                    };
                    let pos = ctx.pointer_latest_pos().unwrap_or(rect.center());
                    let local = (pos - rect.min) * ppp;
                    self.queue.push(Queued::Mouse(MouseEvent {
                        x: local.x.round() as i32,
                        y: local.y.round() as i32,
                        button: MouseButton::Left,
                        action: MouseAction::WheelScrolled { delta_x: (delta.x * scale) as i32, delta_y: (delta.y * scale) as i32 },
                        modifiers: EventModifiers::default(),
                    }));
                }
                egui::Event::Text(text) if self.focused => {
                    for ch in text.chars() {
                        self.queue.push(Queued::Key(KeyEvent {
                            kind: KeyEventKind::Char,
                            windows_key_code: 0,
                            native_key_code: 0,
                            character: Some(ch),
                            modifiers: EventModifiers::default(),
                        }));
                    }
                }
                egui::Event::Key { key, pressed, modifiers, .. } if self.focused => {
                    if pressed && (modifiers.ctrl || modifiers.mac_cmd) && key == egui::Key::C {
                        self.request_copy();
                        continue;
                    }
                    if pressed && (modifiers.ctrl || modifiers.mac_cmd) && key == egui::Key::V {
                        continue; // arrives as Event::Paste
                    }
                    if key == egui::Key::Escape && pressed {
                        self.cancel_input(ctx);
                        self.queue.ready = true;
                        self.queue.ticket = self.admission.as_ref().and_then(FrameAdmission::input_ticket);
                        return;
                    }
                    if let Some(vk) = virtual_key(key) {
                        self.queue.push(Queued::Key(KeyEvent {
                            kind: if pressed { KeyEventKind::RawKeyDown } else { KeyEventKind::KeyUp },
                            windows_key_code: vk,
                            native_key_code: 0,
                            character: None,
                            modifiers: EventModifiers {
                                shift: modifiers.shift,
                                ctrl: modifiers.ctrl,
                                alt: modifiers.alt,
                                // `command` is an alias for ctrl off macOS: mapping it to Meta
                                // made every Ctrl shortcut arrive as Ctrl+Super.
                                meta: modifiers.mac_cmd,
                                ..Default::default()
                            },
                        }));
                    }
                }
                egui::Event::Copy => {
                    if self.focused {
                        self.request_copy();
                    }
                }
                egui::Event::Paste(text) if self.focused => {
                    // CEF's own clipboard is broken under OSR (cef#4341): the system's text is
                    // committed into the focused editable the way an input method commits.
                    self.queue.push(Queued::ImeCommit(text));
                }
                egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) if self.focused => {
                    if !text.is_empty() {
                    }
                    if text.is_empty() {
                        self.queue.push(Queued::ImeCancel);
                    } else {
                        let range = active_range_chars.unwrap_or(0..text.chars().count());
                        self.queue.push(Queued::ImeSet(text, (range.start as u32, range.end as u32)));
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) if self.focused => {
                    self.queue.push(Queued::ImeCommit(text));
                }
                _ => {}
            }
        }
        let _ = solve;
        if self.admission.is_some() && self.queue.overflowed {
            self.input_failed = true;
            self.cancel_input(ctx);
            return;
        }
        self.flush();
        if self.input_failed {
            self.cancel_input(ctx);
        }
    }

    fn set_ime(&mut self, ctx: &Context, on: bool) {
        if self.ime_on != on {
            self.ime_on = on;
            ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(on));
        }
    }

    fn request_copy(&mut self) {
        if self.copy_request.is_some() { return; }
        let Some(document) = self.document.filter(|id| self.authority.permits(*id)) else { return };
        let Some(id) = self.next_script_id() else { return };
        if self.producer.request_script_result(
            WebRequestId::new(id),
            "(() => { const el = document.activeElement; \
             if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) { \
               return el.value.substring(el.selectionStart, el.selectionEnd); } \
             return window.getSelection().toString(); })()",
        ).is_ok() { self.copy_request = Some((id, document)); }
    }

    fn next_script_id(&mut self) -> Option<u64> {
        match self.script_serial.checked_add(1) {
            Some(id) => { self.script_serial = id; Some(id) }
            None => { self.identity_exhausted = true; self.failed = true; None }
        }
    }

    fn check_transport(&mut self) {
        if self.transport_probe.is_some() || Instant::now() < self.probe_due { return; }
        let Some(document) = self.document.filter(|id| self.authority.permits(*id)) else { return };
        let Some(id) = self.next_script_id() else { return };
        self.probe_due = Instant::now() + Duration::from_millis(500);
        if self.producer.request_script_result(WebRequestId::new(id), "Boolean(window.__intcadTransportFailed)").is_ok() {
            self.transport_probe = Some((id, document));
        }
    }

    fn flush(&mut self) {
        if !self.queue.ready {
            return;
        }
        if self.admission.is_some() && !self.can_input() {
            self.queue.invalidate();
            return;
        }
        if let Some((x, y)) = self.queue.motion.take() {
            if let Err(error) = self.producer.send_mouse_input(MouseEvent {
                x,
                y,
                button: MouseButton::Left,
                action: MouseAction::Moved,
                modifiers: EventModifiers::default(),
            }) {
                note(&format!("CEF: motion refused: {error}"));
                self.input_failed |= self.admission.is_some();
                self.queue.invalidate();
                return;
            }
        }
        while let Some(event) = self.queue.critical.pop_front() {
            let result = match event {
                Queued::Mouse(event) => self.producer.send_mouse_input(event),
                Queued::Key(event) => self.producer.send_keyboard_input(event),
                Queued::ImeSet(text, cursor) => self.producer.ime_set_composition(&text, cursor),
                Queued::ImeCommit(text) => self.producer.ime_commit_text(&text),
                Queued::ImeCancel => self.producer.ime_cancel_composition(),
            };
            if let Err(error) = result {
                note(&format!("CEF: input refused: {error}"));
                self.input_failed |= self.admission.is_some();
                self.queue.invalidate();
                return;
            }
        }
    }
}

thread_local! {
    /// Text copied out of the page, handed to egui's clipboard on the next frame.
    static COPY_OUT: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn render_state(frame: &eframe::Frame) -> Option<&eframe::egui_wgpu::RenderState> {
    frame.wgpu_render_state()
}

fn json_string(json: &str) -> String {
    serde_json::from_str::<String>(json).unwrap_or_else(|_| json.trim_matches('"').to_owned())
}

fn virtual_key(key: egui::Key) -> Option<i32> {
    use egui::Key as K;
    Some(match key {
        K::Backspace => 0x08,
        K::Tab => 0x09,
        K::Enter => 0x0D,
        K::Escape => 0x1B,
        K::Space => 0x20,
        K::PageUp => 0x21,
        K::PageDown => 0x22,
        K::End => 0x23,
        K::Home => 0x24,
        K::ArrowLeft => 0x25,
        K::ArrowUp => 0x26,
        K::ArrowRight => 0x27,
        K::ArrowDown => 0x28,
        K::Delete => 0x2E,
        K::A => 0x41,
        K::B => 0x42,
        K::C => 0x43,
        K::D => 0x44,
        K::E => 0x45,
        K::F => 0x46,
        K::G => 0x47,
        K::H => 0x48,
        K::I => 0x49,
        K::J => 0x4A,
        K::K => 0x4B,
        K::L => 0x4C,
        K::M => 0x4D,
        K::N => 0x4E,
        K::O => 0x4F,
        K::P => 0x50,
        K::Q => 0x51,
        K::R => 0x52,
        K::S => 0x53,
        K::T => 0x54,
        K::U => 0x55,
        K::V => 0x56,
        K::W => 0x57,
        K::X => 0x58,
        K::Y => 0x59,
        K::Z => 0x5A,
        K::Num0 => 0x30,
        K::Num1 => 0x31,
        K::Num2 => 0x32,
        K::Num3 => 0x33,
        K::Num4 => 0x34,
        K::Num5 => 0x35,
        K::Num6 => 0x36,
        K::Num7 => 0x37,
        K::Num8 => 0x38,
        K::Num9 => 0x39,
        _ => return None,
    })
}

/// The page-side half of the theme bridge. The host passes the tokens it is drawing with;
/// this function turns them into the CSS variables Mastra's components already use. It is
/// installed on page readiness, without waiting for a texture.
const THEME_BRIDGE_JS: &str = r#"
window.__intcadTheme = (t, ticket) => {
  const c = t.colors, m = t.metrics, root = document.documentElement, set = (k, v) => root.style.setProperty(k, v);
  set('--background', c.window); set('--background-1', c.window);
  set('--background-2', c.surface); set('--background-3', c.raised);
  set('--card', c.raised); set('--popover', c.popup); set('--muted', c.field);
  set('--foreground', c.text); set('--muted-foreground', c.textDim);
  set('--border', c.borderStrong); set('--border-strong', c.border); set('--border-focus', c.accent);
  set('--color-background', c.window); set('--color-brand-blue', c.accent);
  set('--intcad-accent', c.accent); set('--intcad-selected', c.selected);
  const text = [c.text, c.text, c.text, c.textDim, c.textDim, c.textDim, c.border, c.control, c.raised, c.window];
  const ramp = t.dark ? text.slice().reverse() : text;
  ramp.forEach((v, i) => set('--gray-' + (i + 1), v));
  set('--radius-sm', m.radiusSm + 'px'); set('--radius-md', m.radiusMd + 'px');
  set('--radius-lg', m.radiusLg + 'px'); set('--radius-xl', m.radiusXl + 'px'); set('--radius', m.radiusMd + 'px');
  set('--font-body', "'Intcad Sans','IBM Plex Sans','Mona Sans',system-ui,sans-serif");
  // Font roles are point sizes, not fractions of the spacing rem. The old 12px root
  // combined with Mastra's .875rem body / .8125rem labels yielded 10.5 / 9.75px text.
  const fonts = {
    body: m.fontProse, 'body-sm': m.fontBody, label: m.fontLabel, menu: m.fontLabel,
    'card-title': m.fontLabel, 'card-title-tight': m.fontLabel, 'card-title-strong': m.fontLabel,
    caption: m.fontCaption, column: m.fontCaption, eyebrow: m.fontCaption, meta: m.fontSmall,
    subheading: m.fontHeading, heading: m.fontHeading, title: m.fontTitle
  };
  Object.entries(fonts).forEach(([role, size]) => set('--text-' + role, size + 'px'));
  root.style.fontSize = m.fontBody + 'px';
  root.style.colorScheme = t.dark ? 'dark' : 'light';
  root.dataset.intcadTheme = t.id;
  let style = document.getElementById('intcad-theme');
  if (!style) { style = document.createElement('style'); style.id = 'intcad-theme'; document.head.appendChild(style); }
  style.textContent = `
    @font-face { font-family:'Intcad Sans'; src:url('intcad-plex.ttf') format('truetype'); font-weight:100 700; font-display:block; }
    html, body, #root { background: transparent !important; }
    ::selection { background: ${c.selected}; color: ${c.selectedText}; }
    button, input, textarea, select { font-family: inherit; }
    /* Native controls are 28 pt tall (Tokens::control_height); scaled rems alone left the
       page's buttons at 21. A minimum, not a fixed height: larger controls keep their size. */
    /* Text buttons grow to the native control height; icon buttons (a button whose child is an
       SVG) keep their own square/circle size. */
    button:not(:has(svg)), input, select { min-height: ${m.controlHeight}px; }
  `;
  // A bounded receipt on the existing bridge, after all token/root changes.
  // The host checks u64 bounds and whether this is still its current request.
  if (typeof ticket === 'string' && ticket.length <= 41 && /^[0-9]+:[0-9]+$/.test(ticket)) {
    window.ipc.postMessage(JSON.stringify({type:'theme-ready', ticket}));
  }
};
"#;

/// The app's own note channel for the Assistant's web view.
fn note(text: &str) {
    eprintln!("solidcraft: assistant (CEF): {}", crate::assistant_cef_http::sanitized_diagnostic(text));
}

/// One `/ipc` log line keeps at most this many characters, so a page cannot flood the
/// log. Automated evidence reads these lines (scripts in `experiments/cef-osr`), so a
/// `SCOPE` probe message over the cap must not be cut mid-object: it is replaced by a
/// small, parseable `scope_oversize` receipt that names the action and the real length.
const IPC_LOG_TEXT_CAP: usize = 600;

fn bounded_log_text(text: &str) -> String {
    if text.chars().count() <= IPC_LOG_TEXT_CAP {
        return text.to_owned();
    }
    let action = text
        .strip_prefix("SCOPE ")
        .and_then(|rest| serde_json::from_str::<serde_json::Value>(rest).ok())
        .and_then(|value| value.get("action").and_then(serde_json::Value::as_str).map(str::to_owned));
    match action {
        Some(action) => format!("SCOPE {}", serde_json::json!({"action": "scope_oversize", "of": action, "len": text.chars().count()})),
        None => text.chars().take(IPC_LOG_TEXT_CAP).collect(),
    }
}

/// One bounded accelerated-import receipt. `false` comes from the real
/// no-DMA-BUF/refusal path; `true` only after an accelerated texture import was
/// admitted. Absence of either is never treated as success by the validator.
fn emit_accelerated_limit(accelerated: bool, reason: &str) {
    let reason: String = reason.chars().take(120).collect();
    note(&format!("CEF_LIMIT {}", serde_json::json!({"accelerated": accelerated, "reason": reason})));
}

/// One bounded frame receipt. `requested_scale` and `dip` are app placement
/// values; neither is an independently observed native getter.
impl CefAssistant {
    fn frame_receipt(
        &self,
        element: &str,
        metadata: Option<&CaptureMetadata>,
        physical: Option<[u32; 2]>,
        imported_size: Option<[u32; 2]>,
        imported_generation: Option<u64>,
        decision: &str,
        reason: &str,
    ) {
        let scale = self.placement.map(|placement| placement.scale);
        let dip = match (physical, scale) {
            (Some([width, height]), Some(scale)) if scale.is_finite() && scale > 0.0 => {
                let round = |value: u32| ((f64::from(value) / f64::from(scale)) * 1000.0).round() / 1000.0;
                serde_json::json!([round(width), round(height)])
            }
            _ => serde_json::Value::Null,
        };
        let rect = |rect: FrameRect| serde_json::json!([rect.x, rect.y, rect.width, rect.height]);
        let (coded, visible, content, source, generation, capture_counter) = match metadata {
            Some(metadata) => (
                serde_json::json!(metadata.geometry.coded_size),
                rect(metadata.geometry.visible_rect),
                rect(metadata.geometry.content_rect),
                serde_json::json!(metadata.geometry.source_size),
                serde_json::json!(metadata.generation),
                serde_json::json!(metadata.capture_counter),
            ),
            None => (
                serde_json::Value::Null,
                serde_json::Value::Null,
                serde_json::Value::Null,
                serde_json::Value::Null,
                serde_json::Value::Null,
                serde_json::Value::Null,
            ),
        };
        let revision = self
            .queue
            .ticket
            .map(|ticket| ticket.placement_revision)
            .or_else(|| self.admission.as_ref().map(admission::FrameAdmission::revision))
            .unwrap_or(0);
        let reason: String = reason.chars().take(120).collect();
        note(&format!(
            "CEF_FRAME {}",
            serde_json::json!({
                "lifetime": self.lifetime,
                "placement_revision": revision,
                "element": element,
                "generation": generation,
                "capture_counter": capture_counter,
                "requested_scale": scale,
                "dip": dip,
                "physical": physical,
                "coded": coded,
                "visible": visible,
                "content": content,
                "source": source,
                "imported_size": imported_size,
                "imported_generation": imported_generation,
                "decision": decision,
                "reason": reason,
            })
        ));
    }

    fn record_view_admitted(&mut self, metadata: &CaptureMetadata, physical: [u32; 2], imported_size: [u32; 2], imported_generation: u64) {
        let signature = (metadata.geometry.coded_size, self.placement.map(|placement| placement.scale.to_bits()).unwrap_or(0));
        self.last_frame_reason = None;
        if self.last_view_receipt != Some(signature) {
            self.last_view_receipt = Some(signature);
            self.frame_receipt("view", Some(metadata), Some(physical), Some(imported_size), Some(imported_generation), "admitted", "");
        }
    }

    fn record_frame_refusal(
        &mut self,
        element: &str,
        metadata: Option<&CaptureMetadata>,
        physical: Option<[u32; 2]>,
        imported_size: Option<[u32; 2]>,
        imported_generation: Option<u64>,
        reason: &str,
    ) {
        let bounded: String = reason.chars().take(120).collect();
        if self.last_frame_reason.as_deref() == Some(bounded.as_str()) {
            return;
        }
        self.last_frame_reason = Some(bounded.clone());
        self.frame_receipt(element, metadata, physical, imported_size, imported_generation, "refused", &bounded);
    }

    /// Fixed, in-source scope probe for tests. Enabled only by
    /// `SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE=1`; no URL/file/script parameter.
    pub fn inject_scope_probe(&mut self) -> Result<(), welding::WeldError> {
        if std::env::var("SOLIDCRAFT_ASSISTANT_CEF_TEST_SCOPE").ok().as_deref() != Some("1") {
            return Ok(());
        }
        self.producer.execute_script(SCOPE_PROBE_JS, "").map(|_| ())
    }
}

fn backend_label(backend: &config::HostBackend) -> &'static str {
    match backend {
        config::HostBackend::LinuxWayland => "LinuxWayland",
        config::HostBackend::LinuxX11 => "LinuxX11",
        config::HostBackend::Windows => "Windows",
        config::HostBackend::Macos => "Macos",
        config::HostBackend::Unknown => "Unknown",
    }
}

fn emit_runtime_receipt(policy: &config::EffectivePolicy) {
    let mut disabled = Vec::new();
    for switch in policy.switches.iter().filter(|switch| switch.name == "disable-features") {
        if let Some(value) = switch.value.as_deref() {
            for feature in value.split(',').filter(|feature| !feature.is_empty()) {
                if !disabled.iter().any(|seen: &String| seen == feature) {
                    disabled.push(feature.to_owned());
                }
            }
        }
    }
    let identity = policy.identity.as_ref();
    note(&format!(
        "CEF_RUNTIME {}",
        serde_json::json!({
            "backend": backend_label(&policy.backend),
            "osr_only": policy.osr_only,
            "disabled_features": disabled,
            "cef": identity.map(|identity| identity.cef),
            "chromium": identity.map(|identity| identity.chromium),
            "api_version": identity.map(|identity| identity.api_version),
            "cef_commit": identity.map(|identity| identity.cef_commit.as_str()),
            "platform_api_hash": identity.map(|identity| identity.platform_api_hash.as_str()),
        })
    ));
}

/// The page's IPC shim is served at `/ipc.js`; this probe only posts bounded
/// log messages to the existing `/ipc` endpoint.
const SCOPE_PROBE_JS: &str = r#"
(() => {
  if (window.__intcadScopeProbeInstalled) return 'already';
  window.__intcadScopeProbeInstalled = true;
  // One `/ipc` log line keeps 600 characters (IPC_LOG_TEXT_CAP in this file), and the
  // evidence scripts parse it. Every report is therefore its own small message with its
  // own ceiling, and an over-budget message becomes a bounded `scope_oversize` receipt
  // instead of being cut into unparseable JSON.
  const SCOPE_MAX_TEXT = 480;
  const SCOPE_BUDGET = { rects: 380, fonts: 160, tokens: 340, roles: 300, swatches: 460 };
  const send = (value, limit) => {
    const text = 'SCOPE ' + JSON.stringify(value);
    const cap = limit || SCOPE_MAX_TEXT;
    const out = text.length <= cap && text.length <= SCOPE_MAX_TEXT
      ? text
      : 'SCOPE ' + JSON.stringify({ action: 'scope_oversize', of: value.action, len: text.length });
    try {
      window.ipc.postMessage(JSON.stringify({ type: 'log', text: out }));
    } catch (_) {}
  };
  const root = document.createElement('div');
  root.id = '__cef_scope_root';
  root.style.cssText = 'position:fixed;right:8px;top:8px;z-index:2147483647;display:flex;gap:4px;align-items:center;padding:3px';
  const button = (id, label, action) => {
    const element = document.createElement('button');
    element.id = id; element.type = 'button'; element.textContent = label;
    element.style.cssText = 'font:12.5px system-ui;min-width:24px';
    element.addEventListener('click', () => { try { action(); } catch (error) { send({ id, error: String(error).slice(0, 80) }); } });
    root.appendChild(element);
    return element;
  };
  button('__cef_scope_open', 'open', () => send({ action: 'window_open', denied: window.open('about:blank', '_blank', 'noopener') === null }));
  const target = document.createElement('a');
  target.id = '__cef_scope_target'; target.href = 'about:blank'; target.target = '_blank'; target.textContent = 'blank';
  target.style.cssText = 'font:12.5px system-ui'; root.appendChild(target);
  const select = document.createElement('select');
  select.id = '__cef_scope_select';
  for (const value of ['alpha', 'beta']) { const option = document.createElement('option'); option.textContent = value; select.appendChild(option); }
  select.addEventListener('change', () => send({ action: 'select', value: select.value }));
  root.appendChild(select);
  button('__cef_scope_fullscreen', 'full', () => {
    if (document.fullscreenElement) { document.exitFullscreen(); }
    else { document.documentElement.requestFullscreen(); }
  });
  document.addEventListener('fullscreenchange', () => send({ action: 'fullscreen', active: Boolean(document.fullscreenElement) }));
  button('__cef_scope_dialog', 'dialog', () => send({ action: 'dialog_current', cancelled: confirm('cef-scope-dialog') === false }));
  button('__cef_scope_devtools', 'devtools', () => send({ action: 'devtools_entry', denied: window.open('devtools://devtools/bundled/inspector.html') === null }));
  const input = document.createElement('input');
  input.id = '__cef_scope_ime'; input.autocomplete = 'off'; input.placeholder = 'ime';
  input.style.cssText = 'font:12.5px system-ui;width:60px';
  root.appendChild(input);
  document.body.appendChild(root);
  // Fixed analytic test patches under the same opt-in hook. They are test fixtures,
  // not product style tokens: opaque midtones whose painted pixel moves far beyond the
  // evidence tolerance if the pipeline decodes sRGB twice.
  const SWATCHES = [
    { id: 'mid', css: '#808080' },
    { id: 'blue', css: '#336699' },
    { id: 'warm', css: '#e08040' }
  ];
  const strip = document.createElement('div');
  strip.id = '__cef_scope_swatches';
  strip.style.cssText = 'position:fixed;left:8px;top:8px;z-index:2147483647;display:flex;gap:4px;padding:2px;background:#000';
  const patches = SWATCHES.map((fixture) => {
    const patch = document.createElement('div');
    patch.id = '__cef_scope_swatch_' + fixture.id;
    patch.style.cssText = 'width:20px;height:20px;background:' + fixture.css;
    strip.appendChild(patch);
    return patch;
  });
  document.body.appendChild(strip);
  const rect = (element) => {
    const box = element.getBoundingClientRect();
    return { x: Math.round(box.x), y: Math.round(box.y), w: Math.round(box.width), h: Math.round(box.height) };
  };
  const computed = (element) => (element ? getComputedStyle(element) : null);
  const colour = (element, part) => {
    const style = computed(element);
    return style ? style[part] : null;
  };
  const report = () => {
    // Real semantic elements only. The probe does not add its own prose/label
    // element with a fixed font, because that would only measure itself. The
    // shipped page exposes prose, label, textarea and aria-live; its rail is
    // native. A page-level `#swatch` element exists only in the standalone
    // fixture, while the fixed analytic patches below are test instrumentation
    // drawn by this opt-in hook in both the app page and the fixture.
    const theme = document.documentElement.dataset.intcadTheme || null;
    const html = computed(document.documentElement);
    const body = computed(document.body);
    const textarea = document.querySelector('textarea');
    const label = document.querySelector('[class*="text-label"], label');
    const prose = document.querySelector('[class*="text-body"], p');
    const live = document.querySelector('[aria-live]');
    send({ action: 'rects', theme, dpr: devicePixelRatio, inner: [innerWidth, innerHeight],
      root: rect(root), open: rect(document.getElementById('__cef_scope_open')),
      target: rect(target), select: rect(select), ime: rect(input) }, SCOPE_BUDGET.rects);
    send({ action: 'fonts', theme,
      root: parseFloat(computed(document.documentElement).fontSize),
      prose: prose ? parseFloat(computed(prose).fontSize) : null,
      textarea: textarea ? parseFloat(computed(textarea).fontSize) : null,
      label: label ? parseFloat(computed(label).fontSize) : null }, SCOPE_BUDGET.fonts);
    send({ action: 'tokens', theme,
      html_background: html.backgroundColor,
      body_background: body.backgroundColor,
      tokens: {
        background: html.getPropertyValue('--background').trim(),
        background2: html.getPropertyValue('--background-2').trim(),
        border: html.getPropertyValue('--border').trim(),
        accent: html.getPropertyValue('--intcad-accent').trim(),
        selected: html.getPropertyValue('--intcad-selected').trim(),
        foreground: html.getPropertyValue('--foreground').trim()
      } }, SCOPE_BUDGET.tokens);
    send({ action: 'roles', theme,
      prose: colour(prose, 'color'),
      label: colour(label, 'color'),
      textarea: colour(textarea, 'color'),
      live: colour(live, 'color'),
      swatch: colour(document.getElementById('swatch'), 'backgroundColor') }, SCOPE_BUDGET.roles);
    send({ action: 'swatches', theme, inner: [innerWidth, innerHeight],
      patches: SWATCHES.map((fixture, index) => ({
        css: fixture.css,
        computed: colour(patches[index], 'backgroundColor'),
        rect: rect(patches[index])
      })) }, SCOPE_BUDGET.swatches);
  };
  report();
  addEventListener('resize', report);
  // A theme switch changes the page's computed styles without resizing it.
  let lastTheme = document.documentElement.dataset.intcadTheme || null;
  setInterval(() => {
    const current = document.documentElement.dataset.intcadTheme || null;
    if (current !== lastTheme) { lastTheme = current; report(); }
  }, 250);
  return 'installed';
})()
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_device_descriptor_retains_cad_features_and_limits() {
        let base = wgpu::DeviceDescriptor {
            label: Some("CAD-selected-device"),
            required_features: wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES | crate::startup::FLOAT_DEPTH_STENCIL,
            required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..Default::default() },
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        };
        let supported = base.required_features | wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
        let augmented = interop_descriptor(&base, wgpu::Backend::Vulkan, supported).unwrap();
        assert!(augmented.required_features.contains(base.required_features));
        assert_eq!(augmented.required_features, supported);
        assert_eq!(augmented.required_limits, base.required_limits);
        assert_eq!(augmented.label, base.label);
        assert_eq!(format!("{:?}", augmented.memory_hints), format!("{:?}", base.memory_hints));
        assert_eq!(format!("{:?}", augmented.experimental_features), format!("{:?}", base.experimental_features));
        assert_eq!(format!("{:?}", augmented.trace), format!("{:?}", base.trace));
        assert!(interop_descriptor(&base, wgpu::Backend::Gl, supported).is_none());
        assert!(interop_descriptor(&base, wgpu::Backend::Vulkan, base.required_features).is_none());
        assert!(!base.required_features.contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF));
    }

    #[test]
    fn early_interop_configuration_preserves_cad_and_never_replaces_foreign_factory() {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        let descriptor = setup.device_descriptor.clone();
        let backends = setup.instance_descriptor.backends;
        configure_interop(&mut setup, false).unwrap();
        assert!(setup.native_device_creator.is_none());
        configure_interop(&mut setup, true).unwrap();
        assert!(Arc::ptr_eq(&descriptor, &setup.device_descriptor));
        assert_eq!(setup.instance_descriptor.backends, backends);
        #[cfg(target_os = "linux")]
        {
            let installed = setup.native_device_creator.clone().unwrap();
            assert!(configure_interop(&mut setup, true).is_err());
            assert!(Arc::ptr_eq(setup.native_device_creator.as_ref().unwrap(), &installed));
        }
    }

    /// Real wgpu types and a native Vulkan adapter; needs the authorized app test
    /// build. A scalar bitmap or invented adapter is not a substitute.
    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "BLOCKED: requires authorized real Vulkan adapter/device execution"]
    fn unsupported_interop_uses_original_selected_adapter() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("an authorized native Vulkan adapter");
        let base = wgpu::DeviceDescriptor {
            label: Some("CAD-selected-device"),
            required_features: adapter.features() & crate::startup::FLOAT_DEPTH_STENCIL,
            required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..Default::default() },
            ..Default::default()
        };
        // Non-Vulkan backends never enter the interop helper.
        assert!(interop_descriptor(&base, wgpu::Backend::Gl, adapter.features()).is_none());
        // The original descriptor is copied, never mutated: the selected adapter
        // and base request remain the CAD path when interop is unsupported.
        let original = base.clone();
        for backend in [wgpu::Backend::Vulkan, wgpu::Backend::Gl] {
            let _ = interop_descriptor(&base, backend, adapter.features());
            assert_eq!(format!("{:?}", base.required_features), format!("{:?}", original.required_features));
            assert_eq!(base.required_limits, original.required_limits);
        }
        if !adapter.features().contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF) {
            assert!(interop_descriptor(&base, wgpu::Backend::Vulkan, adapter.features()).is_none());
            // The unchanged base descriptor still opens the ordinary CAD device.
            let (device, _queue) = pollster::block_on(adapter.request_device(&base))
                .expect("the ordinary CAD device request stays available");
            drop(device);
        }
    }

    /// The later display/device refusal must be a read-only policy decision.
    #[test]
    #[cfg(target_os = "linux")]
    fn later_display_handle_refusal_keeps_selected_cad_device() {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        configure_interop(&mut setup, true).unwrap();
        let descriptor = setup.device_descriptor.clone();
        let factory = setup.native_device_creator.clone();
        let backends = setup.instance_descriptor.backends;
        assert!(!interop_device_admissible(wgpu::Features::empty()));
        assert_eq!(
            interop_device_admissible(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF),
            NATIVE_INTEROP_CLEANUP_VERIFIED,
            "the cleanup/ABI gate alone decides after the feature exists"
        );
        // Refusal reached the same retained configuration; nothing re-selected
        // the adapter, replaced the descriptor or removed the installed factory.
        assert!(Arc::ptr_eq(&descriptor, &setup.device_descriptor));
        assert!(Arc::ptr_eq(factory.as_ref().expect("installed factory"), setup.native_device_creator.as_ref().expect("retained factory")));
        assert_eq!(backends, setup.instance_descriptor.backends);
    }

    /// Unproven cleanup must not open a native owner that CAD then cannot inherit.
    #[test]
    #[cfg(target_os = "linux")]
    #[ignore = "BLOCKED: requires authorized real Vulkan adapter plus native cleanup evidence"]
    fn interop_failure_releases_native_owners_before_cad_request() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .expect("an authorized native Vulkan adapter");
        let base = wgpu::DeviceDescriptor {
            label: Some("CAD-selected-device"),
            required_features: adapter.features() & crate::startup::FLOAT_DEPTH_STENCIL,
            required_limits: wgpu::Limits { max_texture_dimension_2d: 8192, ..Default::default() },
            ..Default::default()
        };
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        configure_interop(&mut setup, true).unwrap();
        let factory = setup.native_device_creator.clone().expect("installed factory");
        // With cleanup proof withdrawn the factory refuses before the DMA-BUF
        // helper, so no temporary native owner exists to release.
        let result = factory(&adapter, &base).expect("refusal is a clean fallback, not an error");
        assert!(result.is_none());
        // CAD then opens its ordinary device on the same adapter/descriptor.
        let (device, _queue) = pollster::block_on(adapter.request_device(&base))
            .expect("the ordinary CAD device request stays available");
        drop(device);
        // Native release of an actually opened owner is covered by the four
        // ignored HAL boundary tests; this test never claims that proof.
    }

    #[test]
    fn navigation_policy_rejects_consumed_permit_redirect_and_foreign_frame() {
        use welding::request_policy::{RequestCheck, RequestDecision};
        let authority = Arc::new(DocumentAuthority::new());
        let owner = authority.reserve_browser_owner().unwrap();
        let policy = private_request_policy(authority.clone(), owner, Arc::new(AtomicBool::new(false)));
        assert!(matches!(policy(RequestCheck::BrowserCreated { browser_id: 7 }), RequestDecision::Allow));
        let document = authority.begin_document(owner, [1; 32]).unwrap();
        let url = authority.bootstrap_url(document, 43123).unwrap();
        authority.arm_main_navigation(NavigationIdentity { document, owner, browser_id: 7, url: url.clone() }).unwrap();
        for (browser_id, main_frame, is_redirect) in [(8, true, false), (7, false, false), (7, true, true)] {
            assert!(matches!(policy(RequestCheck::BeforeBrowse { browser_id, main_frame, target_url: &url, is_redirect }), RequestDecision::Cancel));
        }
        assert!(!authority.permits(document), "refusals cannot consume the permit");
        assert!(matches!(policy(RequestCheck::BeforeBrowse { browser_id: 7, main_frame: true, target_url: &url, is_redirect: false }), RequestDecision::Allow));
        assert!(authority.permits(document));
        for target_url in ["https://outside.invalid/picture/key", "data:text/html,evil", "about:blank"] {
            assert!(matches!(policy(RequestCheck::Resource { browser_id: 7, target_url, is_image: false }), RequestDecision::Cancel));
        }
        let image = url.replace("index.html", "picture/key");
        assert!(matches!(policy(RequestCheck::Resource { browser_id: 7, target_url: &image, is_image: true }), RequestDecision::Allow));
        assert!(matches!(policy(RequestCheck::Redirect { browser_id: 7, target_url: "https://outside.invalid/index.html", is_image: false }), RequestDecision::Cancel));
    }

    #[test]
    fn blocked_reload_requests_host_renewal_without_rearming_old_document() {
        use welding::request_policy::{RequestCheck, RequestDecision};
        let authority = Arc::new(DocumentAuthority::new());
        let owner = authority.reserve_browser_owner().unwrap();
        authority.bind_browser(owner, 7).unwrap();
        let renewal = Arc::new(AtomicBool::new(false));
        let policy = private_request_policy(authority.clone(), owner, renewal.clone());
        let document = authority.begin_document(owner, [1; 32]).unwrap();
        let url = authority.bootstrap_url(document, 43123).unwrap();
        authority.arm_main_navigation(NavigationIdentity { document, owner, browser_id: 7, url: url.clone() }).unwrap();
        let browse = || RequestCheck::BeforeBrowse { browser_id: 7, main_frame: true, target_url: &url, is_redirect: false };
        assert!(matches!(policy(browse()), RequestDecision::Allow));
        assert!(matches!(policy(browse()), RequestDecision::RenewDocument));
        assert!(renewal.swap(false, Ordering::AcqRel));
        // The host consumes the bounded notification and revokes before entropy,
        // scripts or navigation; no callback is allowed to rearm this tuple.
        authority.revoke();
        assert!(!authority.permits(document));
        assert!(matches!(policy(browse()), RequestDecision::Cancel));
        assert!(!renewal.load(Ordering::Acquire));
        assert!(authority.begin_document(owner, [1; 32]).is_err());
        assert!(authority.begin_document(owner, [2; 32]).is_ok());
    }

    #[test]
    fn private_http_runtime_suppresses_browser_argv_and_native_logs() {
        // Real runtime types/configuration only. Does not load/initialize CEF.
        // Execution is gated with the app crate, never replaced by a fake type.
        for backend in [HostBackend::Windows, HostBackend::Macos, HostBackend::LinuxX11] {
            let policy = config::build_policy(backend, true, None, &[]).unwrap();
            let mut runtime = CefRuntimeConfig::new("unused-test-runtime", CefSandboxMode::Sandboxed);
            configure_private_runtime(&mut runtime, &policy);
            assert!(runtime.command_line_args_disabled);
            assert!(matches!(runtime.log_severity, welding::CefLogSeverity::Disable));
        }
    }

    // Real native event types. This test needs the actual app test build; the
    // std-only admission harness does not prove CEF cancels a drag on blur.
    #[test]
    fn reset_discards_queued_downs_motion_and_delayed_releases() {
        let mut queue = InputQueue::new(256);
        queue.ready = true;
        queue.ticket = Some(admission::InputTicket { lifetime: 1, placement_revision: 2 });
        let mouse = MouseEvent { x: 40, y: 50, button: MouseButton::Left, action: MouseAction::Pressed, modifiers: EventModifiers::default() };
        let key = KeyEvent {
            kind: KeyEventKind::RawKeyDown,
            windows_key_code: 17,
            native_key_code: 0,
            character: None,
            modifiers: EventModifiers { ctrl: true, ..Default::default() },
        };
        queue.push(Queued::Mouse(mouse.clone()));
        queue.push(Queued::Key(key.clone()));
        // The real flusher removes these events after sending them.
        queue.critical.clear();
        queue.push(Queued::Mouse(MouseEvent { action: MouseAction::Moved, ..mouse.clone() }));
        queue.invalidate();
        assert!(!queue.ready);
        assert!(queue.ticket.is_none());
        assert!(queue.motion.is_none());
        queue.ready = true;
        queue.ticket = Some(admission::InputTicket { lifetime: 1, placement_revision: 3 });
        queue.push(Queued::Mouse(MouseEvent { action: MouseAction::Released, ..mouse }));
        queue.push(Queued::Key(KeyEvent { kind: KeyEventKind::KeyUp, ..key }));
        assert!(queue.critical.is_empty(), "old releases cannot reach the new placement");
    }

    #[test]
    fn a_scope_log_line_is_never_cut_into_unparseable_json() {
        // The page's fixed hook splits its report; this is the app-side guarantee that a
        // `SCOPE` message over the `/ipc` log cap still arrives as one parseable receipt.
        let oversized = format!("SCOPE {}", serde_json::json!({"action": "rects", "pad": "x".repeat(IPC_LOG_TEXT_CAP)}));
        assert!(oversized.chars().count() > IPC_LOG_TEXT_CAP);
        let bounded = bounded_log_text(&oversized);
        assert!(bounded.chars().count() <= IPC_LOG_TEXT_CAP, "{bounded}");
        let Some(Ok(value)) = bounded.strip_prefix("SCOPE ").map(serde_json::from_str::<serde_json::Value>) else {
            panic!("the bounded SCOPE line must stay parseable JSON: {bounded}");
        };
        assert_eq!(value["action"], serde_json::json!("scope_oversize"));
        assert_eq!(value["of"], serde_json::json!("rects"));
        // Other page text keeps the previous bounded truncation.
        let plain = "x".repeat(IPC_LOG_TEXT_CAP + 10);
        assert_eq!(bounded_log_text(&plain).chars().count(), IPC_LOG_TEXT_CAP);
    }

    #[test]
    fn current_queue_remains_bounded_and_diagnostic_counters_saturate() {
        let mut queue = InputQueue::new(256);
        queue.held = u64::MAX;
        queue.refused = u64::MAX;
        for _ in 0..300 {
            queue.push(Queued::ImeCancel);
        }
        assert_eq!(queue.critical.len(), 256);
        assert_eq!(queue.held, u64::MAX);
        assert_eq!(queue.refused, u64::MAX);
        assert!(queue.overflowed);
    }
}
