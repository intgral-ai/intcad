//! The Assistant's view at the bottom centre of the window, shown and pressed only at its foot
//! (macOS). The view stays as tall as the page may grow, inside a container view as tall as what
//! the page shows: AppKit hands a press to a view only inside its superview's frame, so above the
//! container a press reaches the window's own view, and the page is see-through there. The view
//! never changes size as the page opens the conversation or a menu. A WebKit view grown taller
//! draws its old picture at its new top until the page has drawn again, which showed the closed
//! message box at the top of the opened conversation for a frame.
//!
//! The one `unsafe` call is AppKit's `superview`, which objc2 has not marked safe; there is no other
//! way to reach the view wry put in the window.
#![allow(unsafe_code)]

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::NSView;
use objc2_foundation::{NSPoint, NSRect, NSSize};

pub struct Clip {
    container: Retained<NSView>,
}

impl Clip {
    /// Puts `view` in a container of its own, where its superview had it. `None` off the main
    /// thread or for a view in no window.
    pub fn new(view: &NSView) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        // SAFETY: a property read on the main thread (the marker proves it) of a view we hold.
        let parent = unsafe { view.superview() }?;
        let container = NSView::initWithFrame(mtm.alloc(), view.frame());
        parent.addSubview(&container);
        // Moves it out of `parent`.
        container.addSubview(view);
        Some(Self { container })
    }

    /// The whole view at (`x`, `y`) from the window's top left, `width` by `height` in the window's
    /// points; `shown` of it, from its foot, is shown and takes presses.
    pub fn place(&self, view: &NSView, x: f64, y: f64, width: f64, height: f64, shown: f64) {
        // SAFETY: as in `new`; `place` runs on the main thread, where the app draws.
        let Some(parent) = (unsafe { self.container.superview() }) else { return };
        let shown = shown.clamp(0.0, height.max(0.0));
        let top = y + height - shown;
        let from_foot = if parent.isFlipped() { top } else { parent.frame().size.height - top - shown };
        self.container.setFrame(NSRect::new(NSPoint::new(x, from_foot), NSSize::new(width, shown)));
        // The container is not flipped: its foot is at 0, and the view rises above its top.
        view.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)));
    }

    pub fn show(&self, shown: bool) {
        self.container.setHidden(!shown);
    }
}

impl Drop for Clip {
    /// The view went with wry's; its frame goes too.
    fn drop(&mut self) {
        self.container.removeFromSuperview();
    }
}
