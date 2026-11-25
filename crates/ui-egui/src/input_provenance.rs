//! Which input this frame came from the platform, and which from app-injected automation.
//!
//! Native-only actions need the gesture that actually activated the control, not merely a
//! frame-wide marker: automation can inject a press in one frame and let a platform event
//! complete it, or restore held typing after the marker was computed. eframe can also keep a
//! logic-only frame's post-hook [`RawInput`](egui::RawInput) and pass it through this hook again,
//! so an unconsumed batch has to stay untrusted/fail closed until a real UI pass consumes it.

use egui::{Event, Key, PointerButton, Pos2, Rect};

/// The pointer buttons this tracker follows. Other buttons are ignored, never aliased to one of
/// these.
const BUTTONS: [PointerButton; 3] = [PointerButton::Primary, PointerButton::Secondary, PointerButton::Middle];

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

/// Provenance for the current frame plus the pointer gesture still in progress across frames.
///
/// eframe appends new input to its pending post-hook batch and calls the hook again before any
/// pass consumes it. Host pre-hooks may transform that batch first, so the retained fact is only
/// whether the unconsumed batch contained synthetic input, not its exact event list. A real UI
/// pass clears it through [`Tracker::input_consumed_by_ui`]; a logic-only pass keeps it.
#[derive(Debug, Default)]
pub(crate) struct Tracker {
    gesture: [Gesture; BUTTONS.len()],
    /// Platform Space/Enter presses this frame; more than one is ambiguous.
    platform_keys: u8,
    synthetic_keys: bool,
    platform_accesskit: u8,
    synthetic_accesskit: bool,
    /// `None` nothing is pending; `Some(false)` an unconsumed all-platform batch; `Some(true)` an
    /// unconsumed batch that contained synthetic input and cannot be mapped through pre-hooks.
    retained: Option<bool>,
}

impl Tracker {
    /// Start a hook batch using the explicit consumption lifecycle.
    ///
    /// No pending batch means this is fresh platform input. An unconsumed all-platform batch stays
    /// trusted even if host pre-hooks transformed it. An unconsumed batch that contained synthetic
    /// input cannot be mapped through those transformations, so the whole batch fails closed for
    /// native-only actions while ordinary input is left intact. Either way, a retained batch is
    /// replayed, so pointer state is rebuilt.
    pub(crate) fn begin_batch(&mut self, events: &[Event]) -> (Vec<bool>, bool) {
        match self.retained.take() {
            None => (vec![true; events.len()], false),
            Some(false) => (vec![true; events.len()], true),
            Some(true) => {
                self.gesture = Default::default();
                (vec![false; events.len()], true)
            }
        }
    }

    /// Record that the current post-hook batch was not consumed by a real UI pass yet.
    ///
    /// Only whether synthetic input was present is kept, so this stays constant-size and cannot
    /// re-label transformed events. A missing origin taints the batch, never drops an event.
    pub(crate) fn retain(&mut self, events: &[Event], from_platform: &[bool]) {
        self.retained =
            if events.is_empty() { None } else { Some(events.len() != from_platform.len() || from_platform.iter().any(|platform| !platform)) };
    }

    /// A real UI pass consumed the input, so no unconsumed batch remains to replay.
    pub(crate) fn input_consumed_by_ui(&mut self) {
        self.retained = None;
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
    fn retained_native_input_stays_trusted_when_pre_hooks_transform_it() {
        let mut tracker = Tracker::default();
        let press = vec![pointer(pos2(120.0, 110.0), true)];
        let (origins, replayed) = tracker.begin_batch(&press);
        assert_eq!(origins, plain(&press));
        assert!(!replayed);
        tracker.update(&press, &origins, replayed);
        tracker.retain(&press, &origins);

        // The desktop pre-hooks may insert/remove events before the app hook sees the batch.
        let transformed = vec![Event::ModifiersChanged(Modifiers::NONE), pointer(pos2(120.0, 110.0), true), pointer(pos2(120.0, 110.0), false)];
        let (origins, replayed) = tracker.begin_batch(&transformed);
        assert_eq!(origins, vec![true, true, true], "an all-platform retained batch stays platform");
        assert!(replayed, "the retained batch is replayed, not applied twice");
        tracker.update(&transformed, &origins, replayed);
        assert!(tracker.pointer_trusted(PointerButton::Primary, control()), "the press was retained and the transformed batch completed it");
    }

    #[test]
    fn retained_synthetic_input_fails_closed_when_pre_hooks_transform_it() {
        let mut tracker = Tracker::default();
        let press = vec![pointer(pos2(120.0, 110.0), true)];
        let (mut origins, replayed) = tracker.begin_batch(&press);
        assert!(!replayed);
        origins[0] = false; // what synthetic injection records for its press
        tracker.update(&press, &origins, false);
        tracker.retain(&press, &origins);

        // We cannot tell which transformed event came from the retained synthetic press.
        let transformed = vec![Event::ModifiersChanged(Modifiers::NONE), pointer(pos2(120.0, 110.0), true), pointer(pos2(120.0, 110.0), false)];
        let (origins, replayed) = tracker.begin_batch(&transformed);
        assert_eq!(origins, vec![false, false, false], "an unconsumed synthetic batch fails closed");
        assert!(replayed);
        tracker.update(&transformed, &origins, replayed);
        assert!(
            !tracker.pointer_trusted(PointerButton::Primary, control()),
            "a transformed retained synthetic batch cannot be completed by a platform release"
        );
    }

    #[test]
    fn visible_ui_consumption_clears_the_retained_batch() {
        let mut tracker = Tracker::default();
        let press = vec![pointer(pos2(120.0, 110.0), true)];
        let (mut origins, replayed) = tracker.begin_batch(&press);
        assert!(!replayed);
        origins[0] = false;
        tracker.update(&press, &origins, false);
        tracker.retain(&press, &origins);

        tracker.input_consumed_by_ui();
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[true], false);
        assert!(!tracker.pointer_trusted(PointerButton::Primary, control()), "the old synthetic press ends untrusted");

        let fresh = vec![pointer(pos2(120.0, 110.0), true)];
        let (origins, replayed) = tracker.begin_batch(&fresh);
        assert_eq!(origins, vec![true], "after a real UI pass the next batch is fresh platform input");
        assert!(!replayed);
        tracker.update(&fresh, &origins, replayed);
        tracker.update(&[pointer(pos2(120.0, 110.0), false)], &[true], false);
        assert!(tracker.pointer_trusted(PointerButton::Primary, control()));
    }

    #[test]
    fn a_missing_origin_taints_the_retained_batch() {
        let mut tracker = Tracker::default();
        let events = vec![key(Key::Enter)];
        tracker.update(&events, &[true], false);
        tracker.retain(&events, &[]);
        let (origins, replayed) = tracker.begin_batch(&[key(Key::Space)]);
        assert_eq!(origins, vec![false], "a retained length mismatch fails closed on the next hook");
        assert!(replayed);
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
