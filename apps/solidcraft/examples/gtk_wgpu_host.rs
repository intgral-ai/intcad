//! Disposable GTK 3 + production-wgpu host prototype (X11 first; Wayland handle included).
//!
//! This is the follow-up to `gtk_host.rs`: instead of a hand-written glow shader, it creates a
//! `wgpu::Surface` from the GTK window and uses eframe's `egui_wgpu::RenderState`, the same
//! renderer family and viewport resources as the production app. `PROTO_UI=1` draws the real
//! `SolveApp` CAD UI; without it a small egui frame proves the renderer path alone.
//!
//! Run on X11:   GDK_BACKEND=x11 PROTO_UI=1 cargo run -p solidcraft --example gtk_wgpu_host
//! Run on Wayland: GDK_BACKEND=wayland PROTO_UI=1 cargo run -p solidcraft --example gtk_wgpu_host
//!
//! The backend is printed as `PROTO wgpu backend=...` so evidence records what actually ran.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use gtk::gdk;
use gtk::gdk::prelude::DisplayExtManual;
use gtk::glib::translate::ToGlibPtr;
use gtk::prelude::*;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, XlibDisplayHandle, XlibWindowHandle};
use solidcraft_engine::Session;

struct State {
    solve: solidcraft_ui_egui::SolveApp,
    surface: wgpu::Surface<'static>,
    rs: egui_wgpu::RenderState,
    config: wgpu::SurfaceConfiguration,
    ctx: egui::Context,
    frames: u64,
    started: std::time::Instant,
}

fn main() {
    if gtk::init().is_err() {
        eprintln!("GTK could not start");
        std::process::exit(1);
    }
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("open-skyfall GTK wgpu prototype");
    window.set_default_size(1200, 800);
    let area = gtk::DrawingArea::new();
    area.set_app_paintable(true);
    area.set_hexpand(true);
    area.set_vexpand(true);
    window.add(&area);
    window.show_all();

    let state: Rc<RefCell<Option<State>>> = Rc::new(RefCell::new(None));
    let state_for_draw = state.clone();
    area.connect_draw(move |area, _cr| {
        let mut slot = state_for_draw.borrow_mut();
        if slot.is_none() {
            match unsafe { make_state(area) } {
                Ok(state) => *slot = Some(state),
                Err(error) => {
                    eprintln!("PROTO wgpu setup failed: {error}");
                    return gtk::glib::Propagation::Stop;
                }
            }
        }
        let Some(state) = slot.as_mut() else { return gtk::glib::Propagation::Stop };
        if let Err(error) = state.draw(area) {
            eprintln!("PROTO draw failed: {error}");
        }
        gtk::glib::Propagation::Stop
    });

    window.connect_destroy(|_| gtk::main_quit());
    // A redraw every 16 ms keeps the measurement honest while the prototype is interactive.
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        if let Some(area) = area.window() {
            area.invalidate_rect(None, false);
        }
        gtk::glib::ControlFlow::Continue
    });
    gtk::main();
}

impl State {
    fn draw(&mut self, area: &gtk::DrawingArea) -> Result<(), String> {
        let scale = area.scale_factor().max(1) as f32;
        let (width, height) = (area.allocated_width(), area.allocated_height());
        let (pw, ph) = ((width as f64 * f64::from(scale)) as u32, (height as f64 * f64::from(scale)) as u32);
        if pw == 0 || ph == 0 {
            return Ok(());
        }
        let State { solve, surface, rs, config, ctx, frames, started } = self;
        if config.width != pw || config.height != ph {
            config.width = pw;
            config.height = ph;
            surface.configure(&rs.device, config);
        }
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32))),
            ..Default::default()
        };
        let real_ui = std::env::var_os("PROTO_UI").is_some();
        if real_ui && *frames == 300 && std::env::var_os("PROTO_MODEL2").is_some() {
            let _ = solve.session.execute("solid.box", &serde_json::json!({"length": 12, "width": 12, "height": 36, "corner": [30, 0, 0]}));
        }
        if real_ui {
            solve.raw_input_hook(&mut raw);
        }
        let output = ctx.run_ui(raw, |ui| {
            if real_ui {
                solve.ui(ui);
            } else {
                ui.heading("GTK 3 + production wgpu");
                ui.label("wgpu surface created from the GTK window; egui_wgpu renders this frame.");
                let elapsed = started.elapsed().as_secs_f32();
                ui.label(format!("frame {} · {:.1} fps average", frames, *frames as f32 / elapsed.max(0.001)));
            }
        });
        if real_ui {
            solve.logic(ctx);
        }
        let mut output = output;
        let mut renderer = rs.renderer.write();
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&rs.device, &rs.queue, *id, delta);
            }
        }
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        output.textures_delta.clear();
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [pw, ph], pixels_per_point: output.pixels_per_point };
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gtk-wgpu") });
        renderer.update_buffers(&rs.device, &rs.queue, &mut encoder, &jobs, &screen);
        let frame = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout => return Err("surface texture timed out".into()),
            wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            wgpu::CurrentSurfaceTexture::Outdated => {
                surface.configure(&rs.device, config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => return Err("surface lost".into()),
            _ => return Err("surface validation failed".into()),
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.055, g: 0.065, b: 0.08, a: 1.0 }), store: wgpu::StoreOp::Store },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            renderer.render(&mut pass, &jobs, &screen);
        }
        rs.queue.submit(Some(encoder.finish()));
        rs.queue.present(frame);
        *frames = frames.saturating_add(1);
        Ok(())
    }
}

unsafe fn make_state(area: &gtk::DrawingArea) -> Result<State, String> {
    let gdk_window = area.window().ok_or("the GTK window is not realized")?;
    let (raw_window_handle, raw_display_handle) = unsafe { handles(&gdk_window)? };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let surface = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle { raw_window_handle, raw_display_handle: Some(raw_display_handle) })
    }
    .map_err(|error| format!("create_surface: {error}"))?;
    let options = egui_wgpu::RendererOptions { msaa_samples: 1, ..Default::default() };
    let rs = pollster::block_on(egui_wgpu::RenderState::create(
        &egui_wgpu::WgpuConfiguration::default(),
        &instance,
        Some(&surface),
        options,
    ))
    .map_err(|error| format!("RenderState::create: {error}"))?;
    println!("PROTO wgpu backend={:?} adapter={:?}", rs.adapter.get_info().backend, rs.adapter.get_info().name);
    let caps = surface.get_capabilities(&rs.adapter);
    // The renderer's pipelines were made for `rs.target_format`; the surface must use it.
    let format = rs.target_format;
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width: (area.allocated_width().max(1) as f64 * f64::from(area.scale_factor().max(1))) as u32,
        height: (area.allocated_height().max(1) as f64 * f64::from(area.scale_factor().max(1))) as u32,
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: caps.alpha_modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto),
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: wgpu::SurfaceColorSpace::Auto,
    };
    surface.configure(&rs.device, &config);
    let ctx = egui::Context::default();
    solidcraft_ui_egui::fonts::setup_bundled_fonts(&ctx);
    let mut solve = solidcraft_ui_egui::SolveApp::new(Session::default(), Default::default());
    if std::env::var_os("PROTO_MODEL").is_some() {
        let _ = solve.session.execute("solid.box", &serde_json::json!({"length": 30, "width": 20, "height": 10}));
    }
    solve.set_wgpu_split(&rs, 24, 0, 4);
    Ok(State { solve, surface, rs, config, ctx, frames: 0, started: std::time::Instant::now() })
}

unsafe fn handles(window: &gdk::Window) -> Result<(RawWindowHandle, RawDisplayHandle), String> {
    #[cfg(target_os = "linux")]
    {
        let display = window.display();
        if display.backend().is_x11() {
            let x11 = window.clone().downcast::<gdkx11::X11Window>().map_err(|_| "not an X11 window")?;
            let display11 = display.clone().downcast::<gdkx11::X11Display>().map_err(|_| "not an X11 display")?;
            let xid = x11.xid();
            let display_ptr: *mut gdkx11::ffi::GdkX11Display = display11.to_glib_none().0;
            let xdisplay = unsafe { gdkx11::ffi::gdk_x11_display_get_xdisplay(display_ptr) };
            let display = NonNull::new(xdisplay.cast::<c_void>()).ok_or("no X display")?;
            return Ok((
                RawWindowHandle::Xlib(XlibWindowHandle::new(xid)),
                RawDisplayHandle::Xlib(XlibDisplayHandle::new(Some(display), 0)),
            ));
        }
        let surface = unsafe { gdk_wayland_window_get_wl_surface(window.to_glib_none().0) };
        let display = unsafe { gdk_wayland_display_get_wl_display(display.to_glib_none().0) };
        let surface = NonNull::new(surface).ok_or("no Wayland surface")?;
        let display = NonNull::new(display).ok_or("no Wayland display")?;
        return Ok((
            RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
            RawDisplayHandle::Wayland(WaylandDisplayHandle::new(display)),
        ));
    }
    #[allow(unreachable_code)]
    Err("no Linux display".into())
}

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn gdk_wayland_window_get_wl_surface(window: *mut gdk::ffi::GdkWindow) -> *mut c_void;
    fn gdk_wayland_display_get_wl_display(display: *mut gdk::ffi::GdkDisplay) -> *mut c_void;
}
