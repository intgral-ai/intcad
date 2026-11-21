//! The home-page account summary and the application bar's credit status chip.

use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Align2, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui};
use solidcraft_engine::entitlement::RefusalCode;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit::{BackingTooltipExt as _, Button, Container, FileView, Menu, Surface, Text, button, text};

use super::{AccountView, Action, DeviceCode, SignInStatus, Timestamp};

/// The product-account context an in-place prompt belongs to. Model authentication is a separate
/// provider concern and never uses this prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptContext {
    Chat,
    Review,
}

impl PromptContext {
    fn message(self) -> &'static str {
        match self {
            Self::Chat => "account.chat_prompt",
            Self::Review => "account.review_prompt",
        }
    }
}

fn wall_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs())
}

fn challenge_expired(challenge: &DeviceCode, now: u64) -> bool {
    challenge.expires_at <= Timestamp(now)
}

/// How the caption chip reads: neutral for unknown or settling state, green for a usable balance,
/// and the error ink for a blocked or empty balance. The short label always carries the meaning,
/// so colour is never the only signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChipTone {
    Neutral,
    Available,
    Blocked,
}

/// Draw the product account and available credits on the start page.
///
/// Returns whether anything was drawn, so the caller can keep the page spacing unchanged when
/// no account port is installed (the default, and the browser build).
pub fn show_summary(app: &mut SolveApp, ui: &mut Ui) -> bool {
    if !app.account.is_installed() {
        return false;
    }
    let t = Tokens::get();
    let view = app.account.view().clone();
    let notice = app.account.notice().cloned();
    let guide = app.account.delete_account_guide();
    let pending = app.account.pending_action();
    let title = app.tr("account.title").to_string();
    let signed_out = app.tr("account.signed_out").to_string();
    let sign_in_label = app.tr("account.sign_in").to_string();
    let sign_in_status = app.account.sign_in_status();
    let signed_in = app.tr("account.signed_in").to_string();
    let unknown = app.tr("account.credits_unknown").to_string();
    let zero = app.tr("account.zero_credits").to_string();
    let zero_chat = app.tr("account.zero_credits_chat").to_string();
    let held = app.tr("account.credits_held").to_string();
    let chat_unavailable = app.tr("account.chat_unavailable").to_string();
    let unavailable = app.tr("account.unavailable").to_string();
    let sign_out_label = app.tr("account.sign_out").to_string();
    let switch_label = app.tr("account.switch_account").to_string();
    let signing_in = app.tr("account.signing_in").to_string();
    let signing_out = app.tr("account.signing_out").to_string();
    let switching = app.tr("account.switching").to_string();
    let action_pending = app.tr("account.action_pending").to_string();
    let blocked = view
        .tool_blocked
        .filter(|code| !matches!(code, RefusalCode::CreditsExhausted | RefusalCode::CreditsHeld))
        .map(|code| refusal_text(app, code));
    let notice = notice.map(|refusal| refusal_text(app, refusal.code));
    let mut sign_in_clicked = false;
    let mut sign_out_clicked = false;
    let mut switch_clicked = false;

    FileView::section(ui, &title, t.space_10, |ui| {
        if view.signed_in {
            match view.account_label.as_deref() {
                Some(label) => FileView::detail(ui, &crate::i18n::format("account.signed_in_as", &[("account", label)])),
                None => FileView::detail(ui, &signed_in),
            }
            match view.available {
                Some(credits) if credits > 0 => FileView::detail(ui, &crate::i18n::format("account.available", &[("credits", &credits.to_string())])),
                Some(_) => {}
                None => FileView::detail(ui, &unknown),
            }
            // Settling is only reported while credits are actually held, and then it is the one
            // explanation for the missing available amount.
            if view.held > 0 {
                FileView::detail(ui, &crate::i18n::format("account.settling", &[("held", &view.held.to_string())]));
            } else if matches!(view.tool_blocked, Some(RefusalCode::CreditsHeld)) {
                FileView::detail(ui, &held);
            }
            if view.no_credits() && view.held == 0 && !matches!(view.tool_blocked, Some(RefusalCode::CreditsHeld)) {
                FileView::detail(ui, if view.chat_available { &zero_chat } else { &zero });
            }
            if let Some(blocked) = &blocked {
                FileView::detail(ui, blocked);
            }
            if !view.chat_available {
                FileView::detail(ui, &chat_unavailable);
            }
            ui.add_space(t.space_8);
            let enabled = pending.is_none();
            ui.horizontal(|ui| {
                let label = if pending == Some(Action::SignOut) { &signing_out } else { &sign_out_label };
                let options = if enabled { Button::Small } else { Button::Enabled(false) };
                let response = button(ui, label, options);
                publish("account:sign_out", &response);
                sign_out_clicked = response.clicked();
                ui.add_space(t.space_8);
                let label = if pending == Some(Action::SwitchAccount) { &switching } else { &switch_label };
                let response = button(ui, label, options);
                publish("account:switch_account", &response);
                switch_clicked = response.clicked();
            });
            if pending.is_some() {
                FileView::detail(ui, &action_pending);
            }
            if let Some(guide) = &guide {
                if !guide.help_url.is_empty() {
                    FileView::detail(ui, &crate::i18n::format("account.delete_guide_help", &[("url", guide.help_url)]));
                }
                for step in guide.steps {
                    // The host may pass a catalog id (localized here) or host-authored text.
                    FileView::detail(ui, app.tr(step));
                }
            }
        } else if view.sign_in_visible {
            FileView::detail(ui, &signed_out);
            challenge_controls(app, ui, ChallengeStyle::Summary);
            let expired = app.account.sign_in_expired();
            let sign_in_running = sign_in_status.waiting_for_worker() && !expired;
            let retry = sign_in_status == SignInStatus::Failed || expired;
            let label = if sign_in_running {
                &signing_in
            } else if retry {
                app.tr("account.sign_in_retry")
            } else {
                &sign_in_label
            };
            let options = if pending.is_none() && !sign_in_running { Button::Primary } else { Button::Enabled(false) };
            let response = button(ui, label, options);
            publish("account:sign_in", &response);
            sign_in_clicked = response.clicked();
            if pending.is_some() {
                FileView::detail(ui, &action_pending);
            }
        } else {
            FileView::detail(ui, &unavailable);
        }
        if let Some(notice) = &notice {
            text(ui, notice, Text::Error("account_notice"));
        }
    });

    if let Some(label) = &view.own_model_label {
        let own_title = app.tr("account.own_model_title").to_string();
        FileView::section(ui, &own_title, t.space_10, |ui| {
            FileView::detail(ui, label);
        });
    }

    if sign_in_clicked {
        app.account.request(Action::SignIn);
    }
    if sign_out_clicked {
        app.account.request(Action::SignOut);
    }
    if switch_clicked {
        app.account.request(Action::SwitchAccount);
    }
    true
}

/// The short, localized caption label for the permanent account entry. A signed-out build with
/// account support says "Sign in"; a signed-in account reports its credit state.
pub(crate) fn chip_label(app: &SolveApp, view: &AccountView) -> Option<String> {
    if !view.signed_in {
        return view.sign_in_visible.then(|| app.tr("account.sign_in").to_string());
    }
    Some(match view.available {
        Some(credits) if credits > 0 => crate::i18n::format("account.chip_credits", &[("credits", &credits.to_string())]),
        Some(_) if view.held > 0 => app.tr("account.chip_settling").to_string(),
        Some(_) => app.tr("account.chip_zero").to_string(),
        None if view.held > 0 => app.tr("account.chip_settling").to_string(),
        None => app.tr("account.chip_unknown").to_string(),
    })
}

/// The chip's tone. Unknown and settling stay neutral; only a usable balance or a blocked/empty
/// one changes the ink.
pub(crate) fn chip_tone(view: &AccountView) -> ChipTone {
    if !view.signed_in {
        return ChipTone::Neutral;
    }
    if view.available == Some(0) || view.tool_blocked.is_some_and(|code| !matches!(code, RefusalCode::CreditsHeld)) {
        ChipTone::Blocked
    } else if view.available.is_some_and(|credits| credits > 0) {
        ChipTone::Available
    } else {
        ChipTone::Neutral
    }
}

/// The width the chip needs at the shared font, for the bar's right-cluster reservation.
pub(crate) fn status_chip_width(ui: &Ui, app: &SolveApp) -> Option<f32> {
    if !app.account.is_installed() {
        return None;
    }
    let label = chip_label(app, app.account.view())?;
    let t = Tokens::get();
    let galley = ui.painter().layout_no_wrap(label, FontId::proportional(t.font_small), t.app_bar_text);
    Some(galley.size().x + 16.0)
}

/// A small, labelled pill at `slot` in the caption strip, clear of the window buttons, Report a
/// bug and the version link. The slot is empty unless the shared bar layout reserved room for it.
///
/// This is the permanent desktop account entry: signed out it says "Sign in", signed in it shows
/// the short credit state. A click opens the shared account menu; no paint pass calls the port and
/// no browser opens without a queued explicit action.
pub fn status_chip(app: &mut SolveApp, ui: &Ui, slot: Rect) {
    if slot.width() <= 0.0 {
        return;
    }
    let view = app.account.view().clone();
    let Some(label) = chip_label(app, &view) else { return };
    let t = Tokens::get();
    let ink = match chip_tone(&view) {
        ChipTone::Available => t.form_success,
        ChipTone::Blocked => t.form_error,
        ChipTone::Neutral => t.app_bar_text,
    };
    let response = ui.interact(slot, Id::new("titlebar:account_status"), Sense::click());
    let hot = response.hovered() || response.has_focus();
    ui.painter().rect_filled(slot, t.radius_sm, if hot { t.app_bar_hover } else { t.app_bar });
    ui.painter().rect_stroke(slot, t.radius_sm, Stroke::new(1.0, t.app_bar_separator), StrokeKind::Inside);
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().text(slot.center(), Align2::CENTER_CENTER, &label, FontId::proportional(t.font_small), ink);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label));
    let tip = if view.signed_in { chip_tip(app, &view) } else { app.tr("account.menu_tooltip").to_string() };
    let response = response.backing_on_hover_text(&tip);
    crate::scenario::publish_handle("titlebar:account_status", slot.center());
    crate::scenario::publish_rect("titlebar:account_status", slot);
    Container::new(ui.ctx(), Surface::Popup).menu(&response).frame(Menu::frame()).show(|ui| show_menu(app, ui));
}

/// Whether this build offers the permanent account entry at all: an installed port and either a
/// signed-in account or an explicit sign-in action. Unavailable and browser builds show neither.
pub(crate) fn menu_available(app: &SolveApp) -> bool {
    if !app.account.is_installed() {
        return false;
    }
    let view = app.account.view();
    view.signed_in || view.sign_in_visible
}

/// The File menu's compact fallback when the caption chip lacks room. It opens the same
/// [`show_menu`] body as the chip, so sign-in, browser, sign-out and switch stay identical.
/// Returns whether an action was taken.
pub(crate) fn compact_menu_entry(app: &mut SolveApp, ui: &mut Ui) -> bool {
    if !menu_available(app) {
        return false;
    }
    let label = format!("{}…", app.tr("account.title"));
    let entry = crate::ui_kit::Menu::button(ui, label, |ui| show_menu(app, ui));
    crate::scenario::publish_handle("account:overflow", entry.response.rect.center());
    crate::scenario::publish_rect("account:overflow", entry.response.rect);
    entry.inner == Some(true)
}

/// The permanent account menu: who is signed in, the one shared device challenge, and the
/// explicit sign-in, browser and sign-out actions. Returns whether an action was taken.
pub(crate) fn show_menu(app: &mut SolveApp, ui: &mut Ui) -> bool {
    let t = Tokens::get();
    let view = app.account.view().clone();
    let notice = app.account.notice().cloned();
    let pending = app.account.pending_action();
    let sign_in = app.account.sign_in_status();
    ui.set_min_width(t.space_24 * 8.0);
    let title = app.tr("account.title").to_string();
    crate::ui_kit::text(ui, &title, Text::Heading("account_menu_title"));
    ui.add_space(t.space_6);
    let mut sign_in_clicked = false;
    let mut sign_out_clicked = false;
    let mut switch_clicked = false;
    if view.signed_in {
        match view.account_label.as_deref() {
            Some(label) => {
                crate::ui_kit::text(ui, &crate::i18n::format("account.signed_in_as", &[("account", label)]), Text::Note("account_menu_signed_in"));
            }
            None => {
                let signed_in = app.tr("account.signed_in").to_string();
                crate::ui_kit::text(ui, &signed_in, Text::Note("account_menu_signed_in"));
            }
        }
        if let Some(credits) = view.available {
            crate::ui_kit::text(
                ui,
                &crate::i18n::format("account.available", &[("credits", &credits.to_string())]),
                Text::Note("account_menu_credits"),
            );
        }
        let enabled = pending.is_none();
        let options = if enabled { Button::Small } else { Button::Enabled(false) };
        ui.horizontal(|ui| {
            let sign_out_label = app.tr("account.sign_out");
            let sign_out = button(ui, sign_out_label, options);
            publish("account:sign_out", &sign_out);
            sign_out_clicked = sign_out.clicked();
            ui.add_space(t.space_8);
            let switch_label = app.tr("account.switch_account");
            let switch = button(ui, switch_label, options);
            publish("account:switch_account", &switch);
            switch_clicked = switch.clicked();
        });
    } else if view.sign_in_visible {
        let signed_out = app.tr("account.signed_out").to_string();
        crate::ui_kit::text(ui, &signed_out, Text::Note("account_menu_signed_out"));
        challenge_controls(app, ui, ChallengeStyle::Inline);
        let expired = app.account.sign_in_expired();
        let running = sign_in.waiting_for_worker() && !expired;
        let label = if running {
            app.tr("account.signing_in")
        } else if sign_in == SignInStatus::Failed || expired {
            app.tr("account.sign_in_retry")
        } else {
            app.tr("account.sign_in")
        };
        let options = if pending.is_none() && !running { Button::Primary } else { Button::Enabled(false) };
        let response = button(ui, label, options);
        publish("account:sign_in", &response);
        sign_in_clicked = response.clicked();
    } else {
        let unavailable = app.tr("account.unavailable").to_string();
        crate::ui_kit::text(ui, &unavailable, Text::Note("account_menu_unavailable"));
    }
    if let Some(notice) = notice {
        let message = refusal_text(app, notice.code);
        text(ui, &message, Text::Error("account_menu_notice"));
    }
    if pending.is_some() {
        let waiting = app.tr("account.action_pending").to_string();
        crate::ui_kit::text(ui, &waiting, Text::Note("account_menu_pending"));
    }
    if sign_in_clicked {
        app.account.request(Action::SignIn);
    }
    if sign_out_clicked {
        app.account.request(Action::SignOut);
    }
    if switch_clicked {
        app.account.request(Action::SwitchAccount);
    }
    sign_in_clicked || sign_out_clicked || switch_clicked
}

/// The shared device challenge, shown by the Home summary, the account menu and the contextual
/// AI Chat and Review prompts. It reads the cached prompt and status only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChallengeStyle {
    Summary,
    Inline,
}

fn challenge_controls(app: &mut SolveApp, ui: &mut Ui, style: ChallengeStyle) {
    let status = app.account.sign_in_status();
    let challenge = app.account.challenge().cloned();
    let t = Tokens::get();
    if status == SignInStatus::Starting && challenge.is_none() {
        let progress = app.tr("account.signing_in").to_string();
        detail_line(ui, style, &progress, false);
        return;
    }
    // A prompt is only current while the cached phase says a browser approval is waiting. A
    // completed, failed, retired or retried flow never leaves its old code visible.
    let Some(challenge) = challenge.filter(|_| status == SignInStatus::Waiting) else { return };
    let progress = app.tr("account.sign_in_waiting").to_string();
    detail_line(ui, style, &progress, false);
    let now = wall_seconds();
    let expired = challenge_expired(&challenge, now);
    if expired {
        let expired = app.tr("account.sign_in_expired").to_string();
        detail_line(ui, style, &expired, false);
    } else {
        let code = crate::i18n::format("account.sign_in_code", &[("code", &challenge.user_code)]);
        detail_line(ui, style, &code, true);
        let remaining = challenge.expires_at.0.saturating_sub(now);
        let minutes = remaining.div_ceil(60).max(1).to_string();
        let expires = crate::i18n::format("account.sign_in_expires", &[("minutes", &minutes)]);
        detail_line(ui, style, &expires, false);
    }
    let label = app.tr("account.sign_in_open_browser").to_string();
    let options = if expired { Button::Enabled(false) } else { Button::Small };
    let response = button(ui, &label, options);
    publish("account:open_browser", &response);
    if response.clicked() && !expired {
        app.account.request(Action::OpenVerification);
    }
    if style == ChallengeStyle::Inline {
        ui.add_space(t.space_4);
    }
}

fn detail_line(ui: &mut Ui, style: ChallengeStyle, message: &str, mono: bool) {
    match (style, mono) {
        (ChallengeStyle::Summary, _) => {
            FileView::detail(ui, message);
        }
        (ChallengeStyle::Inline, true) => {
            crate::ui_kit::text(ui, message, Text::MonoNote("account_sign_in_code"));
        }
        (ChallengeStyle::Inline, false) => {
            crate::ui_kit::text(ui, message, Text::Note("account_sign_in_status"));
        }
    }
}

/// An in-place product-account prompt for AI Chat or Review. It never replaces or speaks for the
/// selected agent/model's own authentication.
pub fn show_product_prompt(app: &mut SolveApp, ui: &mut Ui, context: PromptContext) -> bool {
    if !app.account.is_installed() {
        return false;
    }
    let view = app.account.view().clone();
    if !view.product_sign_in_required() {
        return false;
    }
    let t = Tokens::get();
    let title = app.tr("account.title").to_string();
    let message = app.tr(context.message()).to_string();
    let notice = app.account.notice().cloned();
    let status = app.account.sign_in_status();
    let pending = app.account.pending_action();
    let mut sign_in_clicked = false;
    FileView::section(ui, &title, t.space_8, |ui| {
        FileView::detail(ui, &message);
        challenge_controls(app, ui, ChallengeStyle::Summary);
        let expired = app.account.sign_in_expired();
        let running = status.waiting_for_worker() && !expired;
        let label = if running {
            app.tr("account.signing_in")
        } else if status == SignInStatus::Failed || expired {
            app.tr("account.sign_in_retry")
        } else {
            app.tr("account.sign_in")
        };
        let options = if pending.is_none() && !running { Button::Small } else { Button::Enabled(false) };
        let response = button(ui, label, options);
        publish(if context == PromptContext::Chat { "account:chat_sign_in" } else { "account:review_sign_in" }, &response);
        sign_in_clicked = response.clicked();
        if let Some(notice) = notice {
            let message = refusal_text(app, notice.code);
            text(ui, &message, Text::Error("account_context_notice"));
        }
    });
    if sign_in_clicked {
        app.account.request(Action::SignIn);
    }
    true
}

/// Hover text for the chip, with the same single explanation rule as the start page.
fn chip_tip(app: &SolveApp, view: &AccountView) -> String {
    let settling = crate::i18n::format("account.settling", &[("held", &view.held.to_string())]);
    let mut tip = if view.held > 0 {
        match view.available {
            Some(credits) if credits > 0 => crate::i18n::format("account.available", &[("credits", &credits.to_string())]) + " " + &settling,
            _ => settling,
        }
    } else {
        match view.available {
            Some(credits) if credits > 0 => crate::i18n::format("account.available", &[("credits", &credits.to_string())]),
            Some(_) if view.chat_available => app.tr("account.zero_credits_chat").to_string(),
            Some(_) => app.tr("account.zero_credits").to_string(),
            None => app.tr("account.credits_unknown").to_string(),
        }
    };
    if let Some(code) = view.tool_blocked.filter(|code| !matches!(code, RefusalCode::CreditsExhausted | RefusalCode::CreditsHeld)) {
        tip.push(' ');
        tip.push_str(&refusal_text(app, code));
    }
    if !view.chat_available {
        tip.push(' ');
        tip.push_str(app.tr("account.chat_unavailable"));
    }
    tip
}

fn publish(name: &str, response: &egui::Response) {
    crate::scenario::publish_handle(name, response.rect.center());
    crate::scenario::publish_rect(name, response.rect);
}

fn refusal_text(app: &SolveApp, code: RefusalCode) -> String {
    let id = match code {
        RefusalCode::AuthRequired => "account.reason_auth_required",
        RefusalCode::AuthUnavailable => "account.reason_auth_unavailable",
        RefusalCode::EmailUnverified => "account.reason_email_unverified",
        RefusalCode::AccountDisabled => "account.reason_account_disabled",
        RefusalCode::CapabilityDenied => "account.reason_capability_denied",
        RefusalCode::VersionUnsupported => "account.reason_version_unsupported",
        RefusalCode::ProtocolUnsupported => "account.reason_protocol_unsupported",
        RefusalCode::UnsafeToStart => "account.reason_unsafe_to_start",
        RefusalCode::CreditsExhausted => "account.zero_credits",
        RefusalCode::CreditsHeld => "account.credits_held",
        RefusalCode::OperationConflict | RefusalCode::OperationFinalized | RefusalCode::OperationOwnedElsewhere => "account.reason_operation",
    };
    app.tr(id).to_string()
}
