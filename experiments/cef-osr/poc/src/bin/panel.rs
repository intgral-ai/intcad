//! Phase 3 harness: the CEF page drawn as an egui texture inside one native
//! window, with no child window, no GTK and no X11 window of its own.
//!
//! This is deliberately a *minimal host*, not IntCAD: it uses the same
//! generations the app uses (`eframe`/`egui` 0.37, wgpu 30) so the integration
//! question ("can the production device take CEF's texture?") is answered by
//! the same APIs the app already calls.
//!
//! Env: CEF_PATH, PANEL_URL, PANEL_SANDBOX, PANEL_SWITCHES (see probe.rs).
//! Keys: click the page to give it focus; Esc returns focus to egui.

use cef_osr_poc::queue::{char_event, Input, InputQueue};
use std::time::{Duration, Instant};

use eframe::egui;
use welding::linux_cef::{LinuxCefConfig, LinuxCefProducer};
use welding::wgpu;
use welding::{
    CefRuntime, CefRuntimeConfig, CefSandboxMode, CefSurfaceConfig, CefSurfaceEvent, CefSurfaceProducer, DragEventKind, DragInput, DragOperations,
    DragPayload, EventModifiers, FocusDirection, HostWgpuContext, ImportedTexture, KeyEvent, KeyEventKind, MouseAction, MouseButton, MouseEvent,
    WebRequestId,
};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

struct PanelApp {
    runtime: CefRuntime,
    producer: LinuxCefProducer,
    host: HostWgpuContext,
    render_state: eframe::egui_wgpu::RenderState,
    texture: Option<(egui::TextureId, [u32; 2])>,
    popup: Option<(egui::TextureId, egui::Rect)>,
    placed: Option<[u32; 2]>,
    focused: bool,
    left_down: bool,
    queue: InputQueue,
    copy_request: Option<u64>,
    hidden: bool,
    frames: u64,
    repaints: u64,
    last_frame: Instant,
    last_report: Instant,
    frame_times: Vec<f64>,
    events: Vec<String>,
    ppp: f32,
    drag_paths: Vec<String>,
}

impl PanelApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let cef_path = env("CEF_PATH")?;
        let render_state = cc.wgpu_render_state.as_ref()?;
        let sandbox = match env("PANEL_SANDBOX").as_deref() {
            Some("sandboxed") => CefSandboxMode::Sandboxed,
            _ => CefSandboxMode::UnsandboxedTrustedContent,
        };
        let mut config = CefRuntimeConfig::new(&cef_path, sandbox);
        config.cache_path = Some(env("PANEL_CACHE").map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("cef-osr-panel-cache")));
        config.command_line_switches = vec![("no-first-run".into(), None), ("no-default-browser-check".into(), None)];
        if let Some(list) = env("PANEL_SWITCHES") {
            config.command_line_switches.extend(list.split(',').filter(|part| !part.is_empty()).map(|part| match part.split_once('=') {
                Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
                None => (part.to_owned(), None),
            }));
        }
        let runtime = CefRuntime::initialize(config).ok()?;
        let url = env("PANEL_URL").unwrap_or_else(|| format!("file://{}/fixtures/probe.html", env!("CARGO_MANIFEST_DIR")));
        let host = HostWgpuContext::new(render_state.device.clone(), render_state.queue.clone());
        if !host.device.features().contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF) {
            eprintln!("PANEL: device has no VULKAN_EXTERNAL_MEMORY_DMA_BUF; import will be refused");
        }
        let producer = LinuxCefProducer::new(
            &runtime,
            LinuxCefConfig {
                surface: CefSurfaceConfig {
                    initial_url: url,
                    initial_size: dpi::PhysicalSize::new(1100, 760),
                    scale_factor: cc.egui_ctx.pixels_per_point(),
                    prefer_accelerated: true,
                    ..Default::default()
                },
            },
        )
        .ok()?;
        let now = Instant::now();
        Some(Self {
            runtime,
            producer,
            host,
            render_state: render_state.clone(),
            texture: None,
            popup: None,
            placed: None,
            focused: false,
            left_down: false,
            queue: InputQueue::new(256),
            copy_request: None,
            hidden: false,
            frames: 0,
            repaints: 0,
            last_frame: now,
            last_report: now,
            frame_times: Vec::new(),
            events: Vec::new(),
            ppp: cc.egui_ctx.pixels_per_point(),
            drag_paths: Vec::new(),
        })
    }

    fn publish(&mut self, ctx: &egui::Context, frame: ImportedTexture) {
        let now = Instant::now();
        if self.frames > 0 {
            self.frame_times.push(now.duration_since(self.last_frame).as_secs_f64() * 1000.0);
            if self.frame_times.len() > 600 {
                self.frame_times.remove(0);
            }
        }
        self.last_frame = now;
        self.frames += 1;
        let size = [frame.size.width, frame.size.height];
        let mut renderer = self.render_state.renderer.write();
        if let Some((id, _)) = self.texture.take() {
            renderer.free_texture(&id);
        }
        let id = renderer.register_native_texture(&self.host.device, &frame.view, wgpu::FilterMode::Linear);
        drop(renderer);
        self.texture = Some((id, size));
        // CEF drops input until it has painted once; everything typed before
        // this point is replayed now, in order.
        if !self.queue.is_ready() {
            self.queue.ready();
            eprintln!("QUEUE first frame: replaying {} held event(s)", self.queue.pending());
            // Pointer first; the keys follow on the next frame, after the page
            // has had a chance to focus the field the click landed in.
            let sent = self.queue.flush_pointer(&mut self.producer);
            eprintln!("QUEUE pointer replayed: {sent}");
        }
        ctx.request_repaint();
    }

    fn report(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_report) < Duration::from_secs(1) {
            return;
        }
        self.last_report = now;
        let mut sorted = self.frame_times.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let p50 = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
        eprintln!(
            "STATS frames_total={} frames_last_min={} p50_ms={p50:.1} repaints={} texture={:?}",
            self.frames,
            sorted.len(),
            self.repaints,
            self.texture.map(|(_, size)| size)
        );
    }

    fn send_mouse(&mut self, action: MouseAction, x: f32, y: f32) {
        self.send_mouse_button(action, x, y, MouseButton::Left);
    }

    fn send_mouse_button(&mut self, action: MouseAction, x: f32, y: f32, button: MouseButton) {
        if std::env::var_os("PANEL_TRACE").is_some() {
            eprintln!("INPUT mouse {action:?} {x:.0},{y:.0}");
        }
        if matches!(action, MouseAction::Moved) {
            self.queue.push_motion(x.round() as i32, y.round() as i32, EventModifiers { left_mouse_button: self.left_down, ..Default::default() });
            return;
        }
        let modifiers = EventModifiers { left_mouse_button: self.left_down, ..Default::default() };
        self.queue.push(Input::Mouse(MouseEvent { x: x.round() as i32, y: y.round() as i32, button, action, modifiers }));
    }

    fn send_key(&mut self, key: egui::Key, pressed: bool, modifiers: egui::Modifiers) {
        let Some(vk) = virtual_key(key) else {
            return;
        };
        let modifiers = EventModifiers {
            shift: modifiers.shift,
            ctrl: modifiers.ctrl,
            alt: modifiers.alt,
            // `command` is an alias for ctrl off macOS; sending it as Meta made
            // every Ctrl shortcut arrive as Ctrl+Super and the page treated it
            // as a plain keypress.
            meta: modifiers.mac_cmd,
            ..Default::default()
        };
        let kind = if pressed { KeyEventKind::RawKeyDown } else { KeyEventKind::KeyUp };
        self.queue.push(Input::Key(KeyEvent { kind, windows_key_code: vk, native_key_code: 0, character: None, modifiers }));
    }
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

impl eframe::App for PanelApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.repaints += 1;
        // CEF paints when it wants to; the host only has to pump its loop and
        // ask for a new frame. Idle is deliberately not a 60 Hz timer.
        let animating = self.last_frame.elapsed() < Duration::from_millis(400);
        ctx.request_repaint_after(if animating { Duration::from_millis(16) } else { Duration::from_millis(200) });

        self.runtime.do_message_loop_work();
        while let Some(event) = self.producer.poll_web_event() {
            let text = format!("{event:?}");
            if let CefSurfaceEvent::ScriptCompleted { id, result } = &event {
                if self.copy_request == Some(id.get()) {
                    self.copy_request = None;
                    match result {
                        Ok(json) => {
                            let copied = serde_json_unescape(json);
                            eprintln!("CLIPBOARD copy_out {} byte(s)", copied.len());
                            ctx.copy_text(copied);
                        }
                        Err(error) => eprintln!("CLIPBOARD selection read failed: {error}"),
                    }
                }
            }
            if std::env::var_os("PANEL_TRACE").is_some() {
                eprintln!("NAV {}", text.chars().take(240).collect::<String>());
            }
            if self.events.len() < 40 {
                self.events.push(text.chars().take(200).collect());
            }
        }

        if let Some(native) = self.producer.acquire_native_frame() {
            match welding::WgpuTextureImporter::import(native, &self.host) {
                Ok(texture) => self.publish(&ctx, texture),
                Err(error) => eprintln!("PANEL: import failed: {error}"),
            }
        }
        if let Some(rect) = self.producer.popup_rect() {
            eprintln!("POPUP rect x={} y={} w={} h={}", rect.x, rect.y, rect.width, rect.height);
        }
        match self.producer.acquire_popup(&self.host) {
            Ok(Some(popup)) => {
                let mut renderer = self.render_state.renderer.write();
                if let Some((id, _)) = self.popup.take() {
                    renderer.free_texture(&id);
                }
                let id = renderer.register_native_texture(&self.host.device, &popup.texture.view, wgpu::FilterMode::Linear);
                drop(renderer);
                self.popup = Some((
                    id,
                    egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(popup.texture.size.width as f32, popup.texture.size.height as f32)),
                ));
            }
            Ok(None) => {
                if self.producer.popup_rect().is_none() {
                    if let Some((id, _)) = self.popup.take() {
                        self.render_state.renderer.write().free_texture(&id);
                        eprintln!("POPUP closed");
                    }
                }
            }
            Err(error) => eprintln!("POPUP import failed: {error}"),
        }

        egui::Panel::left("stats").min_size(240.0).show(ui, |ui| {
            ui.heading("CEF OSR panel");
            ui.label(format!("frames: {}", self.frames));
            ui.label(format!("focus: {}", if self.focused { "page" } else { "egui" }));
            ui.label(format!("scale: {:.2}", self.ppp));
            if ui.checkbox(&mut self.hidden, "hide page (set_visible)").changed() && self.hidden {
                self.queue.clear();
            }
            if ui.button("Reload").clicked() {
                let _ = self.producer.reload();
            }
            if ui.button("Focus page").clicked() {
                self.focused = true;
            }
            ui.separator();
            for event in self.events.iter().rev().take(12) {
                ui.small(event);
            }
            ui.separator();
            ui.label("Pages events: title changes come back as navigation events.");
        });

        let ppp = ctx.pixels_per_point();
        if (ppp - self.ppp).abs() > f32::EPSILON {
            self.ppp = ppp;
            let _ = self.producer.set_scale_factor(ppp);
        }

        egui::CentralPanel::default().show(ui, |ui| {
            let available = ui.available_size();
            let (rect, _response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
            let physical = [(rect.width() * ppp).round().max(1.0) as u32, (rect.height() * ppp).round().max(1.0) as u32];
            if self.placed != Some(physical) {
                self.placed = Some(physical);
                eprintln!("PAGE min_x={:.1} min_y={:.1} ppp={:.2} phys={}x{}", rect.min.x, rect.min.y, ppp, physical[0], physical[1]);
                if let Err(error) = self.producer.resize(dpi::PhysicalSize::new(physical[0], physical[1])) {
                    eprintln!("PANEL: resize failed: {error}");
                }
            }
            if self.hidden {
                let _ = self.producer.set_visible(false);
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(30, 30, 34));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "page hidden",
                    egui::FontId::proportional(18.0),
                    egui::Color32::LIGHT_GRAY,
                );
                return;
            }
            let _ = self.producer.set_visible(true);

            if let Some((id, size)) = self.texture {
                let draw = egui::Rect::from_min_size(rect.min, egui::vec2(size[0] as f32 / ppp, size[1] as f32 / ppp));
                ui.painter().image(id, draw, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                if let Some((popup_id, _)) = self.popup {
                    let popup_rect = egui::Rect::from_min_size(draw.min + egui::vec2(80.0, 80.0), egui::vec2(320.0, 200.0));
                    ui.painter().image(
                        popup_id,
                        popup_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            } else {
                ui.painter().rect_filled(rect, 0.0, egui::Color32::from_rgb(20, 20, 24));
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "waiting for the first CEF frame",
                    egui::FontId::proportional(18.0),
                    egui::Color32::GRAY,
                );
            }

            // ── input forwarding ─────────────────────────────────────────────
            // Read egui's events rather than polling the pointer state: a
            // compositor can deliver press+release inside one frame, and a
            // state poll only sees the release, which loses the click.
            let events = ctx.input(|i| i.events.clone());
            for event in events {
                if std::env::var_os("PANEL_TRACE").is_some() {
                    eprintln!("EVENT {event:?}");
                }
                match event {
                    egui::Event::PointerMoved(pos) if rect.contains(pos) => {
                        let local = (pos - rect.min) * ppp;
                        self.send_mouse(MouseAction::Moved, local.x, local.y);
                    }
                    egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                        let inside = rect.contains(pos);
                        if pressed && !inside {
                            // A press on the CAD side takes the keyboard back;
                            // nothing queued for the page may arrive later.
                            self.focused = false;
                            self.left_down = false;
                            self.queue.clear();
                            continue;
                        }
                        let cef_button = match button {
                            egui::PointerButton::Primary => MouseButton::Left,
                            egui::PointerButton::Secondary => MouseButton::Right,
                            egui::PointerButton::Middle => MouseButton::Middle,
                            _ => continue,
                        };
                        if pressed && inside {
                            self.focused = true;
                            let _ = self.producer.move_focus(FocusDirection::Forward);
                        }
                        if cef_button == MouseButton::Left {
                            self.left_down = pressed;
                        }
                        let local = (pos - rect.min) * ppp;
                        let action = if pressed { MouseAction::Pressed } else { MouseAction::Released };
                        let _ = modifiers;
                        self.send_mouse_button(action, local.x, local.y, cef_button);
                    }
                    egui::Event::MouseWheel { unit, delta, .. } if rect.contains(ctx.pointer_latest_pos().unwrap_or(egui::Pos2::ZERO)) => {
                        let scale = match unit {
                            egui::MouseWheelUnit::Line => 20.0,
                            egui::MouseWheelUnit::Page => 120.0,
                            egui::MouseWheelUnit::Point => 1.0,
                        };
                        let pos = ctx.pointer_latest_pos().unwrap_or(rect.center());
                        let local = (pos - rect.min) * ppp;
                        self.send_mouse(
                            MouseAction::WheelScrolled { delta_x: (delta.x * scale) as i32, delta_y: (delta.y * scale) as i32 },
                            local.x,
                            local.y,
                        );
                    }
                    egui::Event::Text(text) if self.focused => {
                        if std::env::var_os("PANEL_TRACE").is_some() {
                            eprintln!("INPUT text {text:?}");
                        }
                        for ch in text.chars() {
                            self.queue.push(char_event(ch));
                        }
                    }
                    egui::Event::Key { key, pressed, modifiers, .. } if self.focused => {
                        let command = modifiers.ctrl || modifiers.mac_cmd;
                        if pressed && command && key == egui::Key::C {
                            // CEF OSR does not publish the page's selection to
                            // the system clipboard (cef#4341), so the host reads
                            // it and gives it to the windowing system via egui.
                            self.copy_request = Some(7);
                            let _ = self.producer.request_script_result(
                                WebRequestId::new(7),
                                "(() => { const el = document.activeElement; \
                                 if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) { \
                                   return el.value.substring(el.selectionStart, el.selectionEnd); } \
                                 return window.getSelection().toString(); })()",
                            );
                            continue;
                        }
                        if pressed && command && key == egui::Key::V {
                            // Delivered as Event::Paste; CEF's own paste path is
                            // the broken half of the same upstream issue.
                            continue;
                        }
                        if key == egui::Key::Escape && pressed {
                            // Leaving the page: nothing queued for it should
                            // arrive after the CAD side has the keyboard.
                            self.queue.clear();
                            self.focused = false;
                        } else {
                            self.send_key(key, pressed, modifiers);
                        }
                    }
                    egui::Event::Copy => {
                        if self.focused {
                            self.copy_request = Some(7);
                            let _ = self.producer.request_script_result(
                                WebRequestId::new(7),
                                "(() => { const el = document.activeElement; \
                                 if (el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA')) { \
                                   return el.value.substring(el.selectionStart, el.selectionEnd); } \
                                 return window.getSelection().toString(); })()",
                            );
                        }
                    }
                    egui::Event::Paste(text) if self.focused => {
                        // Host -> page: CEF's own clipboard is broken under OSR
                        // (cef#4341), so the system text is committed into the
                        // focused editable the way an input method commits.
                        eprintln!("CLIPBOARD paste_in {} byte(s)", text.len());
                        self.queue.push(Input::ImeCommit(text));
                    }
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars }) => {
                        if text.is_empty() {
                            self.queue.push(Input::ImeCancel);
                        } else {
                            let range = active_range_chars.unwrap_or(0..text.chars().count());
                            self.queue.push(Input::ImeSet { text, cursor: (range.start as u32, range.end as u32) });
                        }
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                        self.queue.push(Input::ImeCommit(text));
                    }
                    _ => {}
                }
            }

            // A window that lost focus must not replay input later.
            let window_focused = ctx.input(|i| i.viewport().focused.unwrap_or(true));
            if !window_focused {
                self.queue.clear();
                self.focused = false;
            }
            if self.hidden {
                self.queue.clear();
            }
            let sent = self.queue.flush(&mut self.producer);
            if sent > 0 && std::env::var_os("PANEL_TRACE").is_some() {
                eprintln!("QUEUE flushed {sent}");
            }

            // ── drag and drop ────────────────────────────────────────────────
            let hovering = ctx.input(|i| i.raw.hovered_files.clone());
            let paths: Vec<String> = hovering.iter().filter_map(|file| file.path.as_ref().map(|p| p.display().to_string())).collect();
            if paths != self.drag_paths {
                let (kind, x, y) = if paths.is_empty() {
                    (DragEventKind::Leave, 0, 0)
                } else {
                    (DragEventKind::Enter, rect.width() as i32 / 2, rect.height() as i32 / 2)
                };
                self.drag_paths = paths.clone();
                let _ = self.producer.send_drag_input(DragInput {
                    kind,
                    payload: Some(DragPayload::default()),
                    x,
                    y,
                    modifiers: EventModifiers::default(),
                    allowed_operations: DragOperations::COPY,
                });
            }
            let dropped = ctx.input(|i| i.raw.dropped_files.clone());
            for file in dropped {
                {
                    let path = file.path().to_path_buf();
                    let payload = DragPayload { files: vec![welding::DragFile { path, display_name: None }], ..Default::default() };
                    let _ = self.producer.send_drag_input(DragInput {
                        kind: DragEventKind::Drop,
                        payload: Some(payload),
                        x: rect.width() as i32 / 2,
                        y: rect.height() as i32 / 2,
                        modifiers: EventModifiers::default(),
                        allowed_operations: DragOperations::COPY,
                    });
                }
            }
        });

        self.report();
    }
}

fn main() {
    let Some(cef_path) = env("CEF_PATH") else {
        eprintln!("CEF_PATH must point at a CEF binary distribution containing libcef.so");
        std::process::exit(2);
    };
    let sandbox = match env("PANEL_SANDBOX").as_deref() {
        Some("sandboxed") => CefSandboxMode::Sandboxed,
        _ => CefSandboxMode::UnsandboxedTrustedContent,
    };
    // MUST be first, before eframe/winit/wgpu start any threads.
    match CefRuntime::execute_process_from(std::path::Path::new(&cef_path), sandbox) {
        Ok(Some(code)) => std::process::exit(code),
        Ok(None) => {}
        Err(error) => {
            eprintln!("CEF subprocess probe failed: {error}");
            std::process::exit(2);
        }
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // A device egui can paint with **and** grafting can import CEF's DMABUF
    // into. eframe's default path cannot produce one: it calls
    // `adapter.request_device`, while the import needs the Vulkan device opened
    // with `VK_EXT_queue_family_foreign`, which only `open_with_callback` adds.
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    })) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("no Vulkan adapter: {error}");
            std::process::exit(3);
        }
    };
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("cef-osr-panel"),
        required_features: adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF,
        ..Default::default()
    };
    let (device, queue) = match welding::build_dmabuf_capable_device(&adapter, &descriptor) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("DMABUF-capable device failed: {error}");
            std::process::exit(3);
        }
    };
    let mut options = eframe::NativeOptions { renderer: eframe::Renderer::Wgpu, ..Default::default() };
    options.wgpu_options.wgpu_setup =
        eframe::egui_wgpu::WgpuSetup::Existing(eframe::egui_wgpu::WgpuSetupExisting { instance, adapter, device, queue });
    let result = eframe::run_native(
        "CEF OSR panel",
        options,
        Box::new(|cc| match PanelApp::new(cc) {
            Some(app) => Ok(Box::new(app) as Box<dyn eframe::App>),
            None => Err("CEF panel could not start (see stderr)".into()),
        }),
    );
    if let Err(error) = result {
        eprintln!("eframe: {error}");
        std::process::exit(3);
    }
}

/// The script result is JSON; a string selection arrives quoted and escaped.
fn serde_json_unescape(json: &str) -> String {
    let inner = json.trim();
    let inner = inner.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')).unwrap_or(inner);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('/') => out.push('/'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}
