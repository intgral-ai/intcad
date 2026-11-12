//! Disposable GTK host prototype for the Assistant: a GTK 3 window on X11 or native Wayland
//! with a real `solidcraft-engine` scene in a `GtkGLArea` and the real Assistant page in a
//! WebKitGTK view beside it.
//!
//! This is a prototype, not the app. It answers whether GTK can own the window and host both a
//! GPU CAD viewport and WebKitGTK on native Wayland, what its input/focus/IME and frame timing
//! look like, and how it packages. It is not wired into `main.rs`.
//!
//! Run:
//!   cargo run -p solidcraft --example gtk_host --release
//!   GDK_BACKEND=x11 ...        # X11 / XWayland
//!   GDK_BACKEND=wayland ...    # native Wayland
//! Environment:
//!   PROTO_SPIN=1     request a frame every 8 ms (render-rate measurement; not idle policy)
//!   PROTO_PAGE=path  page to load (default packages/assistant-web/dist/index.html)
//!   PROTO_LOG=1      print frames and input events

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::rc::Rc;
use std::time::{Duration, Instant};

use glow::HasContext;
use gtk::prelude::*;
use webkit2gtk::{SettingsExt, URISchemeRequestExt, UserContentInjectedFrames, UserContentManagerExt, UserScript, UserScriptInjectionTime, WebContextExt, WebViewExt};
use solidcraft_engine::Session;
use solidcraft_render::{Camera, Mat4, Scene};

const VS: &str = r#"#version 330 core
layout(location = 0) in vec3 a_pos;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec3 a_color;
uniform mat4 u_mvp;
out vec3 v_normal;
out vec3 v_color;
void main() {
    v_normal = a_normal;
    v_color = a_color;
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}"#;

const FS: &str = r#"#version 330 core
in vec3 v_normal;
in vec3 v_color;
out vec4 out_color;
void main() {
    vec3 n = normalize(v_normal);
    float light = max(dot(n, normalize(vec3(0.4, 0.6, 1.0))), 0.0) * 0.75 + 0.25;
    out_color = vec4(v_color * light, 1.0);
}"#;

struct Render {
    gl: glow::Context,
    program: glow::Program,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    count: i32,
    cam: Camera,
    radius: f64,
    last: Instant,
    frames: u32,
    log: bool,
}

impl Render {
    fn new(gl: glow::Context, scene: &Scene, cam: Camera) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            let vs = shader(&gl, glow::VERTEX_SHADER, VS)?;
            let fs = shader(&gl, glow::FRAGMENT_SHADER, FS)?;
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
            let mut data: Vec<f32> = Vec::new();
            for m in &scene.meshes {
                let mesh = &m.mesh;
                let c = m.color.f();
                for tri in &mesh.triangles {
                    for &i in tri {
                        let Some(p) = mesh.positions.get(i as usize) else { continue };
                        let n = mesh.normals.get(i as usize).copied().unwrap_or_default();
                        data.extend_from_slice(&[p.x as f32, p.y as f32, p.z as f32, n.x as f32, n.y as f32, n.z as f32, c[0] as f32, c[1] as f32, c[2] as f32]);
                    }
                }
            }
            let bytes: &[u8] = core::slice::from_raw_parts(data.as_ptr().cast(), data.len() * 4);
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STATIC_DRAW);
            let stride = 9 * 4;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 3, glow::FLOAT, false, stride, 3 * 4);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 3, glow::FLOAT, false, stride, 6 * 4);
            gl.enable(glow::DEPTH_TEST);
            gl.enable(glow::CULL_FACE);
            gl.clear_color(0.055, 0.065, 0.08, 1.0);
            Ok(Self {
                gl,
                program,
                vao,
                vbo,
                count: i32::try_from(data.len() / 9).unwrap_or(0),
                cam,
                radius: scene.radius.max(1.0),
                last: Instant::now(),
                frames: 0,
                log: std::env::var_os("PROTO_LOG").is_some(),
            })
        }
    }

    fn draw(&mut self, width: i32, height: i32) {
        unsafe {
            self.gl.viewport(0, 0, width.max(1), height.max(1));
            self.gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            self.gl.use_program(Some(self.program));
            self.gl.bind_vertex_array(Some(self.vao));
            let aspect = f64::from(width.max(1)) / f64::from(height.max(1));
            let mvp: Mat4 = self.cam.view_proj(aspect, self.radius);
            let location = self.gl.get_uniform_location(self.program, "u_mvp");
            self.gl.uniform_matrix_4_f32_slice(location.as_ref(), false, &mvp.to_f32().concat());
            self.gl.draw_arrays(glow::TRIANGLES, 0, self.count);
        }
        self.frames += 1;
        let now = Instant::now();
        if now.duration_since(self.last) >= Duration::from_secs(1) {
            if self.log {
                println!("PROTO frame fps={} last_frame_ms={:.3}", self.frames, now.duration_since(self.last).as_secs_f64() * 1000.0 / f64::from(self.frames.max(1)));
            }
            self.frames = 0;
            self.last = now;
        }
    }
}

fn shader(gl: &glow::Context, kind: u32, source: &str) -> Result<glow::Shader, String> {
    unsafe {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, source);
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            return Err(gl.get_shader_info_log(shader));
        }
        Ok(shader)
    }
}

fn scene() -> Result<(Scene, Camera), String> {
    let mut session = Session::default();
    session.execute("solid.box", &serde_json::json!({"length": 80.0, "width": 50.0, "height": 30.0})).map_err(|e| e.to_string())?;
    let cam = solidcraft_engine::view::home_camera(&session).map_err(|e| e.to_string())?;
    let scene = solidcraft_engine::view::scene(&session, &cam).map_err(|e| e.to_string())?;
    Ok((scene, cam))
}

fn loader() -> *mut core::ffi::c_void {
    // GTK may load GL through a private loader, so its symbols need not be in RTLD_DEFAULT.
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

fn main() {
    if gtk::init().is_err() {
        eprintln!("PROTO: GTK could not start");
        return;
    }
    let (scene, cam) = match scene() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("PROTO: scene: {e}");
            return;
        }
    };
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("open-skyfall GTK host prototype");
    window.set_default_size(1400, 900);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    window.add(&row);

    let area = gtk::GLArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    area.set_can_focus(true);
    area.set_required_version(3, 3);
    area.add_events(gtk::gdk::EventMask::BUTTON_PRESS_MASK | gtk::gdk::EventMask::BUTTON_RELEASE_MASK | gtk::gdk::EventMask::POINTER_MOTION_MASK | gtk::gdk::EventMask::SCROLL_MASK);
    let no_gl = std::env::var("PROTO_NO_GL").is_ok_and(|v| v != "0");
    if no_gl {
        let placeholder = gtk::Label::new(Some("GTK host: GLArea omitted for the no-GL control"));
        placeholder.set_hexpand(true);
        placeholder.set_vexpand(true);
        row.pack_start(&placeholder, true, true, 0);
    } else {
        row.pack_start(&area, true, true, 0);
    }

    let dist = std::env::var("PROTO_DIST").unwrap_or_else(|_| {
        format!("{}/packages/assistant-web/dist", std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default())
    });
    let context = webkit2gtk::WebContext::new_ephemeral();
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
    let web = webkit2gtk::WebView::with_context(&context);
    if let Some(manager) = web.user_content_manager() {
        manager.register_script_message_handler("ipc");
        let shim = UserScript::new(
            "window.ipc = { postMessage: function (m) { window.webkit.messageHandlers.ipc.postMessage(m); } };",
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
    web.set_size_request(430, -1);
    web.connect_size_allocate(|w, a| println!("PROTO web alloc {}x{} at {}x{}", a.width(), a.height(), a.x(), a.y()));
    web.set_hexpand(false);
    web.set_vexpand(true);
    row.pack_end(&web, false, true, 0);
    row.reorder_child(&web, 0);
    web.connect_load_failed(|_, _, uri, error| {
        eprintln!("PROTO load failed {uri}: {error}");
        false
    });
    let mut production = solidcraft_ui_egui::SolveApp::new(Session::default(), solidcraft_ui_egui::Services::default());
    let state_json = solidcraft_ui_egui::agent_chat::web::state(&mut production);
    println!("PROTO state bytes={} head={}", state_json.len(), state_json.chars().take(100).collect::<String>());
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
    if let Some(settings) = WebViewExt::settings(&web) {
        settings.set_enable_write_console_messages_to_stdout(true);
    }
    web.load_uri("openskyfall-assistant://localhost/index.html");
    let web_for_focus = web.clone();
    web.connect_button_press_event(move |_, _| {
        web_for_focus.grab_focus();
        gtk::glib::Propagation::Proceed
    });
    // Files dropped on the page: the real host would attach them; the prototype records that
    // GTK/WebKit received them.
    web.drag_dest_set(gtk::DestDefaults::ALL, &[], gtk::gdk::DragAction::COPY);
    web.drag_dest_add_uri_targets();
    web.connect_drag_data_received(|_, _, _, _, data, _, _| {
        let uris = data.uris();
        if !uris.is_empty() {
            println!("PROTO drop uris={uris:?}");
        }
    });

    let state: Rc<RefCell<Option<Render>>> = Rc::new(RefCell::new(None));
    let state_realize = state.clone();
    area.connect_realize(move |area| {
        area.make_current();
        let Some(_ctx) = area.context() else {
            eprintln!("PROTO: no GdkGLContext");
            return;
        };
        let handle = loader();
        if handle.is_null() {
            eprintln!("PROTO: no GL library");
            return;
        }
        let gl = unsafe { glow::Context::from_loader_function(|s| {
            let c = CString::new(s).unwrap_or_default();
            let p = libc::dlsym(handle, c.as_ptr());
            if p.is_null() { libc::dlsym(libc::RTLD_DEFAULT, c.as_ptr()) } else { p }
        }) };
        match Render::new(gl, &scene, cam) {
            Ok(r) => *state_realize.borrow_mut() = Some(r),
            Err(e) => eprintln!("PROTO: GL init: {e}"),
        }
    });

    let state_draw = state.clone();
    area.connect_render(move |area, _| {
        let _ = area.make_current();
        if let Some(r) = state_draw.borrow_mut().as_mut() {
            r.draw(area.allocated_width(), area.allocated_height());
        }
        gtk::glib::Propagation::Proceed
    });

    let drag = Rc::new(RefCell::new((0.0f64, 0.0f64)));
    let state_press = state.clone();
    let drag_press = drag.clone();
    area.connect_button_press_event(move |area, ev| {
        if let Some(r) = state_press.borrow().as_ref() {
            let _ = r;
        }
        *drag_press.borrow_mut() = ev.position();
        area.grab_focus();
        area.queue_render();
        gtk::glib::Propagation::Proceed
    });
    let drag_move = drag.clone();
    let state_move = state.clone();
    area.connect_motion_notify_event(move |area, ev| {
        let pressed = ev.state().contains(gtk::gdk::ModifierType::BUTTON1_MASK);
        if !pressed {
            return gtk::glib::Propagation::Proceed;
        }
        let (x, y) = ev.position();
        let mut d = drag_move.borrow_mut();
        let (dx, dy) = (x - d.0, y - d.1);
        *d = (x, y);
        if let Some(r) = state_move.borrow_mut().as_mut() {
            r.cam.yaw += dx * 0.01;
            r.cam.pitch = (r.cam.pitch + dy * 0.01).clamp(-1.5, 1.5);
        }
        area.queue_render();
        gtk::glib::Propagation::Proceed
    });
    let state_scroll = state.clone();
    area.connect_scroll_event(move |area, ev| {
        if let Some(r) = state_scroll.borrow_mut().as_mut() {
            let dy = ev.delta().1;
            r.cam.distance = (r.cam.distance * (1.0 + dy * 0.1)).clamp(5.0, 5000.0);
        }
        area.queue_render();
        gtk::glib::Propagation::Proceed
    });
    let state_key = state.clone();
    let area_key = area.clone();
    window.connect_key_press_event(move |_, ev| {
        let Some(key) = ev.keyval().name() else { return gtk::glib::Propagation::Proceed };
        let mut moved = false;
        if let Some(r) = state_key.borrow_mut().as_mut() {
            match key.as_str() {
                "Left" => { r.cam.yaw -= 0.08; moved = true; }
                "Right" => { r.cam.yaw += 0.08; moved = true; }
                "Up" => { r.cam.pitch = (r.cam.pitch + 0.08).clamp(-1.5, 1.5); moved = true; }
                "Down" => { r.cam.pitch = (r.cam.pitch - 0.08).clamp(-1.5, 1.5); moved = true; }
                "plus" | "equal" => { r.cam.distance = (r.cam.distance * 0.9).clamp(5.0, 5000.0); moved = true; }
                "minus" => { r.cam.distance = (r.cam.distance * 1.1).clamp(5.0, 5000.0); moved = true; }
                _ => {}
            }
        }
        if moved {
            area_key.queue_render();
        }
        gtk::glib::Propagation::Proceed
    });

    if std::env::var("PROTO_SPIN").is_ok_and(|v| v != "0") {
        let area_spin = area.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(8), move || {
            area_spin.queue_render();
            gtk::glib::ControlFlow::Continue
        });
    }

    // The page gets the keyboard first once the window is up: a native Wayland compositor has no
    // pointer injection here, so typing into the page can be driven with `wtype`.
    let web_focus = web.clone();
    let window_focus = window.clone();
    let area_focus = area.clone();
    gtk::glib::timeout_add_local(Duration::from_millis(700), move || {
        window_focus.set_focus(Some(&web_focus));
        web_focus.grab_focus();
        println!("PROTO focus web={} area={}", web_focus.has_focus(), area_focus.has_focus());
        web_focus.run_javascript("setTimeout(function(){var t=document.querySelector('textarea'); if(t){t.focus(); t.setAttribute('data-proto-focused','yes');}},100);", None::<&gtk::gio::Cancellable>, |_| {});
        gtk::glib::ControlFlow::Break
    });

    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();
    gtk::main();
}
