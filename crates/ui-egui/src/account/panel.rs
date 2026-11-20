//! The home-page account summary and the application bar's credit status chip.

use egui::{Id, Rect, Sense, Stroke, Ui, pos2, vec2};
use solidcraft_engine::entitlement::RefusalCode;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit::{BackingTooltipExt as _, Button, FileView, Text, button, text};

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
            if view.settling || view.held > 0 {
                FileView::detail(ui, &crate::i18n::format("account.settling", &[("held", &view.held.to_string())]));
            }
            if view.no_credits() && !matches!(view.tool_blocked, Some(RefusalCode::CreditsHeld)) {
                FileView::detail(ui, if view.chat_available { &zero_chat } else { &zero });
            }
            if matches!(view.tool_blocked, Some(RefusalCode::CreditsHeld)) {
                FileView::detail(ui, &held);
            }
            if let Some(blocked) = &blocked {
                FileView::detail(ui, blocked);
            }
            if !view.chat_available && (view.no_credits() || view.tool_blocked.is_some()) {
                FileView::detail(ui, &chat_unavailable);
            }
            ui.add_space(t.space_8);
            ui.horizontal(|ui| {
                let response = button(ui, &sign_out_label, Button::Small);
                publish("account:sign_out", &response);
                sign_out_clicked = response.clicked();
                ui.add_space(t.space_8);
                let response = button(ui, &switch_label, Button::Small);
                publish("account:switch_account", &response);
                switch_clicked = response.clicked();
            });
            if let Some(guide) = &guide {
                if !guide.help_url.is_empty() {
                    FileView::detail(ui, &crate::i18n::format("account.delete_guide_help", &[("url", guide.help_url)]));
                }
                for step in guide.steps {
                    FileView::detail(ui, step);
                }
            }
        } else if view.sign_in_visible {
            FileView::detail(ui, &signed_out);
            let response = button(ui, &sign_in_label, Button::Primary);
            publish("account:sign_in", &response);
            sign_in_clicked = response.clicked();
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
        app.account.sign_in();
    }
    if sign_out_clicked {
        app.account.sign_out();
    }
    if switch_clicked {
        app.account.switch_account();
    }
    true
}

/// A small status chip in the caption strip, clear of the window buttons.
///
/// The home summary carries the full copy; this chip only shows that account status exists and
/// explains it on hover. It is not a login action.
pub fn status_chip(app: &SolveApp, ui: &Ui, bar: Rect) {
    if !app.account.is_installed() {
        return;
    }
    let view = app.account.view();
    let t = Tokens::get();
    let (color, tip) = if view.signed_in {
        let mut tip = match view.available {
            Some(credits) if credits > 0 => crate::i18n::format("account.available", &[("credits", &credits.to_string())]),
            Some(_) => app.tr("account.zero_credits").to_string(),
            None => app.tr("account.credits_unknown").to_string(),
        };
        if view.settling || view.held > 0 {
            tip.push(' ');
            tip.push_str(&crate::i18n::format("account.settling", &[("held", &view.held.to_string())]));
        }
        let color = if view.tool_blocked.is_some() || view.available == Some(0) {
            t.form_error
        } else if view.settling || view.held > 0 {
            t.pref_note
        } else {
            t.form_success
        };
        (color, tip)
    } else if view.sign_in_visible {
        return;
    } else {
        (t.form_dim, app.tr("account.unavailable").to_string())
    };
    let rect = status_chip_rect(bar);
    let response = ui.interact(rect, Id::new("titlebar:account_status"), Sense::hover());
    ui.painter().circle_filled(rect.center(), rect.width() / 2.0, color);
    ui.painter().circle_stroke(rect.center(), rect.width() / 2.0, Stroke::new(1.0, t.app_bar));
    let _ = response.backing_on_hover_text(&tip);
    crate::scenario::publish_handle("titlebar:account_status", rect.center());
    crate::scenario::publish_rect("titlebar:account_status", rect);
}

/// The chip sits in the gap between the version and the caption buttons, never on them.
pub(crate) fn status_chip_rect(bar: Rect) -> Rect {
    let size = 8.0;
    Rect::from_center_size(pos2(bar.right() - crate::titlebar::WIDTH - 6.0, bar.center().y), vec2(size, size))
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
