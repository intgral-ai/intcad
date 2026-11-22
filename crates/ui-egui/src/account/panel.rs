//! The home-page account summary and the application bar's credit status chip.

use egui::{Align2, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui};
use solidcraft_engine::entitlement::RefusalCode;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit::{BackingTooltipExt as _, Button, FileView, Text, button, text};

use super::{AccountView, Action, SignInState};

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
    let sign_in_starting = app.tr("account.sign_in_starting").to_string();
    let sign_in_waiting = app.tr("account.sign_in_waiting").to_string();
    let sign_in_browser = app.tr("account.sign_in_browser").to_string();
    let sign_in_cancel = app.tr("account.sign_in_cancel").to_string();
    let sign_in_expired = app.tr("account.sign_in_expired").to_string();
    let sign_in_failed = app.tr("account.sign_in_failed").to_string();
    let sign_in_retry = app.tr("account.sign_in_retry").to_string();
    let chat_review_sign_in = app.tr("account.chat_review_sign_in").to_string();
    let retry_sign_out = app.tr("account.retry_sign_out").to_string();
    let blocked = view
        .tool_blocked
        .filter(|code| !matches!(code, RefusalCode::CreditsExhausted | RefusalCode::CreditsHeld))
        .map(|code| refusal_text(app, code));
    let notice = notice.map(|refusal| refusal_text(app, refusal.code));
    let mut sign_in_clicked = false;
    let mut sign_in_browser_clicked = false;
    let mut cancel_sign_in_clicked = false;
    let mut retry_sign_out_clicked = false;
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
            FileView::detail(ui, &chat_review_sign_in);
            if view.sign_out_pending {
                FileView::detail(ui, &signing_out);
            }
            if let Some(code) = view.sign_out_error {
                text(ui, &refusal_text(app, code), Text::Error("account_sign_out_error"));
                let response = button(ui, &retry_sign_out, Button::Primary);
                publish("account:retry_sign_out", &response);
                retry_sign_out_clicked = response.clicked();
            }
            match view.sign_in_state {
                SignInState::Starting => {
                    FileView::detail(ui, &sign_in_starting);
                }
                SignInState::Waiting => {
                    FileView::detail(ui, &sign_in_waiting);
                    if let Some(code) = &view.sign_in_prompt {
                        FileView::detail(ui, &crate::i18n::format("account.sign_in_code", &[("code", &code.user_code)]));
                        FileView::detail(ui, &crate::i18n::format("account.sign_in_address", &[("url", &code.verification_uri)]));
                        ui.horizontal(|ui| {
                            let response = button(ui, &sign_in_browser, Button::Primary);
                            publish("account:sign_in_browser", &response);
                            sign_in_browser_clicked = response.clicked();
                            ui.add_space(t.space_8);
                            let response = button(ui, &sign_in_cancel, Button::Normal);
                            publish("account:sign_in_cancel", &response);
                            cancel_sign_in_clicked = response.clicked();
                        });
                    }
                }
                SignInState::Expired | SignInState::Failed => {
                    if view.sign_in_state == SignInState::Expired {
                        FileView::detail(ui, &sign_in_expired);
                    } else {
                        let reason = view.sign_in_error.map(|code| refusal_text(app, code)).unwrap_or_else(|| sign_in_failed.clone());
                        FileView::detail(ui, &reason);
                    }
                    let response = button(ui, &sign_in_retry, Button::Primary);
                    publish("account:sign_in", &response);
                    sign_in_clicked = response.clicked();
                }
                SignInState::Idle => {
                    let label = if pending == Some(Action::SignIn) { &signing_in } else { &sign_in_label };
                    let options = if pending.is_none() { Button::Primary } else { Button::Enabled(false) };
                    let response = button(ui, label, options);
                    publish("account:sign_in", &response);
                    sign_in_clicked = response.clicked();
                    if pending.is_some() {
                        FileView::detail(ui, &action_pending);
                    }
                }
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
    if sign_in_browser_clicked {
        app.account.request(Action::OpenSignInBrowser);
    }
    if cancel_sign_in_clicked {
        app.account.request(Action::CancelSignIn);
    }
    if retry_sign_out_clicked {
        app.account.request(Action::SignOut);
    }
    if sign_out_clicked {
        app.account.request(Action::SignOut);
    }
    if switch_clicked {
        app.account.request(Action::SwitchAccount);
    }
    true
}

/// The short, localized caption label. No label for a signed-out or absent account: sign-in lives
/// on the start page, and the chip only reports an active account's credit state.
pub(crate) fn chip_label(app: &SolveApp, view: &AccountView) -> Option<String> {
    if !view.signed_in {
        return None;
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
/// The home summary carries the full copy; this chip shows the short balance state and explains it
/// on hover. It is not a login action, is not a hit target, and is never colour-only: its short
/// localized text is painted and exposed as the widget label.
pub fn status_chip(app: &SolveApp, ui: &Ui, slot: Rect) {
    if slot.width() <= 0.0 {
        return;
    }
    let view = app.account.view();
    let Some(label) = chip_label(app, view) else { return };
    let t = Tokens::get();
    let ink = match chip_tone(view) {
        ChipTone::Available => t.form_success,
        ChipTone::Blocked => t.form_error,
        ChipTone::Neutral => t.app_bar_text,
    };
    let response = ui.interact(slot, Id::new("titlebar:account_status"), Sense::hover());
    ui.painter().rect_filled(slot, t.radius_sm, t.app_bar);
    ui.painter().rect_stroke(slot, t.radius_sm, Stroke::new(1.0, t.app_bar_separator), StrokeKind::Inside);
    ui.painter().text(slot.center(), Align2::CENTER_CENTER, &label, FontId::proportional(t.font_small), ink);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &label));
    let tip = chip_tip(app, view);
    let _ = response.backing_on_hover_text(&tip);
    crate::scenario::publish_handle("titlebar:account_status", slot.center());
    crate::scenario::publish_rect("titlebar:account_status", slot);
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

/// A local, in-place sign-in prompt for a refusal surface such as Chat or Review. It only
/// enqueues the same explicit action the account menu uses; it never opens a browser itself.
/// `handle` must be unique on the surface so scripted UI checks can address the button.
pub fn show_inline_sign_in(app: &mut SolveApp, ui: &mut Ui, message_id: &str, handle: &str) {
    if !app.account.is_installed() {
        return;
    }
    let view = app.account.view().clone();
    if view.signed_in || !view.sign_in_visible {
        return;
    }
    let t = Tokens::get();
    let message = app.tr(message_id).to_string();
    FileView::detail(ui, &message);
    if view.sign_in_state.pending() {
        FileView::detail(ui, app.tr("account.sign_in_waiting"));
        return;
    }
    let label = match view.sign_in_state {
        SignInState::Expired | SignInState::Failed => app.tr("account.sign_in_retry").to_string(),
        _ => app.tr("account.sign_in").to_string(),
    };
    let response = button(ui, &label, Button::Primary);
    publish(handle, &response);
    if response.clicked() {
        app.account.request(Action::SignIn);
    }
    ui.add_space(t.space_4);
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
