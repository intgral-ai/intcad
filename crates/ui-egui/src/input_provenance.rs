//! Which input this frame came from the platform, and which from app-injected automation.
//!
//! Native-only actions need the gesture that actually activated the control, not merely a
//! frame-wide marker: automation can inject a press in one frame and let a platform event
//! complete it, or restore held typing after the marker was computed. The tracker keeps pointer
//! presses across frames and binds the result to the control's rectangle.

use egui::{Event, Key, PointerButton, Pos2, Rect};

/// The pointer buttons this tracker follows.
const BUTTONS: [PointerButton; 3] = [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle];

/// One button's gesture, kept from its platform or injected press until the frame that observes
/// its release.
#[derive(Clone, Copy, Debug)]
struct Gesture {
    down: bool,
    press_from_platform: bool,
    press: Pos2,
    /// A synthetic event touched this gesture between press and release.
    mixed: bool,
    /// This frame's release origin; `None` until a release arrives.
    release_from_platform: Option<bool>,
}

impl Default for Gesture {
    fn default() -> Self {
        Self { down: false, press_from_platform: false, press: Pos2::ZERO, mixed: false, release_from_platform: None }
    }
}

/// Provenance for the current frame plus the pointer gesture still in progress across frames.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Tracker {
    gesture: [Gesture; BUTTONS.len()],
    /// Platform Space/Enter presses this frame; more than one is ambiguous.
    platform_keys: u8,
    synthetic_keys: bool,
    platform_accesskit: u8,
    synthetic_accesskit: bool,
}

impl Tracker {
    /// Replace this frame's summary and consume its ordered events with their origins.
    ///
    /// `from_platform` has one entry per event. The caller restores held typing before calling
    /// this, so a replayed synthetic key stays synthetic.
    pub(crate) fn update(&mut self, events: &[Event], from_platform: &[bool]) {
        debug_assert_eq!(events.len(), from_platform.len());
        for gesture in &mut self.gesture {
            gesture.release_from_platform = None;
        }
        self.platform_keys = 0;
        self.synthetic_keys = false;
        self.platform_accesskit = 0;
        self.synthetic_accesskit = false;

        for (event, &from_platform) in events.iter().zip(from_platform.iter()) {
            match event {
                Event::PointerButton { pos, button, pressed, .. } => {
                    let index = button_index(*button);
                    if *pressed {
                        self.gesture[index] =
                            Gesture { down: true, press_from_platform: from_platform, press: *pos, mixed: false, release_from_platform: None };
                    } else {
                        let gesture = &mut self.gesture[index];
                        if gesture.down {
                            gesture.down = false;
                            gesture.release_from_platform = Some(from_platform);
                        } else {
                            // A release with no press we observed cannot prove a gesture.
                            gesture.release_from_platform = Some(false);
                        }
                    }
                }
                Event::Key { key: Key::Space | Key::Enter, pressed: true, .. } => {
                    if from_platform {
                        self.platform_keys = self.platform_keys.saturating_add(1);
                    } else {
                        self.synthetic_keys = true;
                    }
                }
                Event::AccessKitActionRequest(_) => {
                    if from_platform {
                        self.platform_accesskit = self.platform_accesskit.saturating_add(1);
                    } else {
                        self.synthetic_accesskit = true;
                    }
                }
                _ => {
                    if !from_platform {
                        for gesture in &mut self.gesture {
                            if gesture.down {
                                gesture.mixed = true;
                            }
                        }
                    }
                }
            }
        }
    }

    /// This frame's primary pointer click is a trusted activation on `rect`.
    ///
    /// Both ends of the gesture must be platform input, no synthetic event may have touched it,
    /// and the press must have begun on the control itself.
    pub(crate) fn pointer_trusted(&self, button: PointerButton, rect: Rect) -> bool {
        let gesture = &self.gesture[button_index(button)];
        gesture.release_from_platform == Some(true) && gesture.press_from_platform && !gesture.mixed && rect.contains(gesture.press)
    }

    /// The frame's keyboard or accessibility activation is one unambiguous platform gesture.
    pub(crate) fn keyboard_trusted(&self) -> bool {
        let platform = u16::from(self.platform_keys) + u16::from(self.platform_accesskit);
        platform == 1 && !self.synthetic_keys && !self.synthetic_accesskit
    }

    /// A keyboard or accessibility activation happened this frame, so a simultaneous pointer
    /// click would have two possible sources and is ambiguous.
    pub(crate) fn any_activation_without_pointer(&self) -> bool {
        self.platform_keys > 0 || self.platform_accesskit > 0 || self.synthetic_keys || self.synthetic_accesskit
    }
}

fn button_index(button: PointerButton) -> usize {
    BUTTONS.iter().position(|known| *known == button).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, Pos2, Rect, pos2, vec2};

    use super::*;

    fn control() -> Rect {
        Rect::from_min_size(pos2(100.0, 100.0), vec2(80.0, 30.0))
    }

    fn pointer(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
    }

    fn key(key: Key) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn a_platform_pointer_press_and_release_across_frames_is_trusted_on_the_control() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true]);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "a press is not a click");
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[true]);
        assert!(tracker.pointer_trusted(PointerButton::Primary, control()));
    }

    #[test]
    fn a_synthetic_press_completed_by_a_platform_release_is_not_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[false]);
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[true]);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "a synthetic press cannot be completed by a platform release");
    }

    #[test]
    fn a_platform_press_completed_by_a_synthetic_release_is_not_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true]);
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[false]);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "a platform press cannot be completed by synthetic input");
    }

    #[test]
    fn a_synthetic_move_during_a_platform_press_makes_the_gesture_ambiguous() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true]);
        tracker.update(&[Event::PointerMoved(pos2(125.0, 112.0))], &[false]);
        tracker.update(&[pointer(pos2(125.0, 112.0), false)], &[true]);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "mixed input is not one trusted gesture");
    }

    #[test]
    fn a_platform_press_outside_the_control_is_not_trusted_for_it() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(20.0, 20.0), true)], &[true]);
        tracker.update(&[pointer(pos2(20.0, 20.0), false)], &[true]);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "the gesture must begin on this control");
    }

    #[test]
    fn one_platform_keyboard_activation_is_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Space)], &[true]);
        assert!(tracker.keyboard_trusted());
        assert!(tracker.any_activation_without_pointer());
    }

    #[test]
    fn a_synthetic_key_is_not_trusted_even_beside_a_platform_key() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Enter), key(Key::Space)], &[true, false]);
        assert!(!tracker.keyboard_trusted(), "one platform and one synthetic activation are ambiguous");
    }

    #[test]
    fn a_held_synthetic_key_restored_after_a_platform_event_is_not_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Enter), key(Key::Tab)], &[false, true]);
        assert!(!tracker.keyboard_trusted(), "restored synthetic typing cannot ride a platform frame");
    }

    #[test]
    fn two_platform_activations_are_ambiguous() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Enter), key(Key::Space)], &[true, true]);
        assert!(!tracker.keyboard_trusted(), "two possible activations are not one explicit gesture");
    }
}
