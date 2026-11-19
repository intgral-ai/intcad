//! The Assistant's page as a CEF accelerated-OSR texture in the app's own window.
//!
//! Experimental replacement for the WRY web view (see `assistant_web.rs`): the page is
//! painted by CEF into a DMA-BUF, imported into the app's existing wgpu device, and drawn
//! with egui in the panel's rectangle. There is no child window and no GTK on Linux.
//!
//! Enabled with `SOLIDCRAFT_ASSISTANT_CEF=1` while the migration is under test. The
//! production path stays `assistant_web.rs` until that switch is removed in the cutover.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender, channel};
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
    HostWgpuContext, ImportedTexture, KeyEvent, KeyEventKind, MouseAction, MouseButton, MouseEvent, WebRequestId,
};

use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::agent_chat::web;

#[path = "assistant_cef_viewport.rs"]
mod viewport;

/// The page's origin while CEF runs: a loopback server this module owns, because CEF has no
/// custom-scheme handler here and the page's ES modules need a real origin.
const PAGE_HOST: &str = "127.0.0.1";

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

/// The device egui draws with **and** the DMA-BUF importer can take: on Linux the Vulkan device
/// has to be opened with `VK_EXT_queue_family_foreign`, which `adapter.request_device` cannot
/// add. `None` leaves eframe's own device creation alone.
pub fn interop_setup() -> Option<eframe::egui_wgpu::WgpuSetup> {
    // Only Linux shares memory through a device extension that has to be enabled at creation
    // (Vulkan DMA-BUF). Windows' D3D12 shared handles and macOS' IOSurfaces are imported on the
    // device eframe makes, so those platforms leave device creation alone.
    #[cfg(not(target_os = "linux"))]
    {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        // Test only: pretend the device cannot import DMA-BUF, to check what the app does when a
        // machine's GPU interop is missing. The Assistant must say so and CAD must keep working.
        if std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_NO_INTEROP").is_some() {
            note("CEF: interop disabled by SOLIDCRAFT_ASSISTANT_CEF_NO_INTEROP");
            return None;
        }
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::VULKAN;
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .ok()?;
        let features = adapter.features();
        if !features.contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF) {
            note("CEF: this Vulkan device cannot import DMA-BUF (no VULKAN_EXTERNAL_MEMORY_DMA_BUF)");
            return None;
        }
        let device_descriptor = wgpu::DeviceDescriptor {
            label: Some("intcad-cef-interop"),
            required_features: features & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF,
            ..Default::default()
        };
        let (device, queue) = welding::build_dmabuf_capable_device(&adapter, &device_descriptor).ok()?;
        Some(eframe::egui_wgpu::WgpuSetup::Existing(eframe::egui_wgpu::WgpuSetupExisting { instance, adapter, device, queue }))
    }
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
        Self { critical: VecDeque::new(), motion: None, capacity, ready: false, held: 0, refused: 0 }
    }

    fn push(&mut self, event: Queued) {
        if let Queued::Mouse(MouseEvent { action: MouseAction::Moved, x, y, .. }) = event {
            self.motion = Some((x, y));
            if !self.ready {
                self.held += 1;
            }
            return;
        }
        if self.critical.len() >= self.capacity {
            self.refused += 1;
            return;
        }
        if !self.ready {
            self.held += 1;
        }
        self.critical.push_back(event);
    }

    fn clear(&mut self) {
        self.critical.clear();
        self.motion = None;
    }
}

/// One picture the page may show, served off the app's own state.
type Pictures = Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>;

pub struct CefAssistant {
    runtime: CefRuntime,
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
    pictures: Pictures,
    inbox: Receiver<String>,
    port: u16,
    copy_request: Option<u64>,
    last_frame: Instant,
    failed: bool,
    ticks: u64,
    pushes: u64,
    frames: u64,
    last_stats: Instant,
    /// Test-only: a native `<select>` was added to the real page to exercise the popup path.
    test_select: bool,
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
    pub fn new(render_state: &eframe::egui_wgpu::RenderState) -> Option<Self> {
        let cef_path = std::env::var("CEF_PATH").ok()?;
        let mut config = CefRuntimeConfig::new(&cef_path, sandbox());
        config.cache_path = Some(crate::settings_dir().ok().flatten().unwrap_or_else(std::env::temp_dir).join("cef-assistant"));
        config.command_line_switches =
            vec![("no-first-run".into(), None), ("no-default-browser-check".into(), None), ("disable-features".into(), Some("Translate".into()))];
        if let Some(list) = std::env::var("SOLIDCRAFT_ASSISTANT_CEF_SWITCHES").ok() {
            config.command_line_switches.extend(list.split(',').filter(|part| !part.is_empty()).map(|part| match part.split_once('=') {
                Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
                None => (part.to_owned(), None),
            }));
        }
        let runtime = match CefRuntime::initialize(config) {
            Ok(runtime) => runtime,
            Err(error) => {
                note(&format!("CEF initialize failed: {error}"));
                return None;
            }
        };

        let pictures: Pictures = Arc::new(Mutex::new(HashMap::new()));
        let (sender, inbox) = channel();
        let port = match serve_pages(pictures.clone(), sender) {
            Ok(port) => port,
            Err(error) => {
                note(&format!("CEF: the page server could not start: {error}"));
                return None;
            }
        };
        let host = HostWgpuContext::new(render_state.device.clone(), render_state.queue.clone());
        #[cfg(target_os = "linux")]
        if !host.device.features().contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF) {
            note("CEF: the app's device has no VULKAN_EXTERNAL_MEMORY_DMA_BUF; the page cannot be shown");
            return None;
        }
        let url = format!("http://{PAGE_HOST}:{port}/index.html");
        let surface = CefSurfaceConfig {
            initial_url: url,
            initial_size: dpi::PhysicalSize::new(900, 600),
            scale_factor: 1.0,
            prefer_accelerated: true,
            // The page's own CSS is see-through outside its rounded surface: CEF must paint
            // transparent there, or every transparent pixel becomes its default white and the
            // panel's corners show as white squares.
            background_color: None,
            // Internal CEF protocol messages configure the raster scale; no remote debugging
            // listener or DevTools window is opened.
            devtools_protocol: true,
            ..Default::default()
        };
        let producer = match create_producer(&runtime, surface, &host) {
            Ok(producer) => producer,
            Err(error) => {
                note(&format!("CEF: the page could not be created: {error}"));
                return None;
            }
        };
        Some(Self {
            runtime,
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
            pictures,
            inbox,
            port,
            copy_request: None,
            last_frame: Instant::now(),
            failed: false,
            ticks: 0,
            pushes: 0,
            frames: 0,
            last_stats: Instant::now(),
            test_select: false,
            ime_on: false,
            started: Instant::now(),
            test_reloaded: false,
        })
    }

    /// The page's messages and the app's state, whether or not the window is painted.
    pub fn logic(&mut self, solve: &mut SolveApp, ctx: &Context) {
        self.pump();
        while let Ok(message) = self.inbox.try_recv() {
            self.receive(solve, ctx, &message);
        }
        // The conversation goes on while the window is behind another one.
        web::advance(solve, ctx);
        if let Some(text) = COPY_OUT.with(|slot| slot.borrow_mut().take()) {
            ctx.copy_text(text);
        }
    }

    /// What the page says: the same contract as the WRY host (`assistant_web::receive`).
    fn receive(&mut self, solve: &mut SolveApp, ctx: &Context, message: &str) {
        let value = serde_json::from_str::<serde_json::Value>(message).unwrap_or_default();
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("ready") => {
                // A reloaded page has neither the theme function nor its CSS variables: put the
                // bridge back and send the theme again, not only when the tokens change.
                let _ = self.producer.execute_script(THEME_BRIDGE_JS, "");
                self.told.clear();
                self.told_theme.clear();
                self.viewport.reset(Instant::now());
            }
            Some("focus") => self.focused = value.get("on").and_then(serde_json::Value::as_bool).unwrap_or(false),
            Some("log") => note(&value.get("text").and_then(serde_json::Value::as_str).unwrap_or_default().chars().take(600).collect::<String>()),
            _ => {
                if let Err(reason) = web::act(solve, ctx, message) {
                    note(&format!("CEF: refused: {reason}"));
                }
            }
        }
    }

    /// Once a frame, after the app's UI: place, resize, take the texture, draw it.
    pub fn tick(&mut self, solve: &mut SolveApp, ctx: &Context, frame: &mut eframe::Frame) {
        if web::take_retry(solve) {
            self.told.clear();
        }
        let Some(rect) = web::placement(solve, ctx) else {
            if self.shown {
                self.shown = false;
                let _ = self.producer.set_visible(false);
                self.queue.clear();
            }
            return;
        };
        // Align texel centres with the window framebuffer, not a fractional panel origin.
        let ppp = ctx.pixels_per_point();
        let rect = egui::Rect::from_min_max(
            egui::pos2((rect.min.x * ppp).round() / ppp, (rect.min.y * ppp).round() / ppp),
            egui::pos2((rect.max.x * ppp).round() / ppp, (rect.max.y * ppp).round() / ppp),
        );
        if !self.shown {
            self.shown = true;
            let _ = self.producer.set_visible(true);
        }
        let physical = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
        let target = (physical, ppp);
        if self.viewport.needs_update(target, Instant::now()) {
            let id = self.viewport.begin(target, Instant::now());
            // welding's native screen-info notification updates DIP layout, but CEF Linux
            // can keep its initial DPR=1. That produced a 370px raster stretched to 463px.
            // Configure Chromium's actual viewport and raster scale together. This changes
            // device pixels, not CSS zoom, page scale or the native input conversion.
            let message = serde_json::json!({
                "id": id,
                "method": "Emulation.setDeviceMetricsOverride",
                "params": {
                    "width": (physical[0] as f32 / ppp).round().max(1.0) as u32,
                    "height": (physical[1] as f32 / ppp).round().max(1.0) as u32,
                    "deviceScaleFactor": ppp,
                    "mobile": false,
                }
            });
            let result = self
                .producer
                .resize(dpi::PhysicalSize::new(physical[0], physical[1]))
                .and_then(|()| self.producer.set_scale_factor(ppp))
                .and_then(|()| self.producer.send_devtools_message(&message.to_string()));
            if let Err(error) = result {
                self.viewport.failed(Instant::now());
                note(&format!("CEF: viewport configuration failed: {error}"));
            }
            note(&format!("CEF: viewport physical={}x{} scale={ppp}", physical[0], physical[1]));
        }
        self.ticks += 1;
        if self.last_stats.elapsed() >= Duration::from_secs(2) {
            note(&format!("stats over 2 s: ticks={} state_pushes={} cef_frames={}", self.ticks, self.pushes, self.frames));
            self.ticks = 0;
            self.pushes = 0;
            self.frames = 0;
            self.last_stats = Instant::now();
        }
        self.pump();
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
        if self.texture.is_some() {
            let theme = serde_json::from_str::<serde_json::Value>(&state)
                .ok()
                .and_then(|value| value.get("theme").cloned())
                .map(|theme| theme.to_string())
                .unwrap_or_default();
            if !theme.is_empty() && theme != self.told_theme {
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
                if self.producer.execute_script(&format!("window.__intcadTheme&&window.__intcadTheme({theme})"), "").is_ok() {
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
                             fetch('/ipc', { method:'POST', body: JSON.stringify({ type:'log', text: 'DOM ' + JSON.stringify({ font: cs.fontSize, iw: innerWidth, ih: innerHeight, dpr: devicePixelRatio, buttons, inputs, composer }) }) }); })()",
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
                self.pushes += 1;
            }
        }

        if let Some(native) = self.producer.acquire_native_frame() {
            match welding::WgpuTextureImporter::import(native, &self.host) {
                Ok(texture) => self.publish(render_state(frame), texture),
                Err(error) => {
                    note(&format!("CEF: the page's frame could not be imported: {error}"));
                    self.failed = true;
                }
            }
        }
        self.popup = self.take_popup(frame);

        if let Some((id, size)) = self.texture {
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("assistant-cef-page")));
            // One imported texel per framebuffer pixel. A stale resize frame is clipped,
            // never stretched; the native themed frame backs any newly exposed area.
            let draw = egui::Rect::from_min_size(rect.min, egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp));
            painter.with_clip_rect(rect).image(id, draw, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            if let Some((popup, at, size)) = self.popup {
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
        if !self.test_select && self.texture.is_some() && std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_PROBE_SWATCH").is_some() {
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
        if !self.test_select && self.texture.is_some() && std::env::var_os("SOLIDCRAFT_ASSISTANT_CEF_TEST_SELECT").is_some() {
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
        self.set_ime(ctx, self.focused);
        // Test only: a real browser reload, to prove the theme survives one.
        if !self.test_reloaded
            && self.texture.is_some()
            && let Some(after) = std::env::var("SOLIDCRAFT_ASSISTANT_CEF_TEST_RELOAD_AFTER").ok().and_then(|value| value.parse::<f32>().ok())
            && self.started.elapsed().as_secs_f32() >= after
        {
            self.test_reloaded = true;
            note("CEF: test reload");
            let _ = self.producer.reload();
        }
        self.input(solve, ctx, rect, ppp);
        // The page paints on change; the host only has to ask soon while it is working.
        let working = self.last_frame.elapsed() < Duration::from_millis(400);
        ctx.request_repaint_after(if working { Duration::from_millis(16) } else { Duration::from_millis(200) });
    }

    pub fn close(&mut self) {
        let _ = self.producer.close();
    }

    pub fn focused(&self) -> bool {
        self.focused && self.shown
    }

    pub fn edit_text(&mut self, redo: bool) {
        let _ = self.producer.execute_script(if redo { "document.execCommand('redo')" } else { "document.execCommand('undo')" }, "");
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
        self.frames += 1;
        if !self.queue.ready {
            self.queue.ready = true;
            // One small function the host calls with the tokens; it owns the CSS variables the
            // Mastra components read, and injects the font and the transparent roots.
            let _ = self.producer.execute_script(THEME_BRIDGE_JS, "");
            self.told_theme.clear();
            note(&format!("CEF: first frame; replaying {} held input event(s)", self.queue.held));
        }
        self.last_frame = Instant::now();
    }

    fn take_popup(&mut self, frame: &eframe::Frame) -> Option<(egui::TextureId, [i32; 2], [u32; 2])> {
        let render_state = frame.wgpu_render_state()?;
        match self.producer.acquire_popup(&self.host) {
            Ok(Some(popup)) => {
                let size = [popup.texture.size.width, popup.texture.size.height];
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
                    if let Some((id, _, _)) = self.popup.take() {
                        render_state.renderer.write().free_texture(&id);
                    }
                }
                self.popup
            }
            Err(_) => self.popup,
        }
    }

    fn pump(&mut self) {
        self.runtime.do_message_loop_work();
        // Keep the bounded protocol queue drained, including viewport failure receipts.
        while let Some(message) = self.producer.poll_devtools_message() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&message)
                && let Some(id) = value.get("id").and_then(serde_json::Value::as_u64).and_then(|id| u32::try_from(id).ok())
            {
                let success = value.get("error").is_none() && value.get("result").is_some();
                if self.viewport.complete(id, success, Instant::now())
                    && let Some(error) = value.get("error")
                {
                    note(&format!("CEF: viewport protocol error: {error}"));
                }
            }
        }
        while let Some(event) = self.producer.poll_web_event() {
            if let CefSurfaceEvent::Navigation(navigation) = &event {
                let text = format!("{navigation:?}");
                if text.contains("LoadEnd") || text.contains("LoadError") || text.contains("TitleChanged") || text.contains("ConsoleMessage") {
                    note(&text.chars().take(200).collect::<String>());
                }
            }
            if let CefSurfaceEvent::ScriptCompleted { id, result } = &event {
                if self.copy_request == Some(id.get()) {
                    self.copy_request = None;
                    if let Ok(json) = result {
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
                        self.focused = false;
                        self.set_ime(ctx, false);
                        self.queue.clear();
                        continue;
                    }
                    let cef_button = match button {
                        egui::PointerButton::Primary => MouseButton::Left,
                        egui::PointerButton::Secondary => MouseButton::Right,
                        egui::PointerButton::Middle => MouseButton::Middle,
                        _ => continue,
                    };
                    if pressed {
                        self.focused = true;
                        self.set_ime(ctx, true);
                        let _ = self.producer.move_focus(FocusDirection::Forward);
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
                        self.queue.clear();
                        self.focused = false;
                        continue;
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
                egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                    if !text.is_empty() {
                        note(&format!("IME composition {}", text.chars().take(40).collect::<String>()));
                    }
                    if text.is_empty() {
                        self.queue.push(Queued::ImeCancel);
                    } else {
                        let range = active_range_chars.unwrap_or(0..text.chars().count());
                        self.queue.push(Queued::ImeSet(text, (range.start as u32, range.end as u32)));
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    note(&format!("IME commit {}", text.chars().take(40).collect::<String>()));
                    self.queue.push(Queued::ImeCommit(text));
                }
                _ => {}
            }
        }
        let _ = solve;
        self.flush();
    }

    fn set_ime(&mut self, ctx: &Context, on: bool) {
        if self.ime_on != on {
            self.ime_on = on;
            ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(on));
        }
    }

    fn request_copy(&mut self) {
        self.copy_request = Some(7);
        let _ = self.producer.request_script_result(
            WebRequestId::new(7),
            "(() => { const el = document.activeElement; \
             if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) { \
               return el.value.substring(el.selectionStart, el.selectionEnd); } \
             return window.getSelection().toString(); })()",
        );
    }

    fn flush(&mut self) {
        if !self.queue.ready {
            return;
        }
        if let Some((x, y)) = self.queue.motion.take() {
            let _ = self.producer.send_mouse_input(MouseEvent {
                x,
                y,
                button: MouseButton::Left,
                action: MouseAction::Moved,
                modifiers: EventModifiers::default(),
            });
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

/// Serve the embedded page over loopback: CEF has no custom-scheme API in welding, and the
/// page's ES modules need a real origin. `POST /ipc` is `window.ipc.postMessage`.
fn serve_pages(pictures: Pictures, inbox: Sender<String>) -> std::io::Result<u16> {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind((PAGE_HOST, 0))?;
    let port = listener.local_addr()?.port();
    note(&format!("page server on http://{PAGE_HOST}:{port}/index.html"));
    std::thread::Builder::new()
        .name("assistant-cef-page".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(stream) => stream,
                    Err(_) => continue,
                });
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                if length > 0 {
                    use std::io::Read;
                    if reader.read_exact(&mut body).is_err() {
                        continue;
                    }
                }
                let mut parts = request.split_whitespace();
                let method = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("/");
                let mut policy_override: Option<&str> = None;
                let (status, kind, policy, payload) = if method == "POST" && path == "/ipc" {
                    let message = String::from_utf8_lossy(&body).into_owned();
                    eprintln!(
                        "solidcraft: assistant (CEF): page said {}",
                        message.chars().take(160).collect::<String>()
                    );
                    let _ = inbox.send(message);
                    (200u16, "text/plain; charset=utf-8", "default-src 'none'", b"ok".to_vec())
                } else if let Some(key) = path.strip_prefix("/picture/") {
                    match pictures.lock().ok().and_then(|held| held.get(key).cloned()) {
                        Some(bytes) => {
                            let kind = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                                "image/png"
                            } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                                "image/jpeg"
                            } else if bytes.starts_with(b"RIFF") {
                                "image/webp"
                            } else {
                                "application/octet-stream"
                            };
                            (200, kind, "default-src 'none'; sandbox", bytes.to_vec())
                        }
                        None => (404, "text/plain", "default-src 'none'", b"not found".to_vec()),
                    }
                } else if path == "/intcad-plex.ttf" {
                    (
                        200,
                        "font/ttf",
                        "default-src 'none'",
                        include_bytes!("../../../assets/fonts/ibm-plex-sans/IBMPlexSans-Variable.ttf").to_vec(),
                    )
                } else if path == "/ipc.js" {
                    (200, "text/javascript; charset=utf-8", "default-src 'none'", IPC_SHIM.as_bytes().to_vec())
                } else {
                    let name = path.trim_start_matches('/');
                    match crate::assistant_page::page_file(if name.is_empty() { "index.html" } else { name }) {
                        Some((kind, bytes)) => {
                            if name.is_empty() || name == "index.html" {
                                // The page asks its own host for `POST /ipc` (the ipc shim);
                                // the shipped policy forbids every connection.
                                policy_override = Some("default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; img-src 'self' data: blob:; connect-src 'self'; base-uri 'none'; form-action 'none'");
                            }
                            (200, kind, crate::assistant_page::page_policy(), bytes.to_vec())
                        }
                        None => (404, "text/plain", "default-src 'none'", b"not found".to_vec()),
                    }
                };
                let payload = if path == "/" || path == "/index.html" {
                    index_with_bridge(&payload)
                } else {
                    payload
                };
                let policy = policy_override.unwrap_or(policy);
                let head = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: {kind}\r\nContent-Security-Policy: {policy}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&payload);
            }
        })?;
    Ok(port)
}

/// The page-side half of the theme bridge. The host passes the tokens it is drawing with;
/// this function turns them into the CSS variables Mastra's components already use. It is
/// injected once, on the first frame, and survives a theme change without a reload.
const THEME_BRIDGE_JS: &str = r#"
window.__intcadTheme = (t) => {
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
    @font-face { font-family:'Intcad Sans'; src:url('/intcad-plex.ttf') format('truetype'); font-weight:100 700; font-display:block; }
    html, body, #root { background: transparent !important; }
    ::selection { background: ${c.selected}; color: ${c.selectedText}; }
    button, input, textarea, select { font-family: inherit; }
    /* Native controls are 28 pt tall (Tokens::control_height); scaled rems alone left the
       page's buttons at 21. A minimum, not a fixed height: larger controls keep their size. */
    /* Text buttons grow to the native control height; icon buttons (a button whose child is an
       SVG) keep their own square/circle size. */
    button:not(:has(svg)), input, select { min-height: ${m.controlHeight}px; }
  `;
};
"#;

/// The page's IPC shim, served as `/ipc.js` so the policy's `script-src 'self'` admits it.
const IPC_SHIM: &str = "window.ipc = { postMessage(message) { fetch('/ipc', { method: 'POST', body: message }); } };\n";

/// The page's HTML with the IPC shim and a CSP that allows it. The shim defines the one call
/// the page makes and nothing else; the bundle is untouched.
fn index_with_bridge(html: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(html);
    // A file of the page's own origin: an inline script would be refused by the policy.
    let shim = r#"<script src="/ipc.js"></script>"#;
    let text = text.replace("connect-src 'none'", "connect-src 'self'");
    let text = if let Some(at) = text.find("<script type=\"module\"") {
        format!("{}{}{}", &text[..at], shim, &text[at..])
    } else {
        text.replace("<body>", &format!("<body>{shim}"))
    };
    text.into_bytes()
}

/// The app's own note channel for the Assistant's web view.
fn note(text: &str) {
    eprintln!("solidcraft: assistant (CEF): {text}");
}
