//! Disposable GTK 3 + GtkGLArea blit prototype: wgpu renders the production egui frame to an
//! offscreen texture, then GTK's own GL context uploads and draws it. This is the candidate path
//! for native Wayland, where a separate wgpu swapchain on GTK's `wl_surface` is not composited.
//!
//! Run: GDK_BACKEND=wayland|:x11 PROTO_UI=1 cargo run -p solidcraft --example gtk_wgpu_blit

#![allow(unsafe_code)]

use std::cell::{Cell, RefCell};
use std::ffi::CString;
use std::rc::Rc;
use std::time::Instant;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use glow::HasContext;
use gtk::prelude::*;
use solidcraft_engine::Session;
use webkit2gtk::{SettingsExt, URISchemeRequestExt, UserContentInjectedFrames, UserContentManagerExt, UserScript, UserScriptInjectionTime, WebContextExt, WebViewExt};

const VS: &str = r#"#version 330 core
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
out vec2 v_uv;
void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}"#;

const FS: &str = r#"#version 330 core
in vec2 v_uv;
out vec4 out_color;
uniform sampler2D u_tex;
void main() { out_color = texture(u_tex, v_uv); }"#;

struct State {
    solve: solidcraft_ui_egui::SolveApp,
    rs: egui_wgpu::RenderState,
    ctx: egui::Context,
    gl: glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    tex: glow::Texture,
    offscreen: Option<wgpu::Texture>,
    gl_format: u32,
    size: (u32, u32),
    frames: u64,
    started: Instant,
    frame_at: Rc<Cell<Option<Instant>>>,
    pending: Rc<RefCell<Vec<egui::Event>>>,
    stats: bool,
    last_stat: Instant,
    last_frames: u64,
    last_delay: Option<std::time::Duration>,
    applied_scale: f32,
}

fn main() {
    if gtk::init().is_err() {
        eprintln!("GTK could not start");
        std::process::exit(1);
    }
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("open-skyfall GTK wgpu blit prototype");
    window.set_default_size(1200, 800);
    let area = gtk::GLArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_can_focus(true);
    area.set_required_version(3, 3);
    let dist = std::env::var("PROTO_DIST").unwrap_or_else(|_| "packages/assistant-web/dist".into());
    let context = webkit2gtk::WebContext::default();
    if let Some(context) = &context {
        let dist = dist.clone();
        context.register_uri_scheme("openskyfall-assistant", move |request| {
            let uri = request.uri().map(|u| u.to_string()).unwrap_or_default();
            let name = uri
                .strip_prefix("openskyfall-assistant://localhost/")
                .unwrap_or("index.html")
                .split(['?', '#'])
                .next()
                .unwrap_or("index.html");
            let path = std::path::Path::new(&dist).join(name);
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let kind = match path.extension().and_then(|e| e.to_str()) {
                        Some("js") => "text/javascript",
                        Some("css") => "text/css",
                        Some("woff2") => "font/woff2",
                        Some("ttf") => "font/ttf",
                        Some("html") => "text/html",
                        _ => "application/octet-stream",
                    };
                    let stream = gtk::gio::MemoryInputStream::from_bytes(&gtk::glib::Bytes::from(&bytes));
                    request.finish(&stream, bytes.len() as i64, Some(kind));
                }
                Err(error) => eprintln!("PROTO page asset {name}: {error}"),
            }
        });
    }
    let web = context.as_ref().map(webkit2gtk::WebView::with_context);
    if let Some(web) = &web {
        if let Some(manager) = web.user_content_manager() {
            manager.register_script_message_handler("ipc");
            let shim = UserScript::new(
                "window.ipc = { postMessage: function (m) { window.webkit.messageHandlers.ipc.postMessage(m); } };\
                 window.addEventListener('error', function (e) { try { console.error('PROTO JSERROR ' + e.message); } catch (_) {} });",
                UserContentInjectedFrames::TopFrame,
                UserScriptInjectionTime::Start,
                &[],
                &[],
            );
            manager.add_script(&shim);
            manager.connect_script_message_received(Some("ipc"), |_, result| {
                println!("PROTO ipc {:?}", result.js_value());
            });
        }
        web.connect_web_process_terminated(|_, reason| eprintln!("PROTO web process terminated: {reason:?}"));
        web.connect_title_notify(|web| println!("PROTO title {:?}", web.title()));
        web.set_size_request(430, -1);
        web.set_hexpand(false);
        web.set_vexpand(true);
        web.connect_load_failed(|_, _, uri, error| {
            eprintln!("PROTO load failed {uri}: {error}");
            false
        });
        let mut production = solidcraft_ui_egui::SolveApp::new(Session::default(), Default::default());
        let state_json = solidcraft_ui_egui::agent_chat::web::state(&mut production);
        let state_for_web = state_json.clone();
        let web_for_state = web.clone();
        web.connect_load_changed(move |_, event| {
            println!("PROTO load {event:?}");
            if event == webkit2gtk::LoadEvent::Finished {
                let js = format!("if (window.__assistantHost) {{ window.__assistantHost.state({state_for_web}); document.body.setAttribute('data-proto-state','pushed'); }} else {{ window.ipc && window.ipc.postMessage(JSON.stringify({{type:'log',text:'no host object'}})); }}");
                let web_later = web_for_state.clone();
                gtk::glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                    web_later.run_javascript(&js, None::<&gtk::gio::Cancellable>, |_| {});
                    gtk::glib::ControlFlow::Break
                });
            }
        });
        if let Some(settings) = WebViewExt::settings(web) {
            settings.set_enable_write_console_messages_to_stdout(true);
        }
        web.load_uri("openskyfall-assistant://localhost/index.html");
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.pack_start(&area, true, true, 0);
    if let Some(web) = &web {
        row.pack_end(web, false, true, 0);
    }
    window.add(&row);

    let frame_at = Rc::new(Cell::new(Some(Instant::now())));
    let frame_at_timer = frame_at.clone();
    let pending = Rc::new(RefCell::new(Vec::new()));
    area.add_events(
        gtk::gdk::EventMask::BUTTON_PRESS_MASK
            | gtk::gdk::EventMask::BUTTON_RELEASE_MASK
            | gtk::gdk::EventMask::POINTER_MOTION_MASK
            | gtk::gdk::EventMask::SCROLL_MASK,
    );
    let press_pending = pending.clone();
    let press_frame = frame_at.clone();
    area.connect_button_press_event(move |area, event| {
        area.grab_focus();
        let (x, y) = event.position();
        press_pending.borrow_mut().push(egui::Event::PointerButton {
            pos: egui::pos2(x as f32, y as f32),
            button: pointer_button(event.button()),
            pressed: true,
            modifiers: modifiers(event.state()),
        });
        press_frame.set(Some(Instant::now()));
        gtk::glib::Propagation::Proceed
    });
    let release_pending = pending.clone();
    let release_frame = frame_at.clone();
    area.connect_button_release_event(move |_, event| {
        let (x, y) = event.position();
        release_pending.borrow_mut().push(egui::Event::PointerButton {
            pos: egui::pos2(x as f32, y as f32),
            button: pointer_button(event.button()),
            pressed: false,
            modifiers: modifiers(event.state()),
        });
        release_frame.set(Some(Instant::now()));
        gtk::glib::Propagation::Proceed
    });
    let motion_pending = pending.clone();
    let motion_frame = frame_at.clone();
    area.connect_motion_notify_event(move |_, event| {
        let (x, y) = event.position();
        motion_pending.borrow_mut().push(egui::Event::PointerMoved(egui::pos2(x as f32, y as f32)));
        motion_frame.set(Some(Instant::now()));
        gtk::glib::Propagation::Proceed
    });
    let scroll_pending = pending.clone();
    let scroll_frame = frame_at.clone();
    area.connect_scroll_event(move |_, event| {
        let (dx, dy) = event.delta();
        scroll_pending.borrow_mut().push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(dx as f32, dy as f32),
            phase: egui::TouchPhase::Move,
            modifiers: modifiers(event.state()),
        });
        scroll_frame.set(Some(Instant::now()));
        gtk::glib::Propagation::Proceed
    });
    let state: Rc<RefCell<Option<State>>> = Rc::new(RefCell::new(None));
    let state_realize = state.clone();
    area.connect_realize(move |area| {
        eprintln!("PROTO blit realize");
        area.make_current();
        match unsafe { init(area, frame_at.clone(), pending.clone()) } {
            Ok(s) => *state_realize.borrow_mut() = Some(s),
            Err(error) => eprintln!("PROTO blit setup failed: {error}"),
        }
    });
    let state_unrealize = state.clone();
    area.connect_unrealize(move |area| {
        area.make_current();
        *state_unrealize.borrow_mut() = None;
    });
    let state_draw = state.clone();
    area.connect_render(move |area, _| {
        area.make_current();
        if let Some(s) = state_draw.borrow_mut().as_mut()
            && let Err(error) = s.draw(area)
        {
            eprintln!("PROTO blit draw failed: {error}");
        }
        gtk::glib::Propagation::Stop
    });
    window.connect_destroy(|_| gtk::main_quit());
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        if let Some(at) = frame_at_timer.get()
            && Instant::now() >= at
        {
            frame_at_timer.set(None);
            area.queue_render();
        }
        gtk::glib::ControlFlow::Continue
    });
    window.show_all();
    gtk::main();
}

impl State {
    fn draw(&mut self, area: &gtk::GLArea) -> Result<(), String> {
        let scale = area.scale_factor().max(1) as f32;
        let (width, height) = (area.allocated_width(), area.allocated_height());
        let (pw, ph) = ((width as f64 * f64::from(scale)) as u32, (height as f64 * f64::from(scale)) as u32);
        if pw == 0 || ph == 0 {
            return Ok(());
        }
        let mut raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width as f32, height as f32))),
            events: self.pending.borrow_mut().drain(..).collect(),
            ..Default::default()
        };
        if (self.applied_scale - scale).abs() > f32::EPSILON {
            self.ctx.set_pixels_per_point(scale);
            self.applied_scale = scale;
        }
        let real_ui = std::env::var_os("PROTO_UI").is_some();
        if real_ui {
            self.solve.raw_input_hook(&mut raw);
        }
        let State { solve, ctx, started, frames, .. } = self;
        let mut output = ctx.run_ui(raw, |ui| {
            if real_ui {
                solve.ui(ui);
            } else {
                ui.heading("GTK 3 + wgpu offscreen -> GtkGLArea");
                ui.label("wgpu renders offscreen; GTK's GL context uploads and draws it.");
                let elapsed = started.elapsed().as_secs_f32();
                ui.label(format!("frame {} · {:.1} fps average", frames, *frames as f32 / elapsed.max(0.001)));
            }
        });
        if real_ui {
            solve.logic(ctx);
        }
        let delay = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(std::time::Duration::MAX, |viewport| viewport.repaint_delay);
        self.last_delay = Some(delay);
        self.frame_at.set(if *frames < 2 {
            Some(Instant::now())
        } else if delay == std::time::Duration::MAX {
            None
        } else {
            Instant::now().checked_add(delay)
        });
        let mut renderer = self.rs.renderer.write();
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&self.rs.device, &self.rs.queue, *id, delta);
            }
        }
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
        output.textures_delta.clear();
        let jobs = ctx.tessellate(output.shapes, output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [pw, ph], pixels_per_point: output.pixels_per_point };

        if self.size != (pw, ph) {
            let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gtk-wgpu-blit"),
                size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.rs.target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            self.offscreen = Some(texture);
            self.size = (pw, ph);
        }
        let texture = self.offscreen.as_ref().ok_or("no offscreen texture")?;
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("blit") });
        renderer.update_buffers(&self.rs.device, &self.rs.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui-offscreen"),
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
        let bytes_per_row = (pw * 4).div_ceil(256) * 256;
        let buffer = self.rs.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(bytes_per_row) * u64::from(ph),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bytes_per_row), rows_per_image: Some(ph) },
            },
            wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
        );
        self.rs.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.rs.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).map_err(|e| format!("poll: {e}"))?;
        rx.recv().map_err(|e| format!("map: {e}"))?.map_err(|e| format!("map: {e}"))?;
        let mapped = buffer.slice(..).get_mapped_range().map_err(|e| format!("map range: {e}"))?;
        let mut tight = Vec::with_capacity((pw * ph * 4) as usize);
        for row in 0..ph {
            let start = (row * bytes_per_row) as usize;
            tight.extend_from_slice(&mapped[start..start + (pw * 4) as usize]);
        }
        drop(mapped);
        buffer.unmap();

        // Upload the wgpu frame into GTK's GL context and draw it.
        unsafe {
            let gl = &self.gl;
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                pw as i32,
                ph as i32,
                0,
                self.gl_format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&tight)),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.viewport(0, 0, pw as i32, ph as i32);
            gl.clear_color(0.0, 0.0, 0.0, 1.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex));
            let loc = gl.get_uniform_location(self.program, "u_tex");
            gl.uniform_1_i32(loc.as_ref(), 0);
            gl.draw_arrays(glow::TRIANGLES, 0, 6);
        }
        self.frames = self.frames.saturating_add(1);
        if self.stats {
            let now = Instant::now();
            let elapsed = now.duration_since(self.last_stat).as_secs_f32();
            if elapsed >= 1.0 {
                let drawn = self.frames.saturating_sub(self.last_frames);
                eprintln!("PROTO fps={:.1} frame_ms={:.2} frames={} delay={:?}", drawn as f32 / elapsed, 1000.0 * elapsed / drawn.max(1) as f32, self.frames, self.last_delay);
                self.last_stat = now;
                self.last_frames = self.frames;
            }
        }
        Ok(())
    }
}


impl Drop for State {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.program);
            self.gl.delete_vertex_array(self.vao);
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_texture(self.tex);
        }
    }
}

unsafe fn init(area: &gtk::GLArea, frame_at: Rc<Cell<Option<Instant>>>, pending: Rc<RefCell<Vec<egui::Event>>>) -> Result<State, String> {
    area.make_current();
    let handle = loader();
    if handle.is_null() {
        return Err("no GL library".into());
    }
    let gl = unsafe {
        glow::Context::from_loader_function(|symbol| {
            let c = CString::new(symbol).unwrap_or_default();
            let p = libc::dlsym(handle, c.as_ptr());
            if p.is_null() { libc::dlsym(libc::RTLD_DEFAULT, c.as_ptr()) } else { p }
        })
    };
    let (program, vao, vbo) = unsafe { gl_setup(&gl)? };
    let tex = unsafe {
        let tex = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        tex
    };
    // GTK's GLArea context is current on this thread. wgpu's GL/EGL backend then fails with
    // BadAccess; force Vulkan (the production app's preferred backend) for the offscreen frame.
    let mut instance_desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    instance_desc.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(instance_desc);
    let rs = pollster::block_on(egui_wgpu::RenderState::create(
        &egui_wgpu::WgpuConfiguration::default(),
        &instance,
        None,
        egui_wgpu::RendererOptions { msaa_samples: 1, ..Default::default() },
    ))
    .map_err(|e| format!("RenderState::create: {e}"))?;
    println!("PROTO blit wgpu backend={:?} adapter={:?}", rs.adapter.get_info().backend, rs.adapter.get_info().name);
    let ctx = egui::Context::default();
    solidcraft_ui_egui::fonts::setup_bundled_fonts(&ctx);
    let mut solve = solidcraft_ui_egui::SolveApp::new(Session::default(), Default::default());
    if std::env::var_os("PROTO_MODEL").is_some() {
        let _ = solve.session.execute("solid.box", &serde_json::json!({"length": 30, "width": 20, "height": 10}));
    }
    solve.set_wgpu_split(&rs, 24, 0, 4);
    let gl_format = match rs.target_format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => glow::BGRA,
        _ => glow::RGBA,
    };
    println!("PROTO blit format={:?} gl_format={:#x}", rs.target_format, gl_format);
    Ok(State {
        solve, rs, ctx, gl, program, vao, vbo, tex, offscreen: None, gl_format, size: (0, 0),
        frames: 0, started: Instant::now(), frame_at, pending,
        stats: std::env::var_os("PROTO_STATS").is_some(), last_stat: Instant::now(), last_frames: 0,
        last_delay: None, applied_scale: 0.0,
    })
}

unsafe fn gl_setup(gl: &glow::Context) -> Result<(glow::Program, glow::VertexArray, glow::Buffer), String> {
    let program = gl.create_program()?;
    let vs = shader(gl, glow::VERTEX_SHADER, VS)?;
    let fs = shader(gl, glow::FRAGMENT_SHADER, FS)?;
    gl.attach_shader(program, vs);
    gl.attach_shader(program, fs);
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        return Err(gl.get_program_info_log(program));
    }
    gl.delete_shader(vs);
    gl.delete_shader(fs);
    let vao = gl.create_vertex_array()?;
    gl.bind_vertex_array(Some(vao));
    let vbo = gl.create_buffer()?;
    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
    #[rustfmt::skip]
    let vertices: [f32; 24] = [
        -1.0, -1.0, 0.0, 1.0,
         1.0, -1.0, 1.0, 1.0,
         1.0,  1.0, 1.0, 0.0,
        -1.0, -1.0, 0.0, 1.0,
         1.0,  1.0, 1.0, 0.0,
        -1.0,  1.0, 0.0, 0.0,
    ];
    let bytes: &[u8] = core::slice::from_raw_parts(vertices.as_ptr().cast(), vertices.len() * 4);
    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
    let stride = 4 * 4;
    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
    gl.enable_vertex_attrib_array(1);
    gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 2 * 4);
    Ok((program, vao, vbo))
}

unsafe fn shader(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader, String> {
    let shader = gl.create_shader(kind)?;
    gl.shader_source(shader, source);
    gl.compile_shader(shader);
    if !gl.get_shader_compile_status(shader) {
        return Err(gl.get_shader_info_log(shader));
    }
    Ok(shader)
}

fn loader() -> *mut core::ffi::c_void {
    unsafe {
        for name in ["libGL.so.1", "libEGL.so.1", "libOpenGL.so.0"] {
            let Ok(name) = CString::new(name) else { continue };
            let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
            if !handle.is_null() {
                return handle;
            }
        }
        libc::RTLD_DEFAULT
    }
}



fn pointer_button(button: u32) -> egui::PointerButton {
    match button {
        1 => egui::PointerButton::Primary,
        2 => egui::PointerButton::Middle,
        3 => egui::PointerButton::Secondary,
        8 => egui::PointerButton::Extra1,
        9 => egui::PointerButton::Extra2,
        _ => egui::PointerButton::Primary,
    }
}

fn modifiers(state: gtk::gdk::ModifierType) -> egui::Modifiers {
    let shift = state.contains(gtk::gdk::ModifierType::SHIFT_MASK);
    let ctrl = state.contains(gtk::gdk::ModifierType::CONTROL_MASK);
    let alt = state.contains(gtk::gdk::ModifierType::MOD1_MASK);
    egui::Modifiers { shift, ctrl, alt, mac_cmd: false, command: ctrl }
}
