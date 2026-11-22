//! Which input this frame came from the platform, and which from app-injected automation.
//!
//! Native-only actions need the gesture that actually activated the control, not merely a
//! frame-wide marker: automation can inject a press in one frame and let a platform event
//! complete it, or restore held typing after the marker was computed. eframe can also keep a
//! logic-only frame's post-hook [`RawInput`](egui::RawInput) and pass it through this hook again,
//! so each event's origin has to survive that replay until the batch is consumed.

use egui::{Event, Key, PointerButton, Pos2, Rect};

/// The pointer buttons this tracker follows. Other buttons are ignored, never aliased to one of
/// these.
const BUTTONS: [PointerButton; 3] = [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle];

/// A backlog larger than this is not copied into the retained batch. The next hook then fails
/// closed for native-only actions instead of letting unlabelled replay through.
const MAX_RETAINED_EVENTS: usize = 16_384;

/// One button's gesture, kept from its platform or injected press until the frame that observes
/// its release.
#[derive(Clone, Copy, Debug, Default)]
struct Gesture {
    down: bool,
    press_from_platform: bool,
    press: Pos2,
    /// A synthetic event touched this gesture between press and release.
    mixed: bool,
    /// This frame's release origin; `None` until a release arrives.
    release_from_platform: Option<bool>,
}

/// The previous post-hook event batch, so a rehook can reuse the origins it already computed.
///
/// eframe appends new input to its pending batch and calls the hook again before any pass
/// consumes it. Events are therefore always a prefix match plus new platform input. A stale
/// batch (already consumed by a pass) does not match and the new input is trusted as platform
/// input. `Unverifiable` is the bounded fallback: one batch fails closed rather than reusing
/// origins we could not preserve.
#[derive(Clone, Debug, Default)]
enum Retained {
    #[default]
    None,
    Events {
        events: Vec<Event>,
        from_platform: Vec<bool>,
    },
    Unverifiable,
}

/// Provenance for the current frame plus the pointer gesture still in progress across frames.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    gesture: [Gesture; BUTTONS.len()],
    /// Platform Space/Enter presses this frame; more than one is ambiguous.
    platform_keys: u8,
    synthetic_keys: bool,
    platform_accesskit: u8,
    synthetic_accesskit: bool,
    retained: Retained,
}

impl Tracker {
    /// Start a hook batch: label events from the retained post-hook prefix, or treat all events
    /// as new platform input when that prefix is stale. Returns the origins and whether the
    /// pointer state must be rebuilt from the entire replayed batch.
    pub(crate) fn begin_batch(&mut self, events: &[Event]) -> (Vec<bool>, bool) {
        match std::mem::take(&mut self.retained) {
            Retained::None => (vec![true; events.len()], false),
            Retained::Unverifiable => {
                // We cannot prove which events were already labelled. Rebuild from this batch
                // with every event untrusted for native-only actions; ordinary input still works.
                self.gesture = Default::default();
                (vec![false; events.len()], true)
            }
            Retained::Events { events: retained, mut from_platform } => {
                if !retained.is_empty() && events.len() >= retained.len() && events[..retained.len()] == retained[..] {
                    from_platform.resize(events.len(), true);
                    (from_platform, true)
                } else if !events.is_empty() && events.len() < retained.len() && retained[..events.len()] == events[..] {
                    // A shorter prefix cannot prove whether these are retained or new events.
                    self.gesture = Default::default();
                    (vec![false; events.len()], true)
                } else {
                    // The previous batch was consumed by a pass: this is a fresh platform batch.
                    (vec![true; events.len()], false)
                }
            }
        }
    }

    /// Keep this post-hook batch for the next hook call, bounded by [`MAX_RETAINED_EVENTS`].
    ///
    /// A length mismatch cannot be labelled safely, so the next batch fails closed for
    /// native-only actions; the events themselves are already preserved by the caller.
    pub(crate) fn retain(&mut self, events: &[Event], from_platform: &[bool]) {
        self.retained = if events.is_empty() {
            Retained::None
        } else if events.len() != from_platform.len() || events.len() > MAX_RETAINED_EVENTS {
            Retained::Unverifiable
        } else {
            Retained::Events { events: events.to_vec(), from_platform: from_platform.to_vec() }
        };
    }

    /// Replace this frame's summary and consume its ordered events with their origins.
    ///
    /// `from_platform` should have one entry per event. A missing entry is untrusted and never
    /// drops the event. When `replayed` is true the batch contains the retained history again,
    /// so the pointer state is rebuilt from the start instead of applying a press twice.
    pub(crate) fn update(&mut self, events: &[Event], from_platform: &[bool], replayed: bool) {
        if replayed {
            self.gesture = Default::default();
        }
        for gesture in &mut self.gesture {
            gesture.release_from_platform = None;
        }
        self.platform_keys = 0;
        self.synthetic_keys = false;
        self.platform_accesskit = 0;
        self.synthetic_accesskit = false;

        for (index, event) in events.iter().enumerate() {
            let from_platform = from_platform.get(index).copied().unwrap_or(false);
            match event {
                Event::PointerButton { pos, button, pressed, .. } => {
                    let Some(index) = button_index(*button) else {
                        // Other buttons must not alias Primary; a synthetic one still taints a
                        // gesture already in progress.
                        if !from_platform {
                            self.mark_down_mixed();
                        }
                        continue;
                    };
                    let gesture = &mut self.gesture[index];
                    if *pressed {
                        if gesture.down {
                            // A second press cannot rewrite the in-progress gesture's origin or
                            // target; it only makes the gesture ambiguous.
                            gesture.mixed = true;
                        } else {
                            *gesture = Gesture {
                                down: true,
                                press_from_platform: from_platform,
                                press: *pos,
                                mixed: !from_platform,
                                release_from_platform: None,
                            };
                        }
                    } else if gesture.down {
                        gesture.down = false;
                        gesture.release_from_platform = Some(from_platform);
                    } else {
                        // A release with no press we observed cannot prove a gesture.
                        gesture.release_from_platform = Some(false);
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
                        self.mark_down_mixed();
                    }
                }
            }
        }
    }

    /// This frame's pointer click is a trusted activation on `rect`.
    ///
    /// Both ends of the gesture must be platform input, no synthetic event may have touched it,
    /// and the press must have begun on the control itself.
    pub(crate) fn pointer_trusted(&self, button: PointerButton, rect: Rect) -> bool {
        let Some(index) = button_index(button) else { return false };
        let gesture = &self.gesture[index];
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

    fn mark_down_mixed(&mut self) {
        for gesture in &mut self.gesture {
            if gesture.down {
                gesture.mixed = true;
            }
        }
    }
}

fn button_index(button: PointerButton) -> Option<usize> {
    BUTTONS.iter().position(|known| *known == button)
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

    fn plain(events: &[Event]) -> Vec<bool> {
        vec![true; events.len()]
    }

    #[test]
    fn a_platform_pointer_press_and_release_across_frames_is_trusted_on_the_control() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true], false);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "a press is not a click");
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[true], false);
        assert!(tracker.pointer_trusted(PointerButton::Primary, control()));
    }

    #[test]
    fn a_synthetic_press_completed_by_a_platform_release_is_not_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[false], false);
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[true], false);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "a synthetic press cannot be completed by a platform release");
    }

    #[test]
    fn a_platform_press_completed_by_a_synthetic_release_is_not_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true], false);
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[false], false);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "a platform press cannot be completed by synthetic input");
    }

    #[test]
    fn a_synthetic_move_during_a_platform_press_makes_the_gesture_ambiguous() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true], false);
        tracker.update(&[Event::PointerMoved(pos2(125.0, 112.0))], &[false], false);
        tracker.update(&[pointer(pos2(125.0, 112.0), false)], &[true], false);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "mixed input is not one trusted gesture");
    }

    #[test]
    fn a_platform_press_outside_the_control_is_not_trusted_for_it() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(20.0, 20.0), true)], &[true], false);
        tracker.update(&[pointer(pos2(20.0, 20.0), false)], &[true], false);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "the gesture must begin on this control");
    }

    #[test]
    fn one_platform_keyboard_activation_is_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Space)], &[true], false);
        assert!(tracker.keyboard_trusted());
        assert!(tracker.any_activation_without_pointer());
    }

    #[test]
    fn a_synthetic_key_is_not_trusted_even_beside_a_platform_key() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Enter), key(Key::Space)], &[true, false], false);
        assert!(!tracker.keyboard_trusted(), "one platform and one synthetic activation are ambiguous");
    }

    #[test]
    fn a_held_synthetic_key_restored_after_a_platform_event_is_not_trusted() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Enter), key(Key::Tab)], &[false, true], false);
        assert!(!tracker.keyboard_trusted(), "restored synthetic typing cannot ride a platform frame");
    }

    #[test]
    fn two_platform_activations_are_ambiguous() {
        let mut tracker = Tracker::default();
        tracker.update(&[key(Key::Enter), key(Key::Space)], &[true, true], false);
        assert!(!tracker.keyboard_trusted(), "two possible activations are not one explicit gesture");
    }

    #[test]
    fn retained_native_press_and_new_release_stay_trusted() {
        let mut tracker = Tracker::default();
        let press = vec![pointer(pos2(120.0, 110.0), true)];
        let (origins, replayed) = tracker.begin_batch(&press);
        assert_eq!(origins, plain(&press));
        assert!(!replayed);
        tracker.update(&press, &origins, replayed);
        tracker.retain(&press, &origins);

        let mut next = press.clone();
        next.push(pointer(pos2(120.0, 110.0), false));
        let (origins, replayed) = tracker.begin_batch(&next);
        assert_eq!(origins, vec![true, true]);
        assert!(replayed, "the retained press is replayed, not applied twice");
        tracker.update(&next, &origins, replayed);
        assert!(tracker.pointer_trusted(PointerButton::Primary, control()));
    }

    #[test]
    fn retained_synthetic_press_stays_tainted_when_a_platform_release_arrives() {
        let mut tracker = Tracker::default();
        let press = vec![pointer(pos2(120.0, 110.0), true)];
        let (mut origins, replayed) = tracker.begin_batch(&press);
        assert!(!replayed);
        origins[0] = false; // what synthetic injection records for its press
        tracker.update(&press, &origins, false);
        tracker.retain(&press, &origins);

        let mut next = press.clone();
        next.push(pointer(pos2(120.0, 110.0), false));
        let (origins, replayed) = tracker.begin_batch(&next);
        assert_eq!(origins, vec![false, true], "the retained synthetic press keeps its origin");
        assert!(replayed);
        tracker.update(&next, &origins, replayed);
        assert!(
            !tracker.pointer_trusted(PointerButton::Primary, control()),
            "a retained synthetic press cannot be completed by a new platform release"
        );
    }

    #[test]
    fn a_stale_retained_batch_does_not_taint_new_native_input() {
        let mut tracker = Tracker::default();
        let old = vec![key(Key::Enter)];
        let origins = plain(&old);
        tracker.update(&old, &origins, false);
        tracker.retain(&old, &origins);

        let fresh = vec![key(Key::Space)];
        let (origins, replayed) = tracker.begin_batch(&fresh);
        assert_eq!(origins, vec![true], "a consumed batch is not a prefix of the next platform batch");
        assert!(!replayed);
    }

    #[test]
    fn a_shorter_retained_prefix_fails_closed() {
        let mut tracker = Tracker::default();
        let retained = vec![key(Key::Enter), key(Key::Space)];
        let origins = plain(&retained);
        tracker.update(&retained, &origins, false);
        tracker.retain(&retained, &origins);

        let shorter = vec![key(Key::Enter)];
        let (origins, replayed) = tracker.begin_batch(&shorter);
        assert_eq!(origins, vec![false], "a truncated prefix cannot prove its origin");
        assert!(replayed);
        tracker.update(&shorter, &origins, replayed);
        assert!(!tracker.keyboard_trusted());
    }

    #[test]
    fn extra_buttons_do_not_alias_primary() {
        let mut tracker = Tracker::default();
        let extra = [
            Event::PointerButton { pos: pos2(120.0, 110.0), button: PointerButton::Extra1, pressed: true, modifiers: Modifiers::NONE },
            Event::PointerButton { pos: pos2(120.0, 110.0), button: PointerButton::Extra1, pressed: false, modifiers: Modifiers::NONE },
        ];
        tracker.update(&extra, &plain(&extra), false);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "Extra1 must not complete a Primary gesture");

        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true], false);
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[true], false);
        assert!(tracker.pointer_trusted(PointerButton::Primary, control()));
    }

    #[test]
    fn a_synthetic_press_does_not_rebind_an_in_progress_platform_gesture() {
        let mut tracker = Tracker::default();
        tracker.update(&[pointer(pos2(120.0, 110.0), true)], &[true], false);
        tracker.update(
            &[Event::PointerButton { pos: pos2(300.0, 300.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
            &[false],
            false,
        );
        tracker.update(&[pointer(pos2(300.0, 300.0), false)], &[true], false);
        let other = Rect::from_min_size(pos2(280.0, 280.0), vec2(80.0, 30.0));
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "the gesture is mixed");
        assert!(!tracker.pointer_trusted(PointerButton::Primary, other), "a synthetic press cannot move trust to another control");
    }
}
