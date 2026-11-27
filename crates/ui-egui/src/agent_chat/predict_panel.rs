//! The right rail's Autocomplete panel (INT-1458): the switch of next-step suggestions, and the
//! latest ones with what became of each. Kept in the window only; nothing here leaves the computer.

use egui::{Rect, RichText, UiBuilder, vec2};

use super::{Logged, unix_ms};
use crate::SolveApp;
use crate::i18n::{format as trf, tr};
use crate::theme::Tokens;

/// Suggestions listed, newest first.
const MAX_ROWS: usize = 50;

/// What became of a suggestion, in words: taken, turned down, taken back by the app, or neither.
fn outcome(l: &Logged) -> &'static str {
    match l.outcome.as_str() {
        "accepted" => tr("ui.next_step_out_accepted"),
        "dismissed" => tr("ui.next_step_out_dismissed"),
        "asking" | "shown" | "ghost" => tr("ui.next_step_out_shown"),
        _ => tr("ui.next_step_out_withdrawn"),
    }
}

/// The suggestions the panel lists: those with a step, newest first.
pub(crate) fn rows(app: &SolveApp) -> Vec<(String, String)> {
    let now = unix_ms();
    let listed = app.agent_chat.predict.log().iter().rev().filter(|l| !l.step.is_empty());
    listed
        .take(MAX_ROWS)
        .map(|l| {
            let minutes = now.saturating_sub(l.at_unix_ms) / 60_000;
            let when =
                if minutes == 0 { tr("ui.next_step_just_now").to_string() } else { trf("ui.next_step_minutes_ago", &[("n", &minutes.to_string())]) };
            (l.step.clone(), format!("{} · {when}", outcome(l)))
        })
        .collect()
}

/// Draw the panel in `area`: the switch, then the list. With nothing to list, the room left for the
/// rail's own empty block.
pub(crate) fn show(app: &mut SolveApp, ui: &mut egui::Ui, area: Rect) -> Option<Rect> {
    let t = Tokens::get();
    let inner = area.shrink2(vec2(t.space_12, t.space_8));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(egui::Layout::top_down(egui::Align::Min)));
    let response = crate::ui_kit::checkbox(&mut child, tr("ui.next_step_setting"), &mut app.preferences.agent_next_step);
    crate::scenario::publish_handle("rail:autocomplete:switch", response.rect.center());
    crate::ui_kit::status_line(&mut child, tr("ui.next_step_cost"), crate::ui_kit::StatusTone::Dim);
    child.add_space(t.space_8);
    let rows = rows(app);
    if rows.is_empty() {
        return Some(Rect::from_min_max(egui::pos2(area.left(), child.cursor().top()), area.max));
    }
    let height = (inner.bottom() - child.cursor().top()).max(0.0);
    egui::ScrollArea::vertical().id_salt("autocomplete_rows").max_height(height).auto_shrink([false, true]).show(&mut child, |ui| {
        for (step, what) in rows {
            ui.add(egui::Label::new(RichText::new(step).size(t.font_body).color(t.text)).wrap());
            ui.add(egui::Label::new(RichText::new(what).size(t.font_caption).color(t.text_dim)).wrap());
            ui.add_space(t.space_8);
        }
    });
    None
}
