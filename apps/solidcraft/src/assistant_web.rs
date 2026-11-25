//! The Assistant's web view (macOS, Linux and Windows): the conversation and the composer drawn by
//! a page in a native web view, laid over the place the Assistant panel gives up for it in the
//! app's own window. The page is built by `packages/assistant-web` and embedded here; it loads
//! nothing from outside the app. What it is told and what it may ask for is
//! `solidcraft_ui_egui::agent_chat::web`.
//!
//! On Linux the view is WebKitGTK in a child of the app's X11 window, which in a Wayland session
//! is an XWayland window: the one the app opens there unless it is told otherwise. A native
//! Wayland window has no X11 child to give it: the panel says so and offers Graphics settings and
//! Retry, never a second conversation (docs/native-wayland.md). winit does not run GTK, so GTK is
//! started here and given its turns each frame. The X server sends typed keys to the window that
//! has the keyboard, the app's, so the view's window is given it in GTK's press callback, before
//! the first key, and the app's window has it back when the rest of it is pressed.
//!
//! On Windows the view is WebView2 in a child window of the app's. WebView2 serves no scheme of
//! the app's own, so the page has an `http` address there that only this view answers, and it
//! keeps its files in the user's data folder. A machine without the WebView2 runtime has no view:
//! the panel names it and offers Retry and Assistant settings.

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

#[cfg(target_os = "linux")]
mod shape;

#[cfg(target_os = "linux")]
#[derive(serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PageShape {
    width: f64,
    height: f64,
    popup: Option<u32>,
    surfaces: Vec<[f64; 12]>,
}

#[cfg(target_os = "linux")]
impl PageShape {
    fn decode(value: serde_json::Value, bytes: usize) -> Option<Self> {
        if bytes > 4096 {
            return None;
        }
        serde_json::from_value::<Self>(value).ok().filter(|report| report.surfaces.len() <= 8)
    }
}

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
/// While the page has the keyboard: a key, a composition or a candidate list is not kept waiting
/// for a turn. The app's own frame is cheap here (the page, not the model, is what changes), and
/// the rate falls back to [`GTK_AT_REST`] as soon as the app's window is pressed again.
#[cfg(target_os = "linux")]
const GTK_WITH_KEYBOARD: std::time::Duration = std::time::Duration::from_millis(16);
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

/// The keyboard between the app's X11 window and the view's (Linux). The view's window is a
/// child of the app's, made by another toolkit, and never has the keyboard of its own accord:
/// until it is given it, typed keys reach the page only while the pointer is over it.
#[cfg(target_os = "linux")]
struct Keyboard {
    /// A connection of its own: neither winit's nor GTK's is reachable.
    connection: x11rb::rust_connection::RustConnection,
    app: u32,
    view: u32,
}

#[cfg(target_os = "linux")]
impl Keyboard {
    /// The app's X11 window and the windows inside it as they are now, before the view is made.
    fn before(frame: &eframe::Frame) -> Option<(x11rb::rust_connection::RustConnection, u32, Vec<u32>)> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let app = match frame.window_handle().ok()?.as_raw() {
            RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok()?,
            RawWindowHandle::Xcb(handle) => handle.window.get(),
            _ => return None,
        };
        let (connection, _) = x11rb::connect(None).ok()?;
        let inside = Self::inside(&connection, app)?;
        Some((connection, app, inside))
    }

    fn inside(connection: &x11rb::rust_connection::RustConnection, app: u32) -> Option<Vec<u32>> {
        use x11rb::protocol::xproto::ConnectionExt;
        Some(connection.query_tree(app).ok()?.reply().ok()?.children)
    }

    /// With the view made: its window is the one that was not in the app's window before.
    fn of(before: Option<(x11rb::rust_connection::RustConnection, u32, Vec<u32>)>) -> Option<Self> {
        let (connection, app, before) = before?;
        let view = new_window(&before, &Self::inside(&connection, app)?)?;
        Some(Self { connection, app, view })
    }

    fn give(&self, window: u32) -> Result<(), String> {
        use x11rb::protocol::xproto::{ConnectionExt, InputFocus};
        // Should the window stop being shown, the keyboard goes to the window it is in.
        let asked = self.connection.set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME).map_err(|error| error.to_string())?;
        asked.check().map_err(|error| error.to_string())
    }
}

/// The window that is among `after` and was not among `before`, when there is exactly one.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn new_window(before: &[u32], after: &[u32]) -> Option<u32> {
    let mut new = after.iter().copied().filter(|window| !before.contains(window));
    let window = new.next()?;
    new.next().is_none().then_some(window)
}

/// Where the view is in the window and at what size its page is drawn, as last given to it.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Placed {
    rect: egui::Rect,
    zoom: f32,
    pixels_per_point: f32,
    /// How much of the view, from its foot, is shown and takes presses (`web::reveal`); `None`: all.
    reveal: Option<f32>,
}

#[derive(Default)]
pub struct AssistantWeb {
    view: Option<wry::WebView>,
    /// The frame that shows only the view's foot (macOS, `assistant_clip`). Without it the view
    /// changes size as the page grows, as it does on the other systems.
    #[cfg(target_os = "macos")]
    clip: Option<crate::assistant_clip::Clip>,
    /// What the page sent since the last frame.
    inbox: Rc<RefCell<Vec<String>>>,
    /// Files dragged over the view or dropped on it since the last frame.
    drags: Rc<RefCell<Vec<Drag>>>,
    /// The pictures the page was last told of.
    pictures: Pictures,
    placed: Option<Placed>,
    /// The place the panel gave the view most recently. The correction after GTK maps the view
    /// reads this, never the place it was made with: the window may have been tiled to another
    /// size by then, and putting the view back on the old place leaves the panel empty.
    #[cfg(target_os = "linux")]
    latest: Rc<Cell<Option<Placed>>>,
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
    /// The GTK window that holds the page: GDK's own toplevel, reparented into the app's X11
    /// window. wry's `build_as_child` makes a foreign GdkWindow that GTK never marks active,
    /// which costs the page its input method; a GDK window keeps focus.
    #[cfg(target_os = "linux")]
    gtk_window: Option<gtk::Window>,
    #[cfg(target_os = "linux")]
    gtk_scale: Option<i32>,
    #[cfg(target_os = "linux")]
    gtk_shape: Option<PageShape>,
    #[cfg(target_os = "linux")]
    applied_shape: Option<(i32, i32, i32, u64)>,
    #[cfg(target_os = "linux")]
    gtk_shape_dirty: Rc<Cell<bool>>,
    /// GTK had something to do in a turn since the last frame. Set by the page's draw, its IPC
    /// and its presses, not by every idle timer GTK happens to have: an Assistant at rest must
    /// not keep the app at 60 fps.
    #[cfg(target_os = "linux")]
    at_work: Rc<Cell<bool>>,
    /// The page drew since the last turn (GTK's `draw` signal on the web view).
    #[cfg(target_os = "linux")]
    drew: Rc<Cell<bool>>,
    /// The page has the X11 keyboard. Set in GTK's press callback, before the next frame, so the
    /// first key after a click is not lost to the hand-over.
    #[cfg(target_os = "linux")]
    page_focused: Rc<Cell<bool>>,
    /// The way to hand the keyboard over, once the view's window is known. Shared with the press
    /// callback, which gives it over at once.
    #[cfg(target_os = "linux")]
    keyboard: Rc<RefCell<Option<Keyboard>>>,
}

impl AssistantWeb {
    /// The host has a web view to offer: the panel may give its conversation up to it.
    pub fn new(solve: &mut SolveApp) -> Self {
        let available = available();
        if cfg!(target_os = "linux") && !available {
            note(
                "this run's window is a native Wayland one, which holds no X11 child window for WebKitGTK: the Assistant panel shows a diagnostic (Window system: Automatic or X11 in Preferences opens an X11 window)",
            );
        }
        web::set_host(solve, available);
        if !available {
            web::set_problem(solve, web::Problem::NativeWayland);
        }
        Self::default()
    }

    /// The view goes before the window it is a child of does.
    pub fn close(&mut self) {
        self.view = None;
        #[cfg(target_os = "macos")]
        {
            self.clip = None;
        }
        #[cfg(target_os = "linux")]
        {
            self.gtk_window = None;
        }
        self.placed = None;
        self.shown = false;
        #[cfg(target_os = "linux")]
        {
            *self.keyboard.borrow_mut() = None;
            self.page_focused.set(false);
            self.turn_gtk();
        }
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
        // A page that drew needs another turn soon (its compositor is still catching up); an
        // idle GTK timer does not. This is what keeps an untouched Assistant from pinning the
        // app to the display's frame rate.
        let drew = self.drew.replace(false);
        self.at_work.set(self.at_work.get() || drew);
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
        #[cfg(target_os = "linux")]
        if !messages.is_empty() {
            self.at_work.set(true);
        }
        for message in messages {
            self.receive(solve, ctx, &message);
        }
        let drags: Vec<Drag> = self.drags.borrow_mut().drain(..).collect();
        #[cfg(target_os = "linux")]
        if !drags.is_empty() {
            self.at_work.set(true);
        }
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
                Ok(()) => {
                    self.told = state;
                    #[cfg(target_os = "linux")]
                    self.at_work.set(true);
                }
                Err(error) => note(&format!("could not be told the state: {error}")),
            }
        }
    }

    /// Once a frame, after the app's UI: keep the web view on the place the panel gave it.
    pub fn tick(&mut self, solve: &mut SolveApp, ctx: &egui::Context, frame: &eframe::Frame) {
        // The panel's Retry: the conversation is the app's state, so a new view picks it up
        // whole; only the view and its place are made again.
        if web::take_retry(solve) {
            self.view = None;
            self.placed = None;
            self.shown = false;
            #[cfg(target_os = "linux")]
            {
                self.gtk_shape = None;
                self.applied_shape = None;
            }
            web::set_host(solve, true);
            ctx.request_repaint();
        }
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
            // A press on the page gives the view's window the keyboard in GTK's own callback
            // (`create`), not a frame later: a key typed at once was lost there. A press on the
            // rest of the app's window gives it back; the app sees only those presses.
            if self.page_focused.get() && ctx.input(|input| input.pointer.any_pressed()) {
                if let Some(keyboard) = self.keyboard.borrow().as_ref()
                    && let Err(error) = keyboard.give(keyboard.app)
                {
                    note(&format!("could not give the keyboard back: {error}"));
                }
                self.page_focused.set(false);
            }
            // The next turn: soon while the page is at work, seldom while it rests, and between
            // the two while it has the keyboard and waits for a key.
            if self.shown {
                let wait = if self.at_work.replace(false) {
                    GTK_AT_WORK
                } else if self.page_focused.get() || self.focused {
                    GTK_WITH_KEYBOARD
                } else {
                    GTK_AT_REST
                };
                ctx.request_repaint_after(wait);
            }
        }
    }

    fn keep_place(&mut self, solve: &mut SolveApp, ctx: &egui::Context, frame: &eframe::Frame) {
        #[cfg(target_os = "linux")]
        if !available() {
            web::set_host(solve, false);
            web::set_problem(solve, web::Problem::NativeWayland);
            self.show(false);
            return;
        }
        let Some(rect) = web::placement(solve, ctx) else {
            self.show(false);
            return;
        };
        let at = Placed { rect, zoom: ctx.zoom_factor(), pixels_per_point: ctx.pixels_per_point(), reveal: web::reveal(solve) };
        #[cfg(target_os = "linux")]
        self.latest.set(Some(at));
        let backing = web::backing(solve, ctx);
        if self.view.is_none() && !self.create(ctx, frame, at, backing) {
            // No web view on this machine: the panel says why and offers Retry; it does not draw
            // a second conversation.
            web::set_host(solve, false);
            web::set_problem(solve, web::Problem::Failed);
            ctx.request_repaint();
            return;
        }
        #[cfg(target_os = "macos")]
        web::set_clips(solve, self.clip.is_some());
        #[cfg(target_os = "linux")]
        let scale_changed = {
            use gtk::prelude::WidgetExt;
            let scale = self.gtk_window.as_ref().map(WidgetExt::scale_factor);
            let changed = scale != self.gtk_scale;
            self.gtk_scale = scale;
            changed && web::centred_view(solve)
        };
        #[cfg(not(target_os = "linux"))]
        let scale_changed = false;
        if (self.placed != Some(at) || scale_changed)
            && let Some(view) = &self.view
        {
            #[cfg(target_os = "linux")]
            if let Some(window) = self.gtk_window.as_ref() {
                place_gtk(window, at);
            }
            #[cfg(target_os = "macos")]
            if let Some(clip) = &self.clip {
                use wry::WebViewExtMacOS;
                let zoom = f64::from(at.zoom);
                let (rect, shown) = (at.rect, at.reveal.unwrap_or(at.rect.height()));
                let points = |value: f32| f64::from(value) * zoom;
                clip.place(&view.webview(), points(rect.left()), points(rect.top()), points(rect.width()), points(rect.height()), points(shown));
            } else if let Err(error) = view.set_bounds(place(at)) {
                note(&format!("could not be moved: {error}"));
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            if let Err(error) = view.set_bounds(place(at)) {
                note(&format!("could not be moved: {error}"));
            }
            // The page is drawn at the app's own size: a point of the page is a point of the app.
            if (scale_changed || self.placed.is_none_or(|before| (before.zoom, before.pixels_per_point) != (at.zoom, at.pixels_per_point)))
                && let Err(error) = view.zoom(page_zoom(view, at))
            {
                note(&format!("could not be zoomed: {error}"));
            }
            self.placed = Some(at);
        }
        // The page's pointer travel (`move`) comes in screen units, which WebKit does not scale with
        // the page's zoom.
        if let Some(view) = &self.view {
            web::set_zoom(solve, page_zoom(view, at) as f32);
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
        self.shape_gtk(web::centred_view(solve), at);
        self.tell(solve);
    }

    fn receive(&mut self, solve: &mut SolveApp, ctx: &egui::Context, message: &str) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(message) else {
            #[cfg(target_os = "linux")]
            self.set_gtk_shape(None, ctx);
            return;
        };
        match value.get("type").and_then(serde_json::Value::as_str) {
            // The page (re)started: it knows nothing yet.
            Some("ready") => {
                self.told.clear();
                #[cfg(target_os = "linux")]
                {
                    self.gtk_shape = None;
                    self.applied_shape = None;
                }
            }
            #[cfg(target_os = "linux")]
            Some("painted-footprint") => {
                self.set_gtk_shape(PageShape::decode(value, message.len()), ctx);
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

    #[cfg(target_os = "linux")]
    fn set_gtk_shape(&mut self, report: Option<PageShape>, ctx: &egui::Context) {
        use gtk::prelude::WidgetExt;
        if report.as_ref().is_none_or(|p| p.popup.is_some() || p.surfaces.is_empty())
            && let Some(native) = self.gtk_window.as_ref().and_then(WidgetExt::window)
        {
            // Remove the old shape even when placement is about to be deferred. Repeated
            // popup requests are intentional; a previous script evaluation might not have run.
            native.shape_combine_region(None, 0, 0);
            native.display().flush();
        }
        if self.gtk_shape != report || report.as_ref().is_some_and(|p| p.popup.is_some()) {
            self.gtk_shape = report;
            self.applied_shape = None;
            ctx.request_repaint();
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
        // The windows inside the app's before the view's is made among them.
        #[cfg(target_os = "linux")]
        let before = Keyboard::before(frame);
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
        #[cfg(target_os = "linux")]
        let builder = builder.with_initialization_script("window.__assistantX11Shape = true;");
        // A browser's own keys are not the Assistant's (Windows): F5 would load the page again
        // and Ctrl+P print it. The keys of text editing stay (WebView2's
        // `AreBrowserAcceleratorKeysEnabled`, as Microsoft describes it).
        #[cfg(target_os = "windows")]
        let builder = {
            use wry::WebViewBuilderExtWindows;
            builder.with_browser_accelerator_keys(false)
        };
        // On Windows the view is made hidden and shown once it is whole. Where it cannot be made
        // (no WebView2 runtime), wry leaves behind the window it began with, and a shown one lay
        // over the built-in conversation as a blank box. Nor does a view just made take the
        // keyboard there: a click on it does.
        let builder = if cfg!(target_os = "windows") { builder.with_visible(false).with_focused(false) } else { builder };
        // The builder panics on a window it cannot take; the app goes on without the web view.
        #[cfg(target_os = "linux")]
        let mut gtk_window = None;
        #[cfg(target_os = "linux")]
        let built: Result<wry::WebView, String> = {
            let latest = self.latest.clone();
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_embedded_gtk(builder, frame, at, latest, &mut gtk_window)));
            match built {
                Ok(result) => result,
                Err(_) => Err("could not be created in this window".into()),
            }
        };
        #[cfg(not(target_os = "linux"))]
        let built: Result<wry::WebView, String> = {
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build_as_child(frame)));
            match built {
                Ok(Ok(view)) => Ok(view),
                Ok(Err(error)) => Err(format!("could not be created: {error}")),
                Err(_) => Err("could not be created in this window".into()),
            }
        };
        match built {
            Ok(view) => {
                #[cfg(target_os = "linux")]
                {
                    use gtk::prelude::{WidgetExt, WidgetExtManual};
                    if let Some(window) = gtk_window.as_ref() {
                        let dirty = self.gtk_shape_dirty.clone();
                        let repaint = ctx.clone();
                        window.connect_map_event(move |_, _| {
                            dirty.set(true);
                            repaint.request_repaint();
                            gtk::glib::Propagation::Proceed
                        });
                        let dirty = self.gtk_shape_dirty.clone();
                        let repaint = ctx.clone();
                        window.connect_configure_event(move |_, _| {
                            dirty.set(true);
                            repaint.request_repaint();
                            false
                        });
                    }
                    self.gtk_window = gtk_window;
                }
                #[cfg(target_os = "macos")]
                {
                    use wry::WebViewExtMacOS;
                    self.clip = crate::assistant_clip::Clip::new(&view.webview());
                    if self.clip.is_none() {
                        note("could not be put in a frame of its own: it changes size as the page grows");
                    }
                }
                if let Err(error) = view.zoom(page_zoom(&view, at)) {
                    note(&format!("could not be zoomed: {error}"));
                }
                if cfg!(target_os = "windows")
                    && let Err(error) = view.set_visible(true)
                {
                    note(&format!("could not be shown: {error}"));
                }
                #[cfg(target_os = "linux")]
                {
                    use gtk::glib::Cast as _;
                    use gtk::prelude::{GtkWindowExt, WidgetExt};
                    use wry::WebViewExtUnix;
                    let found = Keyboard::of(before);
                    if found.is_none() {
                        note("its window was not found among the app's: typed keys reach the page only while the pointer is over it");
                    }
                    *self.keyboard.borrow_mut() = found;
                    let (keyboard, page_focused, repaint) = (self.keyboard.clone(), self.page_focused.clone(), ctx.clone());
                    let press_work = self.at_work.clone();
                    let page_widget = view.webview().clone();
                    {
                        let (drew, at_work) = (self.drew.clone(), self.at_work.clone());
                        page_widget.connect_draw(move |_, _| {
                            drew.set(true);
                            at_work.set(true);
                            gtk::glib::Propagation::Proceed
                        });
                    }
                    if let Some(gtk_window) = page_widget.toplevel().and_then(|top| top.downcast::<gtk::Window>().ok()) {
                        let keyboard = self.keyboard.clone();
                        let page_focused = self.page_focused.clone();
                        let press_work = self.at_work.clone();
                        let repaint = ctx.clone();
                        gtk_window.connect_button_press_event(move |_, _| {
                            press_work.set(true);
                            if let Some(keyboard) = keyboard.borrow().as_ref() {
                                match keyboard.give(keyboard.view) {
                                    Ok(()) => page_focused.set(true),
                                    Err(error) => note(&format!("could not be given the keyboard from the window: {error}")),
                                }
                            }
                            repaint.request_repaint();
                            gtk::glib::Propagation::Proceed
                        });
                    }
                    view.webview().connect_button_press_event(move |_, _| {
                        press_work.set(true);
                        // GTK runs this in the X event that pressed the page, before the first key
                        // that follows it: hand the keyboard to the view's own GTK window here.
                        // That window is GDK's (reparented into the app's), so GTK marks it active
                        // and the page's input method context gets its focus-in.
                        if let Some(keyboard) = keyboard.borrow().as_ref() {
                            match keyboard.give(keyboard.view) {
                                Ok(()) => page_focused.set(true),
                                Err(error) => note(&format!("could not be given the keyboard: {error}")),
                            }
                        }
                        page_widget.grab_focus();
                        if let Some(window) = page_widget.toplevel().and_then(|top| top.downcast::<gtk::Window>().ok()) {
                            window.set_focus(Some(&page_widget));
                        }
                        repaint.request_repaint();
                        gtk::glib::Propagation::Proceed
                    });
                }
                self.view = Some(view);
                self.placed = Some(at);
                self.backed = Some(backing);
                self.shown = true;
                true
            }
            Err(error) => {
                note(&format!("could not be created: {error}"));
                false
            }
        }
    }

    fn show(&mut self, shown: bool) {
        if self.shown == shown {
            return;
        }
        #[cfg(target_os = "linux")]
        if let Some(window) = &self.gtk_window {
            use gtk::prelude::WidgetExt;
            self.applied_shape = None;
            if shown { window.show_all() } else { window.hide() }
        }
        // A hidden frame takes no press either.
        #[cfg(target_os = "macos")]
        if let Some(clip) = &self.clip {
            clip.show(shown);
        }
        if let Some(view) = &self.view {
            if let Err(error) = view.set_visible(shown) {
                note(&format!("could not be {}: {error}", if shown { "shown" } else { "hidden" }));
            }
            // A hidden page cannot keep the keyboard: it goes back to the window. (The X server
            // gives it back by itself as the view's window is hidden. WebView2 is not asked
            // whether it had it: the window having it twice costs nothing.)
            if !shown && (self.focused || cfg!(target_os = "windows")) {
                #[cfg(not(target_os = "linux"))]
                let _ = view.focus_parent();
                self.focused = false;
            }
            #[cfg(target_os = "linux")]
            if !shown {
                self.page_focused.set(false);
            }
        }
        self.shown = shown;
    }

    #[cfg(target_os = "linux")]
    fn shape_gtk(&mut self, centred: bool, at: Placed) {
        use gtk::prelude::WidgetExt;
        let (Some(view), Some(window)) = (&self.view, &self.gtk_window) else { return };
        let Some(native) = window.window() else { return };
        // X11 placement and GTK widget allocation are asynchronous. The region belongs to
        // this GDK window: use its configured bounds, not a potentially older widget allocation.
        let (width, height, scale) = (native.width(), native.height(), window.scale_factor());
        let zoom = page_zoom(view, at);
        let key = (width, height, scale, if centred { zoom.to_bits() } else { 0 });
        if !self.gtk_shape_dirty.replace(false) && self.applied_shape == Some(key) {
            return;
        }
        let report = self.gtk_shape.as_ref().filter(|_| centred);
        let popup = report.and_then(|report| report.popup);
        let fits = report.is_some_and(|p| shape::matches_allocation(p.width, p.height, zoom, width, height));
        let region = report.filter(|_| popup.is_none() && fits).and_then(|p| shape::mask(&p.surfaces, zoom, width, height)).map(|spans| {
            let rectangles: Vec<_> = spans.into_iter().map(|[x, y, w, h]| gtk::cairo::RectangleInt::new(x, y, w, h)).collect();
            gtk::cairo::Region::create_rectangles(&rectangles)
        });
        native.shape_combine_region(region.as_ref(), 0, 0);
        if centred && std::env::var_os("SOLIDCRAFT_ASSISTANT_SHAPE_TRACE").is_some() {
            let allocation = window.allocation();
            let page = report.map(|p| (p.width, p.height, p.surfaces.len()));
            let rectangles = region.as_ref().map(gtk::cairo::Region::num_rectangles);
            note(&format!(
                "shape: native={width}x{height} allocation={}x{} scale={scale} zoom={zoom} page={page:?} fits={fits} popup={popup:?} \
                 rectangles={rectangles:?} shaped={}",
                allocation.width(),
                allocation.height(),
                native.is_shaped(),
            ));
        }
        // Flush before making a portal visible. No new window, focus request or input grab.
        if let Some(token) = popup {
            native.display().flush();
            if let Err(error) = view.evaluate_script(&format!("window.__assistantX11Unmasked?.({token})")) {
                note(&format!("could not acknowledge the popup's native bounds: {error}"));
            }
        }
        self.applied_shape = Some(key);
    }
}

/// The view's place in the window. On macOS it is given in the window's own points. Elsewhere it
/// is given in pixels: WebKitGTK counts points by GTK's scale, which need not be the window's,
/// and a child window on Windows is placed in pixels anyway.
fn place(at: Placed) -> wry::Rect {
    if cfg!(target_os = "macos") { bounds(at.rect, at.zoom) } else { bounds_in_pixels(at.rect, at.pixels_per_point) }
}

/// The page in a GTK window of its own, reparented into the app's X11 window (Linux). wry's
/// `build_as_child` wraps a foreign GdkWindow that GDK selects no focus events on, so GTK never
/// marks the window active and the page's input method never starts. A GtkWindow owns its
/// GdkWindow: reparented before it is mapped it is a child of the app's window with working
/// focus and input, and the window manager never sees it.
#[cfg(target_os = "linux")]
fn build_embedded_gtk(
    builder: wry::WebViewBuilder<'_>,
    frame: &eframe::Frame,
    at: Placed,
    latest: Rc<Cell<Option<Placed>>>,
    slot: &mut Option<gtk::Window>,
) -> Result<wry::WebView, String> {
    use gtk::glib::Cast as _;
    use gtk::prelude::{ContainerExt, GtkWindowExt, WidgetExt, WidgetExtManual};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use wry::WebViewBuilderExtUnix;
    let parent = match frame.window_handle().map_err(|error| error.to_string())?.as_raw() {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).map_err(|_| "the app's window id is too large".to_string())?,
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        _ => return Err("the app's window is not an X11 one".into()),
    };
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_decorated(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
    let wanted = bounds_in_pixels(at.rect, at.pixels_per_point).size.to_logical::<i32>(f64::from(window.scale_factor().max(1)));
    window.set_default_size(wanted.width.max(1), wanted.height.max(1));
    let bx = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&bx);
    let view = builder.build_gtk(&bx).map_err(|error| format!("could not be created: {error}"))?;
    window.realize();
    let child = window
        .window()
        .and_then(|window| window.downcast::<gdkx11::X11Window>().ok())
        .map(|window| window.xid() as u32)
        .ok_or_else(|| "the GTK window has no X11 window".to_string())?;
    {
        // A window that is mapped again (the Assistant hidden and shown, or the window system
        // replaying it) is put back on the panel after GTK has finished mapping it. The signal
        // keeps this window alive, so it must not capture a clone of the window: the one-shot
        // timer takes the only owned clone and drops it when the correction is done.
        let map_latest = latest.clone();
        window.connect_map_event(move |window, _| {
            let mapped = window.clone();
            let map_latest = map_latest.clone();
            gtk::glib::timeout_add_local(std::time::Duration::ZERO, move || {
                place_gtk(&mapped, delayed_place(map_latest.get(), at));
                gtk::glib::ControlFlow::Break
            });
            gtk::glib::Propagation::Proceed
        });
    }
    if let Ok((connection, _)) = x11rb::connect(None) {
        use x11rb::protocol::xproto::ConnectionExt;
        connection.reparent_window(child, parent, 0, 0).map_err(|error| error.to_string())?;
        // A round-trip here: GDK's map (another connection) must not reach the server before
        // this reparent, or the window is mapped as a root child and GTK keeps it at 0,0.
        if let Ok(request) = connection.get_input_focus() {
            let _ = request.reply();
        }
    }
    place_gtk(&window, at);
    window.show_all();
    window.resize(wanted.width.max(1), wanted.height.max(1));
    place_gtk(&window, at);
    // GTK may set a toplevel's position as it maps it; put the child back on its panel once
    // more after the map has gone through. The panel's place may have changed since the view
    // was made (the compositor tiles the new window), so read the latest place here.
    let placed = window.clone();
    let captured = at;
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
        place_gtk(&placed, delayed_place(latest.get(), captured));
        gtk::glib::ControlFlow::Break
    });
    *slot = Some(window);
    Ok(view)
}

/// The place a delayed correction puts the view on: where the panel is now, not where it was
/// when the view was made. A window the compositor has tiled since then has a different panel,
/// and the old place can lie outside it or over another part of the app.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn delayed_place(latest: Option<Placed>, captured: Placed) -> Placed {
    latest.unwrap_or(captured)
}

/// The embedded page's place in the app's window, in the window's pixels.
#[cfg(target_os = "linux")]
fn place_gtk(window: &gtk::Window, at: Placed) {
    use gtk::glib::Cast as _;
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt};
    let Some(xid) = window.window().and_then(|window| window.downcast::<gdkx11::X11Window>().ok()).map(|window| window.xid() as u32) else {
        return;
    };
    let rect = bounds_in_pixels(at.rect, at.pixels_per_point);
    let position = rect.position.to_logical::<i32>(1.0);
    let size = rect.size.to_logical::<u32>(1.0);
    // GTK keeps its own idea of where its toplevel is and how big it is, and puts it there when
    // it maps the window. Keep that idea on the panel too, in GTK's points, so a later map or
    // resize does not put the page back on the size and place it was made with.
    let scale = f64::from(window.scale_factor().max(1));
    let logical = rect.position.to_logical::<f64>(scale);
    let logical_size = rect.size.to_logical::<f64>(scale);
    window.move_(logical.x.round() as i32, logical.y.round() as i32);
    window.resize(logical_size.width.round().max(1.0) as i32, logical_size.height.round().max(1.0) as i32);
    if let Ok((connection, _)) = x11rb::connect(None) {
        let _ = connection
            .configure_window(xid, &ConfigureWindowAux::new().x(position.x).y(position.y).width(size.width.max(1)).height(size.height.max(1)));
        let _ = connection.flush();
    }
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

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_keeps_complete_surfaces_without_their_empty_corners_or_gap() {
        // Two independently rounded surfaces at 125% in a larger allocation. The small
        // header's bottom-right corner is square; neither it nor the composer touches y=12.
        let surfaces = [[0., 0., 16., 8., 4., 4., 4., 4., 0., 0., 4., 4.], [0., 12., 16., 20., 4., 4., 4., 4., 4., 4., 4., 4.]];
        let spans = shape::spans(&surfaces, 1.25, 20, 25);
        let contains = |x, y| spans.iter().any(|[left, top, width, height]| x >= *left && x < left + width && y >= *top && y < top + height);
        assert!(!contains(0, 0));
        assert!(!contains(19, 0));
        assert!(contains(10, 0));
        assert!(contains(19, 9));
        assert!(!contains(10, 12));
        assert!(contains(5, 15));
        assert!(contains(10, 24));
        assert!(!contains(0, 24));
        assert!(!contains(19, 24));
        assert!(!contains(20, 20));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_clips_at_allocation_and_rejects_invalid_surfaces() {
        let surfaces = [
            [-2., -2., 4., 4., 0., 0., 0., 0., 0., 0., 0., 0.],
            [0., 0., 100., 100., f64::INFINITY, 0., 0., 0., 0., 0., 0., 0.],
            [20., 20., 25., 25., 0., 0., 0., 0., 0., 0., 0., 0.],
        ];
        assert_eq!(shape::spans(&surfaces, 2.0, 6, 6), [[0, 0, 6, 6]]);
        assert!(shape::spans(&surfaces, f64::NAN, 6, 6).is_empty());
        assert!(shape::spans(&surfaces[1..], 1.0, 6, 6).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_excludes_the_backing_row_above_a_fractional_top() {
        // At 125% the header starts at y=0.75; the opaque native row starting at y=0
        // must not be included. The second surface must not fill it back in.
        let surfaces = [[0., 0.6, 16., 8., 4., 4., 4., 4., 0., 0., 0., 0.], [0., 8., 16., 20., 4., 4., 4., 4., 4., 4., 4., 4.]];
        let spans = shape::spans(&surfaces, 1.25, 20, 25);
        assert!(spans.iter().all(|&[_, y, _, _]| y >= 1), "{spans:?}");
        assert!(spans.iter().any(|&[x, y, width, _]| y == 1 && x <= 10 && x + width > 10), "{spans:?}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_keeps_only_full_pixels_on_rounded_sides() {
        let surface = [[0.2, 0.6, 16., 20., 0., 0., 4., 4., 0., 0., 4., 4.]];
        let spans = shape::spans(&surface, 1.25, 20, 25);
        let covers = |x, y| spans.iter().any(|&[l, t, w, h]| l <= x && x < l + w && t <= y && y < t + h);
        // The top-right arc crosses y=1 at x≈16.56: column 15 fits the whole row,
        // while 16 is partially covered. The square left side keeps its old rounding.
        assert!(!covers(15, 0));
        assert!(covers(15, 1));
        assert!(!covers(16, 1));
        assert!(covers(0, 1));
        // The straight right edge keeps its last pixel once the arc ends.
        assert!(covers(19, 6));
        // The bottom-left arc is narrowest at the bottom of the last row (x=5.25).
        assert!(!covers(5, 24));
        assert!(covers(6, 24));
        assert!(covers(19, 24));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_starting_fixture_covers_full_width_at_125_percent() {
        // The bundled page's collapsed status/header and ring at 520×137 CSS pixels.
        let surfaces = [[0., 0., 520., 73., 23., 23., 23., 23., 0., 0., 0., 0.], [0., 49., 520., 137., 23., 23., 23., 23., 23., 23., 23., 23.]];
        assert!(shape::matches_allocation(520., 137., 1.25, 650, 171));
        let spans = shape::mask(&surfaces, 1.25, 650, 171).expect("the starting fixture has a native region, not a rectangular fallback");
        let covers = |x, y| spans.iter().any(|&[l, t, w, h]| l <= x && x < l + w && t <= y && y < t + h);
        for y in [30, 72, 120] {
            assert!((0..650).all(|x| covers(x, y)), "the straight row {y} keeps its full width");
        }
        assert!((0..171).all(|y| covers(325, y)), "overlapping surfaces keep the centre column");
        assert!(!covers(647, 168) && !covers(645, 2), "the named CAD corner probes stay outside the native region");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_allocation_allows_one_css_pixel_of_rounding_not_two() {
        assert!(shape::matches_allocation(520., 137., 1.25, 650, 171));
        assert!(shape::matches_allocation(519., 137., 1.25, 650, 171));
        assert!(!shape::matches_allocation(520., 135., 1.25, 650, 171));
        assert!(!shape::matches_allocation(518.4, 137., 1.25, 650, 171));
        assert!(!shape::matches_allocation(520., 137., f64::NAN, 650, 171));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_empty_report_keeps_the_glow_and_clipped_edge_covers_its_corners() {
        assert!(shape::mask(&[], 1., 100, 100).is_none(), "the glow has the whole allocation");
        let header = [[0., 20., 100., 40., 16., 16., 16., 16., 0., 0., 0., 0.]];
        let spans = shape::mask(&header, 1., 100, 100).unwrap();
        assert!(
            spans.iter().any(|&[x, y, width, height]| x == 0 && width == 100 && y + height == 40),
            "a square clipped edge keeps both bottom corners"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn centre_shape_rejects_oversized_or_malformed_reports() {
        let surface = serde_json::json!([0, 0, 10, 10, 2, 2, 2, 2, 0, 0, 0, 0]);
        let mut valid = serde_json::json!({"width":10,"height":10,"popup":null,"surfaces":[surface.clone()]});
        assert!(PageShape::decode(valid.clone(), 512).is_some());
        assert!(PageShape::decode(valid.clone(), 4097).is_none());
        valid["surfaces"] = serde_json::json!(vec![surface; 9]);
        assert!(PageShape::decode(valid.clone(), 1024).is_none());
        valid["surfaces"] = serde_json::json!([[0., 0., 10., 10., f64::NAN, 2., 2., 2., 0., 0., 0., 0.]]);
        assert!(PageShape::decode(valid, 512).is_none());
    }

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

    /// The correction after the view is mapped follows the panel's latest place. A cold start
    /// can tile the window after the view is made, so reusing the place captured at creation
    /// leaves the page outside the panel (INT-1472).
    #[test]
    fn the_delayed_placement_uses_the_panels_latest_place() {
        let made = Placed {
            rect: egui::Rect::from_min_max(egui::pos2(1221.0, 165.8), egui::pos2(1591.0, 944.8)),
            zoom: 1.0,
            pixels_per_point: 1.25,
            reveal: None,
        };
        let tiled = Placed {
            rect: egui::Rect::from_min_max(egui::pos2(638.6, 165.8), egui::pos2(1008.6, 1062.4)),
            zoom: 1.0,
            pixels_per_point: 2.0,
            reveal: None,
        };
        assert_eq!(delayed_place(Some(tiled), made), tiled, "the window was tiled after the view was made");
        assert_eq!(delayed_place(None, made), made, "before the host recorded a place, the made one stands");
    }

    /// The view's X11 window is the one that appeared in the app's window as the view was made.
    /// With none new, or more than one, it is not known, and the keyboard is not handed over.
    #[test]
    fn the_views_window_is_the_one_new_window_in_the_apps() {
        assert_eq!(new_window(&[], &[7]), Some(7));
        assert_eq!(new_window(&[3, 4], &[3, 9, 4]), Some(9));
        assert_eq!(new_window(&[3, 4], &[3, 4]), None, "the view's window is not there");
        assert_eq!(new_window(&[3], &[3, 8, 9]), None, "which of two is the view's is not known");
        assert_eq!(new_window(&[3, 4], &[]), None);
    }
}
