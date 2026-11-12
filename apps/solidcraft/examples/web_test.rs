//! Minimal WebKitGTK page probe for the GTK host prototype (no GL area).
#![allow(unsafe_code)]
use gtk::prelude::*;
use webkit2gtk::WebViewExt;

fn main() {
    gtk::init().unwrap();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_default_size(800, 700);
    let web = webkit2gtk::WebView::new();
    web.connect_load_failed(|_, _, uri, error| {
        eprintln!("WEB load failed {uri}: {error}");
        false
    });
    web.connect_load_changed(|_, event| println!("WEB load {event:?}"));
    let page = std::env::args().nth(1).unwrap_or_else(|| "/tmp/hello.html".into());
    web.load_uri(&format!("file://{page}"));
    window.add(&web);
    window.connect_destroy(|_| gtk::main_quit());
    window.show_all();
    gtk::main();
}
