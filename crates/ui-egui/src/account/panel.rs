//! The home-page account summary and the application bar's credit status chip.

use std::time::{SystemTime, UNIX_EPOCH};

use egui::{Align2, FontId, Id, Rect, Sense, Stroke, StrokeKind, Ui};
use solidcraft_engine::entitlement::RefusalCode;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit::{BackingTooltipExt as _, Button, Container, FileView, Menu, Surface, Text, button, text};

use super::{AccountView, Action, SignInState};

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

    fn handle(self) -> &'static str {
        match self {
            Self::Chat => "agent_chat:sign_in",
            Self::Review => "review:sign_in",
        }
    }
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
    let signed_in = app.tr("account.signed_in").to_string();
    let unknown = app.tr("account.credits_unknown").to_string();
    let zero = app.tr("account.zero_credits").to_string();
    let zero_chat = app.tr("account.zero_credits_chat").to_string();
    let held = app.tr("account.credits_held").to_string();
    let chat_unavailable = app.tr("account.chat_unavailable").to_string();
    let unavailable = app.tr("account.unavailable").to_string();
    let sign_out_label = app.tr("account.sign_out").to_string();
    let switch_label = app.tr("account.switch_account").to_string();
    let signing_out = app.tr("account.signing_out").to_string();
    let switching = app.tr("account.switching").to_string();
    let action_pending = app.tr("account.action_pending").to_string();
    let chat_review_sign_in = app.tr("account.chat_review_sign_in").to_string();
    let blocked = view
        .tool_blocked
        .filter(|code| !matches!(code, RefusalCode::CreditsExhausted | RefusalCode::CreditsHeld))
        .map(|code| refusal_text(app, code));
    let notice = notice.map(|refusal| refusal_text(app, refusal.code));
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
                let response = ui.add_enabled_ui(enabled, |ui| button(ui, label, Button::Small)).inner;
                publish("account:sign_out", &response);
                sign_out_clicked = response.clicked();
                ui.add_space(t.space_8);
                let label = if pending == Some(Action::SwitchAccount) { &switching } else { &switch_label };
                let response = ui.add_enabled_ui(enabled, |ui| button(ui, label, Button::Small)).inner;
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
        } else if needs_sign_out_attention(&view) {
            retry_sign_out_clicked = sign_out_controls(app, ui, &view);
        } else if view.sign_in_visible {
            if view.sign_in_state == SignInState::Idle && pending.is_none() {
                FileView::detail(ui, &chat_review_sign_in);
            }
            sign_in_controls(app, ui, &view, "account:start", "account:sign_in", "account:sign_in_browser", "account:sign_in_cancel");
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

/// The short, localized caption label for the permanent account entry. A signed-out build with
/// account support says "Sign in"; a signed-in account reports its credit state.
pub(crate) fn chip_label(app: &SolveApp, view: &AccountView) -> Option<String> {
    if !view.signed_in {
        if needs_sign_out_attention(view) {
            return Some(app.tr("account.title").to_string());
        }
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
    // The menu hosts the shared challenge's copy feedback and manual disclosure. Clicking inside
    // must not dismiss it, or those secondary controls would close the surface they act on.
    Container::new(ui.ctx(), Surface::Popup)
        .menu(&response)
        .frame(Menu::frame())
        .close_behavior(crate::ui_kit::KEEP_OPEN_ON_CLICK)
        .show(|ui| show_menu(app, ui));
}

/// Whether this build offers the permanent account entry at all: an installed port and either a
/// signed-in account, an explicit sign-in action or sign-out cleanup.
pub(crate) fn menu_available(app: &SolveApp) -> bool {
    if !app.account.is_installed() {
        return false;
    }
    let view = app.account.view();
    view.signed_in || view.sign_in_visible || needs_sign_out_attention(view)
}

/// The File menu's compact fallback when the caption chip lacks room. It opens the same
/// [`show_menu`] body as the chip, so sign-in, browser, sign-out and switch stay identical.
/// Returns whether the File menu should close after the child action. Sign-in requests keep it
/// open so the waiting code and its retry remain visible; other account actions close it.
pub(crate) fn compact_menu_entry(app: &mut SolveApp, ui: &mut Ui) -> bool {
    if !menu_available(app) {
        return false;
    }
    let label = format!("{}…", app.tr("account.title"));
    // Match the caption chip: copy feedback and the manual disclosure need the popup to stay open.
    let entry = crate::ui_kit::Menu::button_keep_open(ui, label, |ui| show_menu(app, ui));
    crate::scenario::publish_handle("account:overflow", entry.response.rect.center());
    crate::scenario::publish_rect("account:overflow", entry.response.rect);
    if entry.inner != Some(true) {
        return false;
    }
    !matches!(app.account.pending_action(), Some(Action::SignIn | Action::OpenSignInBrowser))
}

/// The permanent account menu: who is signed in, the one shared device challenge, and the
/// explicit sign-in, browser and sign-out actions. Returns whether an action was taken.
pub(crate) fn show_menu(app: &mut SolveApp, ui: &mut Ui) -> bool {
    let t = Tokens::get();
    let view = app.account.view().clone();
    let notice = app.account.notice().cloned();
    let pending = app.account.pending_action();
    ui.set_min_width(t.space_24 * 8.0);
    let title = app.tr("account.title").to_string();
    crate::ui_kit::text(ui, &title, Text::Heading("account_menu_title"));
    ui.add_space(t.space_6);
    let mut sign_in_action = false;
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
    } else if needs_sign_out_attention(&view) {
        sign_out_clicked = sign_out_controls(app, ui, &view);
    } else if view.sign_in_visible {
        sign_in_action = sign_in_controls(app, ui, &view, "account:menu", "account:sign_in", "account:sign_in_browser", "account:sign_in_cancel");
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
    if sign_out_clicked {
        app.account.request(Action::SignOut);
    }
    if switch_clicked {
        app.account.request(Action::SwitchAccount);
    }
    sign_in_action || sign_out_clicked || switch_clicked
}

fn needs_sign_out_attention(view: &AccountView) -> bool {
    view.sign_out_pending || view.sign_out_recovery || view.sign_out_error.is_some()
}

fn sign_out_controls(app: &SolveApp, ui: &mut Ui, view: &AccountView) -> bool {
    if view.sign_out_pending {
        FileView::detail(ui, app.tr("account.signing_out"));
        return false;
    }
    let (label, handle) = if view.sign_out_recovery {
        FileView::detail(ui, app.tr("account.sign_out_recovery"));
        ("account.finish_sign_out", "account:finish_sign_out")
    } else if let Some(code) = view.sign_out_error {
        text(ui, &refusal_text(app, code), Text::Error("account_sign_out_error"));
        ("account.retry_sign_out", "account:retry_sign_out")
    } else {
        return false;
    };
    let response = ui.add_enabled_ui(app.account.pending_action().is_none(), |ui| button(ui, app.tr(label), Button::Primary)).inner;
    publish(handle, &response);
    response.clicked()
}

/// An in-place product-account prompt for AI Chat or Review. It never replaces or speaks for the
/// selected agent/model's own authentication.
pub fn show_product_prompt(app: &mut SolveApp, ui: &mut Ui, context: PromptContext) -> bool {
    show_inline_sign_in(app, ui, context.message(), context.handle())
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

/// The short code and explicit actions, with a manual address behind a disclosure. Shared by the
/// account menu and the Chat/Review inline prompts so both surfaces can finish the same flow.
/// The browser action is requested only on the platform gesture that activated this button; an
/// app-injected press, a mixed gesture, or a replayed synthetic key cannot launch it.
fn waiting_sign_in_controls(app: &mut SolveApp, ui: &mut Ui, view: &AccountView, surface: &str, browser_handle: &str, cancel_handle: &str) -> bool {
    let t = Tokens::get();
    let prompt = view.sign_in_prompt.as_ref();
    let prompt_id = prompt.map(|code| egui::Id::new((&code.user_code, &code.verification_uri, code.expires_at.0)));
    // Copy feedback and the manual disclosure belong to one surface and this exact challenge. A
    // replacement attempt starts collapsed and uncopied, even when the prior prompt is still cached.
    let state_id = ui.make_persistent_id(("sign_in_controls", surface));
    let (mut copied, mut manual) = ui.data(|data| {
        data.get_temp::<(Option<Id>, bool, bool)>(state_id)
            .filter(|(prior, _, _)| *prior == prompt_id)
            .map_or((false, false), |(_, copied, manual)| (copied, manual))
    });
    if let Some(code) = prompt {
        FileView::detail(ui, &crate::i18n::format("account.sign_in_code", &[("code", &code.user_code)]));
    }
    let action = ui
        .add_enabled_ui(app.account.pending_action().is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let mut action = None;
                if let Some(code) = prompt {
                    let response = button(ui, app.tr("account.sign_in_browser"), Button::Primary);
                    publish(browser_handle, &response);
                    if app.trusted_click(&response) && !device_code_expired(code) {
                        action = Some(Action::OpenSignInBrowser);
                    }
                    ui.add_space(t.space_8);
                    let label = if copied { "account.device_code_copied" } else { "account.copy_device_code" };
                    let copy = button(ui, app.tr(label), Button::Normal);
                    publish(&format!("{browser_handle}:copy"), &copy);
                    if app.trusted_click(&copy) && !device_code_expired(code) {
                        ui.ctx().copy_text(code.user_code.clone());
                        copied = true;
                    }
                }
                let response = button(ui, app.tr("account.sign_in_cancel"), Button::Normal);
                publish(cancel_handle, &response);
                if response.clicked() { Some(Action::CancelSignIn) } else { action }
            })
            .inner
        })
        .inner;
    if let Some(address) = prompt.and_then(|code| manual_address(&code.verification_uri)) {
        let response = crate::ui_kit::disclosure_label_button(ui, app.tr("account.sign_in_manually"), manual);
        publish(&format!("{browser_handle}:manual"), &response);
        if response.clicked() {
            manual = !manual;
        }
        if manual {
            FileView::detail(ui, &crate::i18n::format("account.sign_in_address", &[("url", address)]));
        }
    }
    ui.data_mut(|data| data.insert_temp(state_id, (prompt_id, copied, manual)));
    if let Some(action) = action {
        app.account.request(action);
    }
    action.is_some()
}

/// The manual fallback shows only a safe HTTPS origin and path. Query and fragment can carry the
/// provider's one-time code or other account data, so the disclosure never reveals them.
pub(super) fn manual_address(uri: &str) -> Option<&str> {
    let address = uri.split(['?', '#']).next().unwrap_or_default();
    let rest = address.strip_prefix("https://")?;
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.split(':').next().unwrap_or_default();
    (!host.is_empty() && !rest.bytes().any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\' || byte == b'@'))
        .then_some(address)
}

/// A local, in-place sign-in prompt for a refusal surface such as Chat or Review. It only
/// enqueues the same explicit action the account menu uses; it never opens a browser itself.
/// While a login waits it shows the same code, copy, browser and cancel controls
/// as the account menu, so the surface that refused the operation can complete the flow.
/// `handle` must be unique on the surface so scripted UI checks can address the button.
pub fn show_inline_sign_in(app: &mut SolveApp, ui: &mut Ui, message_id: &str, handle: &str) -> bool {
    if !app.account.is_installed() || !app.account.view().product_sign_in_required() {
        return false;
    }
    let view = app.account.view().clone();
    if view.sign_in_state == SignInState::Idle {
        FileView::detail(ui, app.tr(message_id));
    }
    sign_in_controls(app, ui, &view, handle, handle, &format!("{handle}:browser"), &format!("{handle}:cancel"));
    if let Some(notice) = app.account.notice() {
        text(ui, &refusal_text(app, notice.code), Text::Error("account_context_notice"));
    }
    ui.add_space(Tokens::get().space_4);
    true
}

fn device_code_expired(code: &super::DeviceCode) -> bool {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
    code.expires_at.0 <= now
}

fn sign_in_controls(
    app: &mut SolveApp,
    ui: &mut Ui,
    view: &AccountView,
    surface: &str,
    handle: &str,
    browser_handle: &str,
    cancel_handle: &str,
) -> bool {
    let pending = app.account.pending_action();
    let label = match view.sign_in_state {
        SignInState::Starting => {
            FileView::detail(ui, app.tr("account.sign_in_starting"));
            return false;
        }
        SignInState::Waiting if !view.sign_in_prompt.as_ref().is_some_and(device_code_expired) => {
            FileView::detail(ui, app.tr("account.sign_in_waiting"));
            return waiting_sign_in_controls(app, ui, view, surface, browser_handle, cancel_handle);
        }
        SignInState::Waiting | SignInState::Expired | SignInState::Failed => {
            let text = if view.sign_in_state != SignInState::Failed {
                app.tr("account.sign_in_expired").to_string()
            } else {
                view.sign_in_error.map(|code| refusal_text(app, code)).unwrap_or_else(|| app.tr("account.sign_in_failed").to_string())
            };
            FileView::detail(ui, &text);
            "account.sign_in_retry"
        }
        SignInState::Idle if pending == Some(Action::SignIn) => "account.signing_in",
        SignInState::Idle => "account.sign_in",
    };
    let response = ui.add_enabled_ui(pending.is_none(), |ui| button(ui, app.tr(label), Button::Primary)).inner;
    publish(handle, &response);
    if response.clicked() {
        app.account.request(Action::SignIn);
    }
    response.clicked()
}

fn publish(name: &str, response: &egui::Response) {
    crate::scenario::publish_handle(name, response.rect.center());
    crate::scenario::publish_rect(name, response.rect);
}

pub(crate) fn refusal_text(app: &SolveApp, code: RefusalCode) -> String {
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
