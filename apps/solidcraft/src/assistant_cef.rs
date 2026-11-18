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
use welding::linux_cef::{LinuxCefConfig, LinuxCefProducer};
use welding::wgpu;
use welding::{
    CefRuntime, CefRuntimeConfig, CefSandboxMode, CefSurfaceConfig, CefSurfaceEvent,
    CefSurfaceProducer, EventModifiers, FocusDirection, HostWgpuContext, ImportedTexture, KeyEvent,
    KeyEventKind, MouseAction, MouseButton, MouseEvent, WebRequestId,
};

use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::agent_chat::web;

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
    match std::env::var("SOLIDCRAFT_ASSISTANT_CEF_SANDBOX").ok().as_deref() {
        Some("off") => CefSandboxMode::UnsandboxedTrustedContent,
        _ => CefSandboxMode::Sandboxed,
    }
}

/// The device egui draws with **and** the DMA-BUF importer can take: on Linux the Vulkan device
/// has to be opened with `VK_EXT_queue_family_foreign`, which `adapter.request_device` cannot
/// add. `None` leaves eframe's own device creation alone.
pub fn interop_setup() -> Option<eframe::egui_wgpu::WgpuSetup> {
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
    Some(eframe::egui_wgpu::WgpuSetup::Existing(
        eframe::egui_wgpu::WgpuSetupExisting {
            instance,
            adapter,
            device,
            queue,
        },
    ))
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
        Self {
            critical: VecDeque::new(),
            motion: None,
            capacity,
            ready: false,
            held: 0,
            refused: 0,
        }
    }

    fn push(&mut self, event: Queued) {
        if let Queued::Mouse(MouseEvent {
            action: MouseAction::Moved,
            x,
            y,
            ..
        }) = event
        {
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
    producer: LinuxCefProducer,
    host: HostWgpuContext,
    queue: InputQueue,
    texture: Option<(egui::TextureId, [u32; 2])>,
    popup: Option<egui::TextureId>,
    told: String,
    focused: bool,
    shown: bool,
    placed: Option<[u32; 2]>,
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
}

impl CefAssistant {
    /// Make the browser and its page. `None` when CEF cannot start: the panel then says why
    /// instead of drawing a second conversation.
    pub fn new(render_state: &eframe::egui_wgpu::RenderState) -> Option<Self> {
        let cef_path = std::env::var("CEF_PATH").ok()?;
        let mut config = CefRuntimeConfig::new(&cef_path, sandbox());
        config.cache_path = Some(crate::settings_dir().ok().flatten().unwrap_or_else(std::env::temp_dir).join("cef-assistant"));
        config.command_line_switches = vec![
            ("no-first-run".into(), None),
            ("no-default-browser-check".into(), None),
            ("disable-features".into(), Some("Translate".into())),
        ];
        if let Some(list) = std::env::var("SOLIDCRAFT_ASSISTANT_CEF_SWITCHES").ok() {
            config.command_line_switches.extend(
                list.split(',')
                    .filter(|part| !part.is_empty())
                    .map(|part| match part.split_once('=') {
                        Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
                        None => (part.to_owned(), None),
                    }),
            );
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
        if !host
            .device
            .features()
            .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
        {
            note("CEF: the app's device has no VULKAN_EXTERNAL_MEMORY_DMA_BUF; the page cannot be shown");
            return None;
        }
        let url = format!("http://{PAGE_HOST}:{port}/index.html");
        let producer = match LinuxCefProducer::new(
            &runtime,
            LinuxCefConfig {
                surface: CefSurfaceConfig {
                    initial_url: url,
                    initial_size: dpi::PhysicalSize::new(900, 600),
                    scale_factor: 1.0,
                    prefer_accelerated: true,
                    ..Default::default()
                },
            },
        ) {
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
            popup: None,
            told: String::new(),
            focused: false,
            shown: false,
            placed: None,
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
            Some("ready") => self.told.clear(),
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
        if !self.shown {
            self.shown = true;
            let _ = self.producer.set_visible(true);
        }
        let ppp = ctx.pixels_per_point();
        let physical = [
            (rect.width() * ppp).round().max(1.0) as u32,
            (rect.height() * ppp).round().max(1.0) as u32,
        ];
        if self.placed != Some(physical) {
            self.placed = Some(physical);
            let _ = self.producer.resize(dpi::PhysicalSize::new(physical[0], physical[1]));
            let _ = self.producer.set_scale_factor(ppp);
        }
        self.ticks += 1;
        if self.last_stats.elapsed() >= Duration::from_secs(2) {
            note(&format!(
                "stats over 2 s: ticks={} state_pushes={} cef_frames={}",
                self.ticks, self.pushes, self.frames
            ));
            self.ticks = 0;
            self.pushes = 0;
            self.frames = 0;
            self.last_stats = Instant::now();
        }
        self.pump();
        // State is pushed with the app's own logic; the page is told when it changed.
        let state = web::state(solve);
        if state != self.told {
            if let Ok(mut held) = self.pictures.lock() {
                *held = web::pictures(solve).into_iter().collect();
            }
            if self.producer.execute_script(
                &format!("window.__assistantHost&&window.__assistantHost.state({state})"),
                "",
            ).is_ok() {
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

        if let Some((id, _size)) = self.texture {
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("assistant-cef-page"),
            ));
            // The panel's rectangle, not the last texture's size: a frame painted before a
            // resize arrives a frame later, and drawing it at its own size left the panel's
            // edge black on a fractional-scale Wayland window.
            let draw = rect;
            painter.image(
                id,
                draw,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            if let Some(popup) = self.popup {
                let popup_rect = egui::Rect::from_min_size(
                    draw.min + egui::vec2(80.0, 80.0),
                    egui::vec2(320.0, 200.0),
                );
                painter.image(
                    popup,
                    popup_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }

        self.input(solve, ctx, rect, ppp);
        // The page paints on change; the host only has to ask soon while it is working.
        let working = self.last_frame.elapsed() < Duration::from_millis(400);
        ctx.request_repaint_after(if working {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(200)
        });
    }

    pub fn close(&mut self) {
        let _ = self.producer.close();
    }

    pub fn focused(&self) -> bool {
        self.focused && self.shown
    }

    pub fn edit_text(&mut self, redo: bool) {
        let _ = self.producer.execute_script(
            if redo { "document.execCommand('redo')" } else { "document.execCommand('undo')" },
            "",
        );
    }

    fn publish(
        &mut self,
        render_state: Option<&eframe::egui_wgpu::RenderState>,
        frame: ImportedTexture,
    ) {
        let Some(render_state) = render_state else { return };
        let size = [frame.size.width, frame.size.height];
        let mut renderer = render_state.renderer.write();
        if let Some((id, _)) = self.texture.take() {
            renderer.free_texture(&id);
        }
        let id = renderer.register_native_texture(
            &self.host.device,
            &frame.view,
            wgpu::FilterMode::Linear,
        );
        drop(renderer);
        self.frames += 1;
        self.texture = Some((id, size));
        if !self.queue.ready {
            self.queue.ready = true;
            note(&format!(
                "CEF: first frame; replaying {} held input event(s)",
                self.queue.held
            ));
        }
        self.last_frame = Instant::now();
    }

    fn take_popup(&mut self, frame: &eframe::Frame) -> Option<egui::TextureId> {
        let render_state = frame.wgpu_render_state()?;
        match self.producer.acquire_popup(&self.host) {
            Ok(Some(popup)) => {
                let mut renderer = render_state.renderer.write();
                if let Some(id) = self.popup.take() {
                    renderer.free_texture(&id);
                }
                let id = renderer.register_native_texture(
                    &self.host.device,
                    &popup.texture.view,
                    wgpu::FilterMode::Linear,
                );
                Some(id)
            }
            Ok(None) => {
                if self.producer.popup_rect().is_none() {
                    if let Some(id) = self.popup.take() {
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
                        action: MouseAction::WheelScrolled {
                            delta_x: (delta.x * scale) as i32,
                            delta_y: (delta.y * scale) as i32,
                        },
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
                    if text.is_empty() {
                        self.queue.push(Queued::ImeCancel);
                    } else {
                        let range = active_range_chars.unwrap_or(0..text.chars().count());
                        self.queue.push(Queued::ImeSet(text, (range.start as u32, range.end as u32)));
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    self.queue.push(Queued::ImeCommit(text));
                }
                _ => {}
            }
        }
        let _ = solve;
        self.flush();
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
        K::A => 0x41, K::B => 0x42, K::C => 0x43, K::D => 0x44, K::E => 0x45,
        K::F => 0x46, K::G => 0x47, K::H => 0x48, K::I => 0x49, K::J => 0x4A,
        K::K => 0x4B, K::L => 0x4C, K::M => 0x4D, K::N => 0x4E, K::O => 0x4F,
        K::P => 0x50, K::Q => 0x51, K::R => 0x52, K::S => 0x53, K::T => 0x54,
        K::U => 0x55, K::V => 0x56, K::W => 0x57, K::X => 0x58, K::Y => 0x59,
        K::Z => 0x5A,
        K::Num0 => 0x30, K::Num1 => 0x31, K::Num2 => 0x32, K::Num3 => 0x33,
        K::Num4 => 0x34, K::Num5 => 0x35, K::Num6 => 0x36, K::Num7 => 0x37,
        K::Num8 => 0x38, K::Num9 => 0x39,
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
