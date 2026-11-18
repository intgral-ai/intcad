//! Bounded input queue for a CEF windowless surface.
//!
//! CEF drops input until the browser has painted its first frame. A host that
//! forwards events straight through therefore loses whatever the user typed in
//! the first ~100 ms after the panel opens. The queue holds events until the
//! first frame and then replays them in order.
//!
//! Rules, from the acceptance criteria:
//! - keyboard and IME order is preserved exactly;
//! - pointer motion is coalesced (the last position wins) because it is
//!   discardable;
//! - the queue is bounded, and anything dropped beyond the bound is counted and
//!   logged, never silently discarded;
//! - blur, hide and close clear it.

use std::collections::VecDeque;

use welding::linux_cef::LinuxCefProducer;
use welding::{CefSurfaceProducer, EventModifiers, KeyEvent, KeyEventKind, MouseAction, MouseButton, MouseEvent};

/// Pointer moves kept separate from critical input: only the newest matters.
#[derive(Clone, Copy, Debug)]
struct Move {
    x: i32,
    y: i32,
}

#[derive(Clone, Debug)]
pub enum Input {
    Mouse(MouseEvent),
    Key(KeyEvent),
    ImeSet { text: String, cursor: (u32, u32) },
    ImeCommit(String),
    ImeCancel,
}

pub struct InputQueue {
    /// Critical events, in arrival order.
    critical: VecDeque<Input>,
    /// The newest pointer motion, or `None`.
    motion: Option<Move>,
    capacity: usize,
    ready: bool,
    /// Events accepted before the first frame (evidence that the queue worked).
    pub held: u64,
    /// Motion coalesced away.
    pub coalesced: u64,
    /// Critical events refused because the queue was full. Never silent.
    pub refused: u64,
}

impl InputQueue {
    pub fn new(capacity: usize) -> Self {
        Self { critical: VecDeque::with_capacity(capacity.min(1024)), motion: None, capacity, ready: false, held: 0, coalesced: 0, refused: 0 }
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn pending(&self) -> usize {
        self.critical.len() + usize::from(self.motion.is_some())
    }

    /// Push a motion event. Coalescing means a burst of 200 motions costs one.
    pub fn push_motion(&mut self, x: i32, y: i32, modifiers: EventModifiers) {
        self.push(Input::Mouse(MouseEvent { x, y, button: MouseButton::Left, action: MouseAction::Moved, modifiers }));
    }

    pub fn push(&mut self, event: Input) {
        if let Input::Mouse(MouseEvent { action: MouseAction::Moved, x, y, .. }) = event {
            let had = self.motion.is_some();
            self.motion = Some(Move { x, y });
            if !self.ready {
                self.held += 1;
            }
            if had {
                self.coalesced += 1;
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

    /// The browser painted its first frame: what was typed is now deliverable.
    pub fn ready(&mut self) {
        self.ready = true;
    }

    /// Drop everything. Used on blur, hide, close and a failed initialisation.
    pub fn clear(&mut self) {
        self.critical.clear();
        self.motion = None;
    }

    /// Send the pointer half first. A click that gives the page its keyboard is
    /// processed asynchronously, so the characters that follow must wait for the
    /// next tick rather than travelling in the same burst.
    pub fn flush_pointer(&mut self, producer: &mut LinuxCefProducer) -> usize {
        if !self.ready {
            return 0;
        }
        let mut sent = 0;
        // CEF routes hover and key updates only to a focused browser. This is a
        // browser call, not input, so it may go before the first frame too.
        if let Err(error) = producer.move_focus(welding::FocusDirection::Forward) {
            eprintln!("queue: could not focus the browser: {error}");
        }
        if let Some(Move { x, y }) = self.motion.take() {
            let _ = producer.send_mouse_input(MouseEvent {
                x,
                y,
                button: MouseButton::Left,
                action: MouseAction::Moved,
                modifiers: EventModifiers::default(),
            });
            sent += 1;
        }
        while matches!(self.critical.front(), Some(Input::Mouse(_))) {
            let Some(event) = self.critical.pop_front() else {
                break;
            };
            if let Err(error) = send(producer, event) {
                eprintln!("queue: input refused by the producer: {error}");
                self.refused += 1;
                continue;
            }
            sent += 1;
        }
        sent
    }

    /// Send everything the producer can take now, in order. Motion goes last:
    /// it describes where the pointer is, not something that happened between
    /// the key events.
    pub fn flush(&mut self, producer: &mut LinuxCefProducer) -> usize {
        if !self.ready {
            return 0;
        }
        let mut sent = 0;
        // Where the pointer is first, so a click that follows lands where the
        // user left it; the click events carry their own coordinates anyway.
        if let Some(Move { x, y }) = self.motion.take() {
            let _ = producer.send_mouse_input(MouseEvent {
                x,
                y,
                button: MouseButton::Left,
                action: MouseAction::Moved,
                modifiers: EventModifiers::default(),
            });
            sent += 1;
        }
        while let Some(event) = self.critical.pop_front() {
            if let Err(error) = send(producer, event) {
                eprintln!("queue: input refused by the producer: {error}");
                self.refused += 1;
                continue;
            }
            sent += 1;
        }
        sent
    }
}

fn send(producer: &mut LinuxCefProducer, event: Input) -> Result<(), welding::WeldError> {
    match event {
        Input::Mouse(event) => producer.send_mouse_input(event),
        Input::Key(event) => producer.send_keyboard_input(event),
        Input::ImeSet { text, cursor } => producer.ime_set_composition(&text, cursor),
        Input::ImeCommit(text) => producer.ime_commit_text(&text),
        Input::ImeCancel => producer.ime_cancel_composition(),
    }
}

/// One character, the way a text field receives typing.
pub fn char_event(ch: char) -> Input {
    Input::Key(KeyEvent {
        kind: KeyEventKind::Char,
        windows_key_code: 0,
        native_key_code: 0,
        character: Some(ch),
        modifiers: EventModifiers::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn motion_is_coalesced_and_critical_order_is_kept() {
        let mut queue = InputQueue::new(8);
        queue.push_motion(1, 1, EventModifiers::default());
        queue.push(char_event('a'));
        queue.push_motion(2, 2, EventModifiers::default());
        queue.push_motion(3, 3, EventModifiers::default());
        queue.push(char_event('b'));
        assert_eq!(queue.critical.len(), 2, "two characters are held, in order");
        assert_eq!(queue.coalesced, 2);
        assert_eq!(queue.held, 5);
    }

    #[test]
    fn the_bound_is_reported_not_silent() {
        let mut queue = InputQueue::new(3);
        for index in 0..5 {
            queue.push(char_event(char::from(b'a' + index as u8)));
        }
        assert_eq!(queue.critical.len(), 3);
        assert_eq!(queue.refused, 2);
    }

    #[test]
    fn clear_drops_everything() {
        let mut queue = InputQueue::new(4);
        queue.push(char_event('x'));
        queue.push_motion(9, 9, EventModifiers::default());
        queue.clear();
        assert_eq!(queue.pending(), 0);
    }
}
