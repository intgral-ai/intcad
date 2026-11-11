//! The Assistant's web view (macOS, Linux and Windows): the conversation and the composer drawn by
//! a page in a native web view, laid over the place the Assistant panel gives up for it in the
//! app's own window. The page is built by `packages/assistant-web` and embedded here; it loads
//! nothing from outside the app. What it is told and what it may ask for is
//! `solidcraft_ui_egui::agent_chat::web`.
//!
//! On Linux the view is WebKitGTK in a child of the app's X11 window, an XWayland window
//! included. A native Wayland window has no such child: there the panel draws its own
//! conversation. winit does not run GTK, so GTK is started here and given its turns each frame.
//!
//! On Windows the view is WebView2 in a child window of the app's. WebView2 serves no scheme of
//! the app's own, so the page has an `http` address there that only this view answers, and it
//! keeps its files in the user's data folder. A machine without the WebView2 runtime has no view:
//! there the panel draws its own conversation.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use eframe::egui;
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::agent_chat::web;
use wry::dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize};
use wry::http::{Request, Response, header};

/// The page's own address: a scheme only this web view serves, from the files below.
const SCHEME: &str = "openskyfall-assistant";
const PAGE: &str = "openskyfall-assistant://localhost/index.html";

/// WebView2 (Windows) has the page at `http://<scheme>.<host>/…`: that is how wry serves a scheme
/// of the app's own there, and the address a navigation is asked about. The requests still come
/// to the scheme's handler, in the scheme's form.
const OVER_HTTP: bool = cfg!(target_os = "windows");

/// Agent output is untrusted text: the page may load only its own files and send nothing out.
/// The page's `<meta>` says the same; this header is the copy the page cannot change.
const POLICY: &str = "default-src 'none'; script-src 'self' openskyfall-assistant:; style-src 'self' openskyfall-assistant: 'unsafe-inline'; \
                      font-src 'self' openskyfall-assistant:; img-src 'self' openskyfall-assistant: data: blob:; connect-src 'none'; \
                      base-uri 'none'; form-action 'none'";

/// Outside what the page draws the view is seen through, and the panel's frame shows around the
/// page's rounded corners (macOS). A child window (X11, Windows) is not seen through: there the
/// view has the colour of the page's own surface behind it.
const SEE_THROUGH: bool = cfg!(target_os = "macos");

/// How often GTK is given its turns while the page is at work or has the keyboard, and while it
/// rests. Nothing of the page moves between turns, and each turn is a frame of the app.
#[cfg(target_os = "linux")]
const GTK_AT_WORK: std::time::Duration = std::time::Duration::from_millis(16);
#[cfg(target_os = "linux")]
const GTK_AT_REST: std::time::Duration = std::time::Duration::from_millis(100);
/// No more of GTK's turns than this at once: the app's own frame is waiting.
#[cfg(target_os = "linux")]
const GTK_TURNS: usize = 256;

/// The page as `packages/assistant-web` builds it (see its README.md): name, type, bytes.
const FILES: &[(&str, &str, &[u8])] = &[
    ("index.html", "text/html; charset=utf-8", include_bytes!("../../../packages/assistant-web/dist/index.html")),
    ("assistant.js", "text/javascript; charset=utf-8", include_bytes!("../../../packages/assistant-web/dist/assistant.js")),
    ("style.css", "text/css; charset=utf-8", include_bytes!("../../../packages/assistant-web/dist/style.css")),
    ("CommitMono-400-Regular.woff2", "font/woff2", include_bytes!("../../../packages/assistant-web/dist/CommitMono-400-Regular.woff2")),
    ("CommitMono-700-Regular.woff2", "font/woff2", include_bytes!("../../../packages/assistant-web/dist/CommitMono-700-Regular.woff2")),
    ("MonaSans-VariableFont_wdth-wght.ttf", "font/ttf", include_bytes!("../../../packages/assistant-web/dist/MonaSans-VariableFont_wdth-wght.ttf")),
];

/// A picture is served as a picture and nothing else: opened on its own it could run nothing.
const PICTURE_POLICY: &str = "default-src 'none'; sandbox";

/// The conversation's pictures by the name the page is told for each, for the page's requests
/// (which are answered off the app's own state).
type Pictures = Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>;

/// What a drag over the web view came to.
enum Drag {
    /// Files are over the view, or no longer are.
    Over(bool),
    Dropped(Vec<PathBuf>),
}

/// A line about the web view on the app's error output, where the app's other notes go.
fn note(text: &str) {
    eprintln!("solidcraft: assistant web view: {text}");
}

fn respond(status: u16, kind: &str, policy: &str, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, kind)
        .header(header::CONTENT_SECURITY_POLICY, policy)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&b""[..])))
}

fn not_found() -> Response<Cow<'static, [u8]>> {
    respond(404, "text/plain; charset=utf-8", POLICY, Cow::Borrowed(b"not found"))
}

/// One of the page's files, or one of the conversation's pictures, for a request to the page's
/// scheme; anything else is not found.
fn serve(request: &Request<Vec<u8>>, pictures: &Pictures) -> Response<Cow<'static, [u8]>> {
    let name = request.uri().path().trim_start_matches('/');
    if let Some(key) = name.strip_prefix("picture/") {
        let bytes = pictures.lock().ok().and_then(|held| held.get(key).cloned());
        return match bytes.as_deref().and_then(|bytes| picture_kind(bytes).map(|kind| (kind, bytes))) {
            Some((kind, bytes)) => respond(200, kind, PICTURE_POLICY, Cow::Owned(bytes.clone())),
            None => not_found(),
        };
    }
    match FILES.iter().find(|(file, _, _)| *file == name) {
        Some((_, kind, bytes)) => respond(200, kind, POLICY, Cow::Borrowed(bytes)),
        None => not_found(),
    }
}

/// What kind of picture the bytes begin as, of the kinds messages are sent with. Anything else
/// is not served: the page is given a picture to draw, never a document to open.
fn picture_kind(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("image/webp")
    } else {
        None
    }
}

/// The page's address as the view has it (see [`OVER_HTTP`]).
fn page_address(over_http: bool) -> String {
    if over_http { PAGE.replacen(&format!("{SCHEME}://"), &format!("http://{SCHEME}."), 1) } else { PAGE.to_string() }
}

/// The page stays the page: nothing else loads in the view, not even another of its own files.
fn own_address(url: &str, over_http: bool) -> bool {
    url == page_address(over_http) || url == "about:blank"
}

/// Where WebView2 keeps its own files (Windows). Left to itself it makes a folder beside the
/// app's exe, which an installed app may not write to.
fn data_folder() -> Option<PathBuf> {
    // A run that was given a settings folder of its own keeps the view's files there as well.
    let folder = if std::env::var_os("SOLIDCRAFT_SETTINGS_DIR").is_some() {
        crate::settings_dir().ok().flatten()
    } else {
        solidcraft_engine::recovery::data_dir()
    };
    folder.map(|folder| folder.join("assistant-web-view"))
}

/// Whether this run's window can hold the web view at all.
fn available() -> bool {
    // On Linux the view is a child of an X11 window; a native Wayland window has no such child.
    #[cfg(target_os = "linux")]
    return crate::window_system_now() == Some("x11");
    #[cfg(not(target_os = "linux"))]
    true
}

/// Start GTK for the web view (Linux), once: `false` when it cannot start.
#[cfg(target_os = "linux")]
fn start_gtk() -> bool {
    // In a Wayland session the app's window is an XWayland one, and GDK left to choose opens the
    // Wayland display. The view is a child of the app's X11 window, on the X11 display.
    gtk::gdk::set_allowed_backends("x11");
    // GTK would set the process to the user's locale; the app reads and writes numbers in C's.
    gtk::disable_setlocale();
    match gtk::init() {
        Ok(()) => true,
        Err(error) => {
            note(&format!("GTK could not start: {error}"));
            false
        }
    }
}

/// Where the view is in the window and at what size its page is drawn, as last given to it.
#[derive(Clone, Copy, PartialEq)]
struct Placed {
    rect: egui::Rect,
    zoom: f32,
    pixels_per_point: f32,
}

#[derive(Default)]
pub struct AssistantWeb {
    view: Option<wry::WebView>,
    /// What the page sent since the last frame.
    inbox: Rc<RefCell<Vec<String>>>,
    /// Files dragged over the view or dropped on it since the last frame.
    drags: Rc<RefCell<Vec<Drag>>>,
    /// The pictures the page was last told of.
    pictures: Pictures,
    placed: Option<Placed>,
    /// The colour last put behind the page, where the view is not seen through.
    backed: Option<egui::Color32>,
    shown: bool,
    /// The state the page was last told, to tell it only changes.
    told: String,
    /// The page has the keyboard.
    focused: bool,
    /// GTK was started for the view, and takes turns.
    #[cfg(target_os = "linux")]
    gtk: bool,
    /// GTK had something to do in a turn since the last frame.
    #[cfg(target_os = "linux")]
    at_work: bool,
}

impl AssistantWeb {
    /// The host has a web view to offer: the panel may give its conversation up to it.
    pub fn new(solve: &mut SolveApp) -> Self {
        web::set_host(solve, available());
        Self::default()
    }

    /// The view goes before the window it is a child of does.
    pub fn close(&mut self) {
        self.view = None;
        self.placed = None;
        self.shown = false;
        #[cfg(target_os = "linux")]
        self.turn_gtk();
    }

    /// GTK's turns (Linux): what the page drew, sent and was told moves in them, and the app's
    /// own event loop gives it none.
    #[cfg(target_os = "linux")]
    fn turn_gtk(&mut self) {
        if !self.gtk {
            return;
        }
        let mut turns = 0;
        while turns < GTK_TURNS && gtk::events_pending() {
            gtk::main_iteration_do(false);
            turns += 1;
        }
        self.at_work |= turns > 0;
    }

    /// The page has the keyboard: text editing keys and menu items belong to it.
    #[cfg(target_os = "macos")]
    pub fn focused(&self) -> bool {
        self.focused && self.shown
    }

    /// Undo (or redo) the text being typed in the page: the Edit menu's Undo while the page has
    /// the keyboard.
    #[cfg(target_os = "macos")]
    pub fn edit_text(&self, redo: bool) {
        if let Some(view) = &self.view
            && let Err(error) = view.evaluate_script(if redo { "document.execCommand('redo')" } else { "document.execCommand('undo')" })
        {
            note(&format!("could not undo in the page: {error}"));
        }
    }

    /// With the app's own logic, which also runs while the window is not painted: act on what
    /// the page asked and tell it what changed, so the conversation never waits for a repaint.
    pub fn logic(&mut self, solve: &mut SolveApp, ctx: &egui::Context) {
        #[cfg(target_os = "linux")]
        self.turn_gtk();
        web::advance(solve, ctx);
        let messages: Vec<String> = self.inbox.borrow_mut().drain(..).collect();
        for message in messages {
            self.receive(solve, ctx, &message);
        }
        let drags: Vec<Drag> = self.drags.borrow_mut().drain(..).collect();
        for drag in drags {
            match drag {
                Drag::Over(over) => web::dragging(solve, over),
                Drag::Dropped(paths) => web::dropped(solve, paths),
            }
        }
        self.tell(solve);
    }

    /// Tell the shown page the Assistant's state when it changed.
    fn tell(&mut self, solve: &mut SolveApp) {
        let Some(view) = self.view.as_ref().filter(|_| self.shown) else { return };
        let state = web::state(solve);
        if state != self.told {
            // Before the page hears of a picture, the host can serve it.
            if let Ok(mut held) = self.pictures.lock() {
                *held = web::pictures(solve).into_iter().collect();
            }
            match view.evaluate_script(&format!("window.__assistantHost&&window.__assistantHost.state({state})")) {
                Ok(()) => self.told = state,
                Err(error) => note(&format!("could not be told the state: {error}")),
            }
        }
    }

    /// Once a frame, after the app's UI: keep the web view on the place the panel gave it.
    pub fn tick(&mut self, solve: &mut SolveApp, ctx: &egui::Context, frame: &eframe::Frame) {
        self.keep_place(solve, ctx, frame);
        // WebView2 takes the keyboard when it is clicked, and a press on the rest of the window
        // does not take it back (Windows): the window is given it. The app sees only the presses
        // that are not on the view. (The answer names the window that had the keyboard, and
        // "none" reads as an error.)
        if cfg!(target_os = "windows")
            && ctx.input(|input| input.pointer.any_pressed())
            && let Some(view) = self.view.as_ref().filter(|_| self.shown)
        {
            let _ = view.focus_parent();
        }
        #[cfg(target_os = "linux")]
        {
            self.turn_gtk();
            // The next turn: soon while the page is at work or typed in, seldom while it rests.
            if self.shown {
                let at_work = std::mem::take(&mut self.at_work) || self.focused;
                ctx.request_repaint_after(if at_work { GTK_AT_WORK } else { GTK_AT_REST });
            }
        }
    }

    fn keep_place(&mut self, solve: &mut SolveApp, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(rect) = web::placement(solve, ctx) else {
            self.show(false);
            return;
        };
        let at = Placed { rect, zoom: ctx.zoom_factor(), pixels_per_point: ctx.pixels_per_point() };
        let backing = web::backing(solve, ctx);
        if self.view.is_none() && !self.create(ctx, frame, at, backing) {
            // No web view on this machine: the panel draws its own conversation again.
            web::set_host(solve, false);
            ctx.request_repaint();
            return;
        }
        if self.placed != Some(at)
            && let Some(view) = &self.view
        {
            if let Err(error) = view.set_bounds(place(at)) {
                note(&format!("could not be moved: {error}"));
            }
            // The page is drawn at the app's own size: a point of the page is a point of the app.
            if self.placed.is_none_or(|before| (before.zoom, before.pixels_per_point) != (at.zoom, at.pixels_per_point))
                && let Err(error) = view.zoom(page_zoom(view, at))
            {
                note(&format!("could not be zoomed: {error}"));
            }
            self.placed = Some(at);
        }
        // Behind a view that is not seen through: the page's own surface, once the page said it.
        if !SEE_THROUGH
            && self.backed != Some(backing)
            && let Some(view) = &self.view
        {
            if let Err(error) = view.set_background_color(opaque(backing)) {
                note(&format!("could not be given its background: {error}"));
            }
            self.backed = Some(backing);
        }
        self.show(true);
        #[cfg(target_os = "linux")]
        if let Some(view) = &self.view {
            fit(view, at);
        }
        self.tell(solve);
    }

    fn receive(&mut self, solve: &mut SolveApp, ctx: &egui::Context, message: &str) {
        let value = serde_json::from_str::<serde_json::Value>(message).unwrap_or_default();
        match value.get("type").and_then(serde_json::Value::as_str) {
            // The page (re)started: it knows nothing yet.
            Some("ready") => {
                self.told.clear();
            }
            Some("focus") => self.focused = value.get("on").and_then(serde_json::Value::as_bool).unwrap_or(false),
            Some("log") => note(&value.get("text").and_then(serde_json::Value::as_str).unwrap_or_default().chars().take(600).collect::<String>()),
            _ => {
                if let Err(reason) = web::act(solve, ctx, message) {
                    note(&format!("refused: {reason}"));
                }
            }
        }
    }

    fn create(&mut self, ctx: &egui::Context, frame: &eframe::Frame, at: Placed, backing: egui::Color32) -> bool {
        #[cfg(target_os = "linux")]
        {
            if !self.gtk && !start_gtk() {
                return false;
            }
            self.gtk = true;
        }
        let inbox = self.inbox.clone();
        let repaint = ctx.clone();
        let (drags, dragged) = (self.drags.clone(), ctx.clone());
        let pictures = self.pictures.clone();
        // Only files are the app's to take; text dragged within the page stays the page's.
        let files = Cell::new(false);
        // WebView2 is told where its files go (Windows); the other views keep theirs where the
        // system has them. It reads this as the view is built.
        let mut context = cfg!(target_os = "windows").then(|| {
            let folder = data_folder();
            if let Some(folder) = &folder
                && let Err(error) = std::fs::create_dir_all(folder)
            {
                note(&format!("could not make its data folder {}: {error}", folder.display()));
            }
            wry::WebContext::new(folder)
        });
        let builder = match context.as_mut() {
            Some(context) => wry::WebViewBuilder::new_with_web_context(context),
            None => wry::WebViewBuilder::new(),
        };
        let builder = builder
            .with_custom_protocol(SCHEME.to_string(), move |_view, request| serve(&request, &pictures))
            .with_url(PAGE)
            .with_bounds(place(at))
            .with_accept_first_mouse(true)
            .with_navigation_handler(|url| {
                let own = own_address(&url, OVER_HTTP);
                if !own {
                    note(&format!("would not go to {}", url.chars().take(200).collect::<String>()));
                }
                own
            })
            .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
            // The paths come from the system's drag, never from the page.
            .with_drag_drop_handler(move |event| {
                match event {
                    wry::DragDropEvent::Enter { paths, .. } => {
                        files.set(!paths.is_empty());
                        if files.get() {
                            drags.borrow_mut().push(Drag::Over(true));
                        }
                    }
                    wry::DragDropEvent::Drop { paths, .. } => {
                        files.set(false);
                        if paths.is_empty() {
                            return false;
                        }
                        drags.borrow_mut().push(Drag::Dropped(paths));
                        dragged.request_repaint();
                        return true;
                    }
                    wry::DragDropEvent::Leave => {
                        if files.replace(false) {
                            drags.borrow_mut().push(Drag::Over(false));
                        }
                    }
                    // Moving over the view changes nothing the app shows.
                    _ => return files.get(),
                }
                dragged.request_repaint();
                files.get()
            })
            .with_ipc_handler(move |message| {
                inbox.borrow_mut().push(message.into_body());
                repaint.request_repaint();
            });
        // See-through outside what the page draws, where a view can be: the page rounds its own
        // corners, and the panel's frame (its border and shadow) shows around them.
        let builder = if SEE_THROUGH { builder.with_transparent(true) } else { builder.with_background_color(opaque(backing)) };
        // A browser's own keys are not the Assistant's (Windows): F5 would load the page again
        // and Ctrl+P print it. The keys of text editing stay (WebView2's
        // `AreBrowserAcceleratorKeysEnabled`, as Microsoft describes it).
        #[cfg(target_os = "windows")]
        let builder = {
            use wry::WebViewBuilderExtWindows;
            builder.with_browser_accelerator_keys(false)
        };
        // The builder panics on a window it cannot take; the app goes on without the web view.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build_as_child(frame)));
        match built {
            Ok(Ok(view)) => {
                if let Err(error) = view.zoom(page_zoom(&view, at)) {
                    note(&format!("could not be zoomed: {error}"));
                }
                self.view = Some(view);
                self.placed = Some(at);
                self.backed = Some(backing);
                self.shown = true;
                true
            }
            Ok(Err(error)) => {
                note(&format!("could not be created: {error}"));
                false
            }
            Err(_) => {
                note("could not be created in this window");
                false
            }
        }
    }

    fn show(&mut self, shown: bool) {
        if self.shown == shown {
            return;
        }
        if let Some(view) = &self.view {
            if let Err(error) = view.set_visible(shown) {
                note(&format!("could not be {}: {error}", if shown { "shown" } else { "hidden" }));
            }
            // A hidden page cannot keep the keyboard: it goes back to the window. (An X11 child
            // never took it from the window, and is not to be given it as it is hidden. WebView2
            // is not asked whether it had it: the window having it twice costs nothing.)
            if !shown && (self.focused || cfg!(target_os = "windows")) {
                #[cfg(not(target_os = "linux"))]
                let _ = view.focus_parent();
                self.focused = false;
            }
        }
        self.shown = shown;
    }
}

/// The view's place in the window. On macOS it is given in the window's own points. Elsewhere it
/// is given in pixels: WebKitGTK counts points by GTK's scale, which need not be the window's,
/// and a child window on Windows is placed in pixels anyway.
fn place(at: Placed) -> wry::Rect {
    if cfg!(target_os = "macos") { bounds(at.rect, at.zoom) } else { bounds_in_pixels(at.rect, at.pixels_per_point) }
}

/// An egui rectangle as the web view's place in the window: egui points times the UI zoom are the
/// window's own points.
fn bounds(rect: egui::Rect, zoom: f32) -> wry::Rect {
    let zoom = f64::from(zoom);
    wry::Rect {
        position: LogicalPosition::new(f64::from(rect.left()) * zoom, f64::from(rect.top()) * zoom).into(),
        size: LogicalSize::new(f64::from(rect.width()) * zoom, f64::from(rect.height()) * zoom).into(),
    }
}

/// An egui rectangle as the web view's place in the window's pixels: each edge on the pixel egui
/// draws it on, so the view neither leaves a gap beside the panel's frame nor covers it.
fn bounds_in_pixels(rect: egui::Rect, pixels_per_point: f32) -> wry::Rect {
    let pixel = |point: f32| (f64::from(point) * f64::from(pixels_per_point)).round();
    let (left, top) = (pixel(rect.left()), pixel(rect.top()));
    wry::Rect {
        position: PhysicalPosition::new(left, top).into(),
        size: PhysicalSize::new((pixel(rect.right()) - left).max(1.0), (pixel(rect.bottom()) - top).max(1.0)).into(),
    }
}

/// Give the page the size of its place (Linux), every frame it is shown. wry resizes the X11
/// window the view is in, but GTK goes on laying the page out at the size that window was made
/// with, and does so again whenever the view is shown: the page's widget is given its size here,
/// which costs nothing while it has it.
#[cfg(target_os = "linux")]
fn fit(view: &wry::WebView, at: Placed) {
    use gtk::prelude::WidgetExt;
    use wry::WebViewExtUnix;
    let page = view.webview();
    let size = bounds_in_pixels(at.rect, at.pixels_per_point).size.to_logical::<i32>(f64::from(page.scale_factor().max(1)));
    page.size_allocate(&gtk::Allocation::new(0, 0, size.width.max(1), size.height.max(1)));
}

/// The page's zoom at which a point of the page is a point of the app.
fn page_zoom(view: &wry::WebView, at: Placed) -> f64 {
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::WidgetExt;
        use wry::WebViewExtUnix;
        zoom_over(at.pixels_per_point, view.webview().scale_factor())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = view;
        f64::from(at.zoom)
    }
}

/// The page's zoom where the engine draws a page point as a whole number of pixels (GTK's scale)
/// and the app draws a point as `pixels_per_point` of them, a fraction on many screens.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn zoom_over(pixels_per_point: f32, engine_scale: i32) -> f64 {
    f64::from(pixels_per_point) / f64::from(engine_scale.max(1))
}

/// A colour with nothing seen through it, as the web view takes one.
fn opaque(colour: egui::Color32) -> wry::RGBA {
    (colour.r(), colour.g(), colour.b(), 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_with(path: &str, pictures: &Pictures) -> Response<Cow<'static, [u8]>> {
        serve(&Request::builder().uri(format!("openskyfall-assistant://localhost{path}")).body(Vec::new()).unwrap(), pictures)
    }

    fn get(path: &str) -> Response<Cow<'static, [u8]>> {
        get_with(path, &Pictures::default())
    }

    /// The page's files are served with their types and the policy that keeps the page from
    /// loading or sending anything else; a path that is not one of them is not found.
    #[test]
    fn the_page_is_served_from_the_embedded_files_only() {
        let page = get("/index.html");
        assert_eq!(page.status(), 200);
        assert_eq!(page.headers()[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert!(page.headers()[header::CONTENT_SECURITY_POLICY].to_str().unwrap().contains("connect-src 'none'"));
        assert!(std::str::from_utf8(page.body()).unwrap().contains("assistant.js"), "the page loads its script by its fixed name");
        for (name, kind, bytes) in FILES {
            let file = get(&format!("/{name}"));
            assert_eq!(file.status(), 200, "{name}");
            assert_eq!(file.headers()[header::CONTENT_TYPE], *kind, "{name}");
            assert_eq!(file.body().len(), bytes.len(), "{name}");
        }
        for path in ["/", "/../Cargo.toml", "/index.html/x", "/assistant.js.map", "/%2e%2e/secret"] {
            assert_eq!(get(path).status(), 404, "{path}");
        }
    }

    /// A picture of the conversation is served by the name the page was told, as the kind of
    /// picture its bytes begin as and under a policy of its own; a name the page was not told,
    /// and bytes that are no picture, are not found.
    #[test]
    fn a_conversations_picture_is_served_as_a_picture_only() {
        let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0; 16]].concat();
        let webp = [b"RIFF".as_slice(), &[0; 4], b"WEBPVP8 "].concat();
        let pictures = Pictures::default();
        pictures.lock().unwrap().extend([
            ("a-1".to_string(), Arc::new(png.clone())),
            ("b-2".to_string(), Arc::new(webp)),
            ("c-3".to_string(), Arc::new(b"<html><script>alert(1)</script>".to_vec())),
            ("d-4".to_string(), Arc::new(b"<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>".to_vec())),
        ]);
        let shown = get_with("/picture/a-1", &pictures);
        assert_eq!(shown.status(), 200);
        assert_eq!(shown.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(shown.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(shown.headers()[header::CONTENT_SECURITY_POLICY], PICTURE_POLICY);
        assert_eq!(shown.body().as_ref(), png.as_slice());
        assert_eq!(get_with("/picture/b-2", &pictures).headers()[header::CONTENT_TYPE], "image/webp");
        for path in ["/picture/c-3", "/picture/d-4", "/picture/e-5", "/picture/", "/picture/a-1/x", "/picture/../index.html"] {
            assert_eq!(get_with(path, &pictures).status(), 404, "{path}");
        }
        assert_eq!(get("/picture/a-1").status(), 404, "a picture the page was not told of");
        for (bytes, kind) in
            [(&[0xff_u8, 0xd8, 0xff, 0xe0][..], Some("image/jpeg")), (b"GIF89a..", Some("image/gif")), (b"RIFF", None), (b"BM..", None), (b"", None)]
        {
            assert_eq!(picture_kind(bytes), kind, "{bytes:?}");
        }
    }

    /// Where the view is placed in pixels, its edges are the pixels egui draws the panel's on, at
    /// a whole scale and at a fraction; and the page is zoomed by what the engine's own scale
    /// leaves of the app's.
    #[test]
    fn the_view_is_placed_on_the_pixels_of_its_panel() {
        let rect = egui::Rect::from_min_max(egui::pos2(900.5, 40.0), egui::pos2(1260.5, 700.25));
        for (pixels_per_point, expected) in [(1.0, (901, 40, 360, 660)), (1.25, (1126, 50, 450, 825)), (2.0, (1801, 80, 720, 1321))] {
            let placed = bounds_in_pixels(rect, pixels_per_point);
            let position = placed.position.to_physical::<i32>(1.0);
            let size = placed.size.to_physical::<u32>(1.0);
            assert_eq!((position.x, position.y, size.width, size.height), expected, "at {pixels_per_point}");
        }
        let nothing = bounds_in_pixels(egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(10.0, 10.0)), 1.5).size.to_physical::<u32>(1.0);
        assert_eq!((nothing.width, nothing.height), (1, 1), "an X11 window has a size");
        assert_eq!(zoom_over(1.0, 1), 1.0);
        assert_eq!(zoom_over(1.25, 1), 1.25);
        assert_eq!(zoom_over(2.5, 2), 1.25);
        assert_eq!(zoom_over(1.5, 0), 1.5, "a scale GTK did not give");
        assert_eq!(opaque(egui::Color32::from_rgba_unmultiplied(10, 20, 30, 40)).3, 255);
    }

    /// Nothing but the page's own address loads in the view: in the scheme's form where the
    /// view serves the scheme, and in WebView2's `http` form of it on Windows, where the other
    /// form is no address of the page's.
    #[test]
    fn only_the_pages_own_address_loads() {
        // What wry makes of the scheme for WebView2, and what a navigation there is asked about.
        const HTTP: &str = "http://openskyfall-assistant.localhost/index.html";
        assert_eq!(page_address(false), PAGE);
        assert_eq!(page_address(true), HTTP);
        assert!(PAGE.starts_with(&format!("{SCHEME}://localhost/")), "a host WebView2 does not take for a site on the net");
        for (own, over_http) in [(PAGE, false), (HTTP, true)] {
            assert!(own_address(own, over_http), "{own}");
            assert!(own_address("about:blank", over_http));
            for url in [
                "https://example.com/",
                "http://127.0.0.1:8080/",
                "http://localhost/index.html",
                "file:///etc/passwd",
                "openskyfall-assistant://elsewhere/index.html",
                "openskyfall-assistant://localhost/assistant.js",
                "openskyfall-assistant://localhost/picture/a-1",
                "openskyfall-assistant://localhost/index.html/../picture/a-1",
                "https://openskyfall-assistant.localhost/index.html",
                "http://openskyfall-assistant.localhost.example.com/index.html",
                "http://openskyfall-assistant.localhost:8080/index.html",
                "http://openskyfall-assistant.localhost/assistant.js",
                "http://openskyfall-assistant.localhost/picture/a-1",
                "http://openskyfall-assistant.localhost/index.html?x",
                "http://openskyfall-assistant.elsewhere/index.html",
                "javascript:alert(1)",
            ] {
                assert!(!own_address(url, over_http), "{url}");
            }
        }
        assert!(!own_address(HTTP, false), "the scheme's own form only, where the view serves the scheme");
        assert!(!own_address(PAGE, true), "WebView2 never has the page there");
    }

    /// WebView2's files go under a folder of the app's, never beside the exe; a run with a
    /// settings folder of its own keeps them there.
    #[test]
    fn webview2_keeps_its_files_in_a_folder_of_the_apps() {
        if let Some(folder) = data_folder() {
            assert!(folder.is_absolute(), "{folder:?}");
            assert!(folder.ends_with("assistant-web-view"), "{folder:?}");
        }
    }
}
