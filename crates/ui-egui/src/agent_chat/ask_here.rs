//! Ask on the model (the "Cursor for CAD" spec, feature 3): select a face, an edge or a feature,
//! press the key of **Ask Here** (`ui.ask_here`, Ctrl+K by default) and type into the small box
//! that opens beside the pointer, where the pick was made. Enter sends it as a message of the
//! conversation with exactly what is selected attached, as **Attach selection** would attach it;
//! Escape closes the box. With **Preview the agent's changes** on, the turn runs on a copy and its
//! answer comes back as a preview card ([`super::review`]), as any message's does.

use egui::{Pos2, pos2};

use super::{context, tr};
use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit::{self, Container, Surface};

/// The most characters the box takes.
const MAX_TEXT: usize = 4000;

/// The box beside the pick.
#[derive(Default)]
pub struct AskBox {
    pub open: bool,
    /// What is typed.
    pub text: String,
    /// Where the box sits (the pointer when it opened); `None`: placed on the next frame.
    at: Option<Pos2>,
    /// Give the field focus on the next frame only (see `shortcut_box::ShortcutBox::focus`).
    focus: bool,
}

/// Open the box for the selection. `Err`: why it did not open (AI features off, nothing selected).
pub fn open(app: &mut SolveApp) -> Result<(), String> {
    if !super::shown(app) {
        return Err(tr("ui.agent_chat_ask_here_off").to_string());
    }
    if app.session.selection.is_empty() {
        let why = tr("ui.agent_chat_ask_here_empty").to_string();
        app.set_status(why.clone(), false);
        return Err(why);
    }
    let ask = &mut app.agent_chat.ask_box;
    ask.open = true;
    ask.at = None;
    ask.focus = true;
    Ok(())
}

/// Close the box, keeping nothing of what was typed.
pub fn close(app: &mut SolveApp) {
    app.agent_chat.ask_box = AskBox::default();
}

/// Send `text` with exactly the selection attached (no files, no picture of the view), as a
/// message of the conversation, and close the box. `Err`: why nothing was sent; the box stays.
pub fn submit(app: &mut SolveApp, ctx: &egui::Context, text: &str) -> Result<(), String> {
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return Err(tr("ui.agent_chat_ask_here_empty_text").to_string());
    }
    let selected = context::describe_selection(&mut app.session);
    if selected.items.is_empty() {
        return Err(tr("ui.agent_chat_ask_here_empty").to_string());
    }
    // Not now (a card waits for Accept or Reject, the queue is full): the box stays, with the words.
    super::send_with(app, ctx, text, Some(selected))?;
    close(app);
    // The answer comes in the conversation: show it.
    app.agent_chat.open = true;
    app.preferences.agent_chat_minimized = false;
    Ok(())
}

/// Draw the box while it is open: a one-line field beside where it opened, with how many things
/// go with it.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    if !app.agent_chat.ask_box.open {
        return;
    }
    // The selection went (another click, Escape in the view): nothing to ask about.
    if app.session.selection.is_empty() || !super::shown(app) {
        close(app);
        return;
    }
    let t = Tokens::get();
    let screen = ctx.content_rect();
    let width = (t.space_24 * 14.0).min((screen.width() - t.space_8 * 2.0).max(t.space_24));
    let at = *app.agent_chat.ask_box.at.get_or_insert_with(|| {
        let pointer = ctx.input(|i| i.pointer.latest_pos()).unwrap_or(screen.center());
        // Beside the pick, below and right of the pointer, kept on the screen.
        pos2(
            (pointer.x + t.space_12).clamp(screen.left() + t.space_8, (screen.right() - width - t.space_8).max(screen.left() + t.space_8)),
            (pointer.y + t.space_12).clamp(screen.top() + t.space_8, (screen.bottom() - t.control_height * 3.0).max(screen.top() + t.space_8)),
        )
    });
    let count = context::describe_selection(&mut app.session).items.len();
    let (enter, escape) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    let mut text = std::mem::take(&mut app.agent_chat.ask_box.text);
    let focus = std::mem::take(&mut app.agent_chat.ask_box.focus);
    let mut send = false;
    Container::new(ctx, Surface::Popup).area(egui::Id::new("sc_ask_here")).fixed_pos(at).order(egui::Order::Foreground).constrain(true).show(
        ctx,
        |ui| {
            Container::popup_frame(ui.style()).show(ui, |ui| {
                ui.set_width(width);
                ui_kit::scope(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = t.space_4;
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .id(egui::Id::new("sc_ask_here_text"))
                            .hint_text(tr("ui.agent_chat_ask_here_hint"))
                            .char_limit(MAX_TEXT)
                            .desired_width(width),
                    );
                    if focus {
                        field.request_focus();
                    }
                    crate::scenario::publish_handle("agent_chat:ask_here", field.rect.center());
                    send = field.lost_focus() && enter;
                    let note = super::trf("ui.agent_chat_ask_here_note", &[("n", &count.to_string())]);
                    ui.add(egui::Label::new(egui::RichText::new(note).color(t.text_dim).size(t.font_small)).wrap());
                });
            });
        },
    );
    if escape {
        close(app);
        return;
    }
    if send {
        if let Err(why) = submit(app, ctx, &text) {
            app.set_status(why, false);
            app.agent_chat.ask_box.text = text;
        }
        return;
    }
    app.agent_chat.ask_box.text = text;
}
