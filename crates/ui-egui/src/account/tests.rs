//! Scripted, in-memory account port tests. Nothing here reaches a provider, keychain or network.

use std::sync::{Arc, Mutex};

use egui::Pos2;
use solidcraft_engine::entitlement::{Refusal, RefusalCode};

use super::*;

struct FakeAccountPort {
    view: AccountView,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeAccountPort {
    fn signed_out(calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { view: AccountView::signed_out(), calls }
    }

    fn with_view(view: AccountView, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { view, calls }
    }

    fn log(&self, name: &str) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(name.to_string());
        }
    }
}

impl AccountPort for FakeAccountPort {
    fn view(&self) -> AccountView {
        self.log("view");
        self.view.clone()
    }

    fn refresh_status(&mut self) -> Result<AccountView, Refusal> {
        self.log("refresh_status");
        Ok(self.view.clone())
    }

    fn sign_in(&mut self) -> Result<(), Refusal> {
        self.log("sign_in");
        self.view.signed_in = true;
        self.view.sign_in_visible = false;
        self.view.available = Some(42);
        self.view.chat_available = true;
        self.view.tool_blocked = None;
        Ok(())
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        self.log("sign_out");
        self.view = AccountView::signed_out();
        Ok(())
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        self.log("switch_account");
        self.view = AccountView::signed_out();
        Ok(())
    }

    fn delete_account_guide(&self) -> DeleteAccountGuide {
        DeleteAccountGuide { help_url: "https://example.invalid/account/delete", steps: &["Confirm the request."] }
    }

    fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        self.log("begin_device_code");
        Err(Refusal::new(RefusalCode::AuthUnavailable, "not in this fake"))
    }

    fn own_model_view(&self) -> Option<OwnModelView> {
        self.log("own_model_view");
        None
    }
}

fn signed_in_view() -> AccountView {
    AccountView {
        signed_in: true,
        sign_in_visible: false,
        account_label: Some("alice@example.com".into()),
        available: Some(999),
        held: 0,
        settling: false,
        chat_available: true,
        tool_blocked: None,
        own_model_label: None,
    }
}

#[test]
fn signed_out_shows_sign_in_and_no_quota() {
    let view = AccountView::signed_out();
    assert!(view.sign_in_visible);
    assert!(!view.signed_in);
    assert_eq!(view.available, None);
}

#[test]
fn no_installed_port_shows_no_login_or_quota() {
    let state = State::default();
    assert!(!state.is_installed());
    let view = state.view();
    assert!(!view.sign_in_visible);
    assert!(!view.signed_in);
    assert_eq!(view.available, None);
}

#[test]
fn ui_calls_only_authorized_account_port_methods() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut state = State::with_port(Box::new(FakeAccountPort::signed_out(calls.clone())));
    assert!(!state.view().signed_in);
    state.sign_in();
    assert!(state.view().signed_in);
    let calls = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(calls, vec!["view", "sign_in", "view"]);
}

#[test]
fn signed_in_shows_available_and_settling_when_held_positive() {
    let view = AccountView { available: Some(999), held: 1, settling: true, ..signed_in_view() };
    assert_eq!(view.available, Some(999));
    assert!(view.settling);
    let texts = render(view);
    assert!(texts.iter().any(|(text, _)| text.contains("999")), "{texts:?}");
    assert!(texts.iter().any(|(text, _)| text.to_lowercase().contains("settling")), "{texts:?}");
}

#[test]
fn zero_credits_still_allows_plain_chat_but_blocks_billable_tools() {
    let view = AccountView { available: Some(0), chat_available: true, tool_blocked: Some(RefusalCode::CreditsExhausted), ..signed_in_view() };
    assert!(view.chat_available);
    assert_eq!(view.tool_blocked, Some(RefusalCode::CreditsExhausted));
    let texts = render(view);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("plain chat"), "{texts:?}");
}

#[test]
fn insufficient_credits_explains_and_has_no_retry_or_payment_entry() {
    let view = AccountView { available: Some(0), chat_available: true, tool_blocked: Some(RefusalCode::CreditsExhausted), ..signed_in_view() };
    let texts = render(view);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    for forbidden in ["retry", "buy", "purchase", "payment", "upgrade", "add credits"] {
        assert!(!joined.contains(forbidden), "`{forbidden}` appears in {texts:?}");
    }
}

#[test]
fn own_model_login_is_shown_separately_from_product_account() {
    let view = AccountView { own_model_label: Some("Anthropic · signed in".into()), ..signed_in_view() };
    let texts = render(view);
    let product = texts.iter().find(|(text, _)| text.contains("alice@example.com")).expect("product account label");
    let own = texts.iter().find(|(text, _)| text.contains("Anthropic")).expect("own model label");
    assert!((product.1.y - own.1.y).abs() > 8.0, "the two identities share a row: {product:?} {own:?}");
}

#[test]
fn unavailable_shows_no_login_and_no_quota() {
    let view = UnavailableAccount.view();
    assert!(!view.sign_in_visible);
    assert!(!view.signed_in);
    assert_eq!(view.available, None);
    let texts = render(view);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("not available"), "{texts:?}");
    assert!(!joined.contains("sign in"), "the unavailable build offers a login: {texts:?}");
    assert!(!joined.contains("available credits"), "{texts:?}");
}

#[test]
fn status_chip_stays_clear_of_the_caption_buttons() {
    let bar = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 34.0));
    let chip = super::panel::status_chip_rect(bar);
    assert!(chip.right() <= bar.right() - crate::titlebar::WIDTH - 2.0);
    assert!(chip.left() >= bar.right() - crate::titlebar::WIDTH - 12.0);
}

#[test]
fn clicking_sign_in_is_the_only_action_that_starts_login() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut harness = crate::scenario::Harness::new();
    harness.app.account = State::with_port(Box::new(FakeAccountPort::signed_out(calls.clone())));
    harness.app.home.open = true;
    harness.frames(2);
    assert!(crate::scenario::handle_at("account:sign_in").is_some(), "handles: {:?}", crate::scenario::handles(""));
    crate::test_support::click(&mut harness, "account:sign_in");
    let calls = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(calls, vec!["view", "sign_in", "view"]);
}

fn render(view: AccountView) -> Vec<(String, Pos2)> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(Box::new(FakeAccountPort::with_view(view, calls)));
    let ctx = crate::fonts::test_context();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))), ..Default::default() };
    crate::scenario::clear_handles();
    let output = crate::test_support::frame(&ctx, input, |ui| {
        crate::account::show_summary(&mut app, ui);
    });
    crate::test_support::text_shapes(&output).map(|shape| (shape.galley.text().to_string(), shape.pos)).collect()
}
