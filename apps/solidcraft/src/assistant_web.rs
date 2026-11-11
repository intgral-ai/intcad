//! The Assistant's web view (macOS): the conversation and the composer drawn by a page in a
//! native web view, laid over the place the Assistant panel gives up for it in the app's own
//! window. The page is built by `packages/assistant-web` and embedded here; it loads nothing from
//! outside the app. What it is told and what it may ask for is
//! `solidcraft_ui_egui::agent_chat::web`.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use eframe::egui;
use solidcraft_ui_egui::SolveApp;
use solidcraft_ui_egui::agent_chat::web;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::http::{Request, Response, header};

/// The page's own address: a scheme only this web view serves, from the files below.
const SCHEME: &str = "openskyfall-assistant";
const PAGE: &str = "openskyfall-assistant://app/index.html";

/// Agent output is untrusted text: the page may load only its own files and send nothing out.
/// The page's `<meta>` says the same; this header is the copy the page cannot change.
const POLICY: &str = "default-src 'none'; script-src 'self' openskyfall-assistant:; style-src 'self' openskyfall-assistant: 'unsafe-inline'; \
                      font-src 'self' openskyfall-assistant:; img-src 'self' openskyfall-assistant: data: blob:; connect-src 'none'; \
                      base-uri 'none'; form-action 'none'";

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

/// The page stays the page: nothing else loads in the view, not even another of its own files.
fn own_address(url: &str) -> bool {
    url == PAGE || url == "about:blank"
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
    placed: Option<(egui::Rect, f32)>,
    shown: bool,
    /// The state the page was last told, to tell it only changes.
    told: String,
    /// The page has the keyboard.
    focused: bool,
}

impl AssistantWeb {
    /// The host has a web view to offer: the panel may give its conversation up to it.
    pub fn new(solve: &mut SolveApp) -> Self {
        web::set_host(solve, true);
        Self::default()
    }

    /// The page has the keyboard: text editing keys and menu items belong to it.
    pub fn focused(&self) -> bool {
        self.focused && self.shown
    }

    /// Undo (or redo) the text being typed in the page: the Edit menu's Undo while the page has
    /// the keyboard.
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
        let Some(rect) = web::placement(solve, ctx) else {
            self.show(false);
            return;
        };
        if self.view.is_none() && !self.create(ctx, frame, rect) {
            // No web view on this machine: the panel draws its own conversation again.
            web::set_host(solve, false);
            ctx.request_repaint();
            return;
        }
        let zoom = ctx.zoom_factor();
        if self.placed != Some((rect, zoom))
            && let Some(view) = &self.view
        {
            if let Err(error) = view.set_bounds(bounds(rect, zoom)) {
                note(&format!("could not be moved: {error}"));
            }
            // The page is drawn at the app's own zoom: a point of the page is a point of the app.
            if self.placed.is_none_or(|(_, before)| before != zoom)
                && let Err(error) = view.zoom(f64::from(zoom))
            {
                note(&format!("could not be zoomed: {error}"));
            }
            self.placed = Some((rect, zoom));
        }
        self.show(true);
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

    fn create(&mut self, ctx: &egui::Context, frame: &eframe::Frame, rect: egui::Rect) -> bool {
        let inbox = self.inbox.clone();
        let repaint = ctx.clone();
        let (drags, dragged) = (self.drags.clone(), ctx.clone());
        let pictures = self.pictures.clone();
        // Only files are the app's to take; text dragged within the page stays the page's.
        let files = Cell::new(false);
        let zoom = ctx.zoom_factor();
        let builder = wry::WebViewBuilder::new()
            .with_custom_protocol(SCHEME.to_string(), move |_view, request| serve(&request, &pictures))
            .with_url(PAGE)
            .with_bounds(bounds(rect, zoom))
            // See-through outside what the page draws: the page rounds its own corners, and the
            // panel's frame (its border and shadow) shows around them.
            .with_transparent(true)
            .with_accept_first_mouse(true)
            .with_navigation_handler(|url| own_address(&url))
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
        // The builder panics on a window it cannot take; the app goes on without the web view.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| builder.build_as_child(frame)));
        match built {
            Ok(Ok(view)) => {
                if let Err(error) = view.zoom(f64::from(zoom)) {
                    note(&format!("could not be zoomed: {error}"));
                }
                self.view = Some(view);
                self.placed = Some((rect, zoom));
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
            // A hidden page cannot keep the keyboard: it goes back to the window.
            if !shown && self.focused {
                let _ = view.focus_parent();
                self.focused = false;
            }
        }
        self.shown = shown;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn get_with(path: &str, pictures: &Pictures) -> Response<Cow<'static, [u8]>> {
        serve(&Request::builder().uri(format!("openskyfall-assistant://app{path}")).body(Vec::new()).unwrap(), pictures)
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

    /// Nothing but the page's own address loads in the view.
    #[test]
    fn only_the_pages_own_address_loads() {
        assert!(own_address(PAGE));
        assert!(own_address("about:blank"));
        for url in [
            "https://example.com/",
            "http://127.0.0.1:8080/",
            "file:///etc/passwd",
            "openskyfall-assistant://elsewhere/index.html",
            "openskyfall-assistant://app/assistant.js",
            "openskyfall-assistant://app/picture/a-1",
            "openskyfall-assistant://app/index.html/../picture/a-1",
            "javascript:alert(1)",
        ] {
            assert!(!own_address(url), "{url}");
        }
    }
}
