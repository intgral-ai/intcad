//! The chip of a next-step suggestion at the end of History, and what recorded demos draw (INT-1458;
//! docs/design/next-step-prediction.md): pressed keys and the person's pointer. The suggestion
//! itself is a line and a ghost in the view (`predict.rs`, `ghost.rs`).

use std::sync::OnceLock;

use egui::{Painter, Rect, pos2, vec2};

use crate::SolveApp;
use crate::theme::Tokens;

/// How long a pressed key stays in the demo key strip (seconds).
const KEY_S: f64 = 1.8;

/// The key strip shows pressed keys at the view's bottom left: for recorded demos only
/// (`SOLIDCRAFT_DEMO_KEYS=1`).
fn key_strip_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("SOLIDCRAFT_DEMO_KEYS").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Note this frame's key presses, before anything consumes them, for the demo key strip.
pub(crate) fn note_keys(app: &mut SolveApp, ctx: &egui::Context, now: f64) {
    if !key_strip_on() {
        return;
    }
    let pressed: Vec<String> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => key_name(*key, *modifiers),
                _ => None,
            })
            .collect()
    });
    let keys = &mut app.agent_chat.predict.keys;
    keys.retain(|(_, at)| now - at < KEY_S);
    keys.extend(pressed.into_iter().map(|k| (k, now)));
    if keys.len() > 5 {
        let extra = keys.len() - 5;
        keys.drain(..extra);
    }
    // Presses, for the demo pointer's click ripple.
    let press = ctx.input(|i| i.pointer.any_pressed().then(|| i.pointer.interact_pos()).flatten());
    let presses = &mut app.agent_chat.predict.presses;
    presses.retain(|(_, at)| now - at < RIPPLE_S);
    if let Some(p) = press {
        presses.push((p, now));
    }
}

/// How long a click's ripple lasts (seconds).
const RIPPLE_S: f64 = 0.6;

/// The name a key shows, for the keys worth showing (`None`: not one of them).
fn key_name(key: egui::Key, m: egui::Modifiers) -> Option<String> {
    let base = match key {
        egui::Key::Tab => "Tab",
        egui::Key::Escape => "Esc",
        egui::Key::Enter => "Enter",
        _ if m.ctrl || m.command => key.name(),
        _ => return None,
    };
    let mut s = String::new();
    if m.ctrl || m.command {
        s.push_str("Ctrl+");
    }
    if m.shift {
        s.push_str("Shift+");
    }
    s.push_str(base);
    Some(s)
}

/// The icon of the command a step names, from its first verb (English or Chinese); the agent's own
/// icon when none is known.
pub(crate) fn step_icon(step: &str) -> &'static str {
    const VERBS: &[(&str, &str)] = &[
        ("fillet", "fillet"),
        ("round", "fillet"),
        ("chamfer", "chamfer"),
        ("bevel", "chamfer"),
        ("shell", "shell"),
        ("hollow", "shell"),
        ("countersink", "hole"),
        ("counterbore", "hole"),
        ("drill", "hole"),
        ("hole", "hole"),
        ("pattern", "pattern_rect"),
        ("array", "pattern_rect"),
        ("mirror", "mirror"),
        ("combine", "combine"),
        ("join", "combine"),
        ("union", "combine"),
        ("merge", "combine"),
        ("subtract", "cut"),
        ("cut", "cut"),
        ("notch", "cut"),
        ("pocket", "cut"),
        ("extrude", "extrude"),
        ("revolve", "revolve"),
        ("draft", "draft"),
        ("rib", "rib"),
        ("standoff", "boss"),
        ("boss", "boss"),
        ("lip", "lip"),
        ("lid", "box"),
        ("thread", "thread"),
        ("engrave", "emboss"),
        ("emboss", "emboss"),
        ("text", "text"),
        ("sketch", "sketch"),
        ("split", "split_body"),
        ("offset", "offset_face"),
        ("thicken", "thicken"),
        ("sweep", "sweep"),
        ("loft", "loft"),
        ("move", "move"),
        ("cylinder", "cylinder"),
        ("box", "box"),
        ("倒圆", "fillet"),
        ("圆角", "fillet"),
        ("倒角", "chamfer"),
        ("抽壳", "shell"),
        ("沉头", "hole"),
        ("孔", "hole"),
        ("阵列", "pattern_rect"),
        ("镜像", "mirror"),
        ("合并", "combine"),
        ("拉伸", "extrude"),
        ("旋转", "revolve"),
        ("拔模", "draft"),
        ("加强筋", "rib"),
        ("凸台", "boss"),
        ("草图", "sketch"),
    ];
    let lower = step.to_lowercase();
    VERBS.iter().filter_map(|(verb, icon)| lower.find(verb).map(|at| (at, *icon))).min_by_key(|(at, _)| *at).map_or("agent_chat", |(_, icon)| icon)
}

/// The command of the step suggested, and whether its ghost is drawn over the design.
pub(crate) fn ghost(app: &SolveApp) -> Option<(&'static str, bool)> {
    let p = &app.agent_chat.predict;
    p.shown.as_ref().map(|g| (step_icon(&g.step), p.ghost.is_some()))
}

/// Paint the ghost of the next History item after its last one, from `x`: the History chip of the
/// step's command, faded as a rolled-back item is (it is not in the design yet), or selected once
/// its ghost is drawn. The x it ends at.
pub(crate) fn paint_ghost(painter: &Painter, ctx: &egui::Context, x: f32, row: Rect, (icon, ready): (&'static str, bool)) -> f32 {
    let t = Tokens::get();
    let rect = Rect::from_center_size(pos2(x + t.space_24 / 2.0, row.center().y), vec2(t.space_24, t.space_24));
    crate::ui_kit::HistoryChip {
        icon,
        name: "",
        kind: "",
        selected: ready,
        hovered: false,
        dim: !ready,
        suppressed: false,
        error: false,
        warning: false,
        reference: false,
        ink: None,
        accent: crate::theme::TabAccent::current(ctx),
    }
    .paint(painter, rect, crate::ui_kit::HistoryStyle::Bottom);
    rect.right()
}

#[cfg(not(target_arch = "wasm32"))]
pub use hud::show;

#[cfg(target_arch = "wasm32")]
pub fn show(_: &mut SolveApp, _: &egui::Context) {}

#[cfg(not(target_arch = "wasm32"))]
mod hud {
    use egui::{Align2, RichText, pos2};

    use super::{KEY_S, key_strip_on};
    use crate::SolveApp;
    use crate::theme::Tokens;
    use crate::ui_kit::Container;

    /// Draw the demo key strip and pointer, when a recording asks for them.
    pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
        if !key_strip_on() {
            return;
        }
        let Some(layout) = app.viewport.layout.clone() else { return };
        if layout.drawable.is_positive() {
            key_strip(app, ctx, layout.drawable, ctx.input(|i| i.time));
            demo_pointer(app, ctx);
        }
    }

    /// Pressed keys at the view's bottom left, fading out (demo recordings): the navigation
    /// tooltip's keycap outline on the shared anchored HUD host.
    fn key_strip(app: &SolveApp, ctx: &egui::Context, view: egui::Rect, now: f64) {
        let keys = &app.agent_chat.predict.keys;
        if keys.is_empty() {
            return;
        }
        let t = Tokens::get();
        let anchor = pos2(view.left() + t.space_16, view.bottom() - t.space_16);
        let latest = keys.iter().map(|(_, at)| *at).fold(f64::MIN, f64::max);
        let fade = (1.0 - ((now - latest) as f32 - (KEY_S as f32 - 0.4)) / 0.4).clamp(0.0, 1.0);
        Container::navigation_card(ctx, egui::Id::new("sc_demo_keys"), anchor, Align2::LEFT_BOTTOM, |ui| {
            ui.multiply_opacity(fade);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = t.space_6;
                for (name, _) in keys {
                    egui::Frame::NONE
                        .stroke(egui::Stroke::new(1.0, t.nav_tip_key_border))
                        .fill(t.control_fill)
                        .corner_radius(t.radius_sm)
                        .inner_margin(egui::Margin::symmetric(t.space_8 as i8, t.space_4 as i8))
                        .show(ui, |ui| {
                            // A key's name is never cut ("Enter" read "En…" when the card was narrow).
                            ui.add(egui::Label::new(RichText::new(name).monospace().size(t.font_subtitle).color(t.form_ink)).extend());
                        });
                }
            });
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }

    /// The person's pointer for recordings (`SOLIDCRAFT_DEMO_KEYS=1`): control-channel pointer
    /// input moves no screen cursor, so the app draws one where its own input says the pointer is,
    /// with a ripple where it pressed. Drawn like the agent cursor (top layer), in neutral tokens
    /// and without the agent's tag, so the person's hand reads apart from the agent's.
    pub(super) fn demo_pointer(app: &SolveApp, ctx: &egui::Context) {
        let Some(p) = ctx.input(|i| i.pointer.latest_pos()) else { return };
        let t = Tokens::get();
        let now = ctx.input(|i| i.time);
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Debug, egui::Id::new("sc_demo_pointer")));
        for (at, t0) in &app.agent_chat.predict.presses {
            let k = ((now - t0) / super::RIPPLE_S).clamp(0.0, 1.0) as f32;
            painter.circle_stroke(*at, t.space_6 + t.space_24 * k, egui::Stroke::new(t.space_3 * (1.0 - k) + 0.5, t.accent.gamma_multiply(1.0 - k)));
        }
        let arrow: Vec<egui::Pos2> = [(0.0, 0.0), (0.0, 17.0), (4.5, 13.0), (7.5, 20.0), (10.5, 18.8), (7.5, 12.0), (13.0, 12.0)]
            .iter()
            .map(|(x, y)| p + egui::vec2(*x, *y))
            .collect();
        painter.add(egui::Shape::convex_polygon(arrow.clone(), t.text, egui::Stroke::NONE));
        painter.add(egui::Shape::closed_line(arrow, egui::Stroke::new(1.5, t.panel)));
        if !app.agent_chat.predict.presses.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}
