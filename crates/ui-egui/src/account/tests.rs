//! Scripted, in-memory account port tests. Nothing here reaches a provider, keychain or network.

use std::sync::{Arc, Mutex};

use egui::Pos2;
use solidcraft_engine::entitlement::{AuthenticatedHostScope, Refusal, RefusalCode};

use super::panel::ChipTone;
use super::*;

pub(crate) struct FakeAccountPort {
    view: AccountView,
    guide: DeleteAccountGuide,
    switch_label: Option<&'static str>,
    switch_guide: Option<&'static str>,
    lifecycle: Option<LifecycleEvent>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl FakeAccountPort {
    pub(crate) fn signed_out(calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self::with_view(AccountView::signed_out(), calls)
    }

    pub(crate) fn with_view(view: AccountView, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            view,
            guide: DeleteAccountGuide { help_url: "https://example.invalid/account/delete", steps: &["Confirm the request."] },
            switch_label: None,
            switch_guide: None,
            lifecycle: None,
            calls,
        }
    }

    fn with_guide(view: AccountView, guide: DeleteAccountGuide, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { view, guide, switch_label: None, switch_guide: None, lifecycle: None, calls }
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
        self.lifecycle = Some(LifecycleEvent::SignedOut);
        Ok(())
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        self.log("switch_account");
        self.lifecycle = Some(LifecycleEvent::AccountSwitched);
        if let Some(label) = self.switch_label {
            self.view.signed_in = true;
            self.view.sign_in_visible = false;
            self.view.account_label = Some(label.to_string());
            self.view.available = Some(7);
            self.view.held = 0;
            self.view.settling = false;
            self.view.chat_available = true;
            self.view.tool_blocked = None;
            if let Some(help_url) = self.switch_guide {
                self.guide = DeleteAccountGuide { help_url, steps: &["account.delete_step_confirm"] };
            }
        } else {
            self.view = AccountView::signed_out();
        }
        Ok(())
    }

    fn delete_account_guide(&self) -> DeleteAccountGuide {
        self.guide
    }

    fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        self.log("begin_device_code");
        Err(Refusal::new(RefusalCode::AuthUnavailable, "not in this fake"))
    }

    fn own_model_view(&self) -> Option<OwnModelView> {
        self.log("own_model_view");
        None
    }

    fn take_lifecycle_event(&mut self) -> Option<LifecycleEvent> {
        self.lifecycle.take()
    }
}

/// A signed-in, unknown-quota port for the shared application-bar regression test.
pub(crate) fn signed_in_unknown_port() -> Box<dyn AccountPort> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    Box::new(FakeAccountPort::with_view(AccountView { available: None, settling: false, ..signed_in_view() }, calls))
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
    state.request(Action::SignIn);
    assert_eq!(state.pending_action(), Some(Action::SignIn));
    state.process_actions();
    assert!(state.view().signed_in);
    let calls = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(calls, vec!["view", "sign_in", "view"]);
}

#[test]
fn rendered_frames_never_call_the_port() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = true;
    harness.app.account =
        State::with_port(Box::new(FakeAccountPort::with_view(AccountView { available: Some(3), ..signed_in_view() }, calls.clone())));
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    harness.frames(3);
    let calls = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert!(calls.is_empty(), "the render path called {calls:?}");
    assert_eq!(harness.app.account.pending_action(), None);
}

#[test]
fn clicking_sign_in_enqueues_and_processes_outside_a_frame() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut harness = crate::scenario::Harness::new();
    harness.app.account = State::with_port(Box::new(FakeAccountPort::signed_out(calls.clone())));
    harness.app.home.open = true;
    harness.frames(2);
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    assert!(crate::scenario::handle_at("account:sign_in").is_some(), "handles: {:?}", crate::scenario::handles(""));
    crate::test_support::click(&mut harness, "account:sign_in");
    assert_eq!(harness.app.account.pending_action(), Some(Action::SignIn));
    harness.frames(1);
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert!(log.is_empty(), "a rendered frame dispatched {log:?}");

    harness.app.account.process_actions();
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(log, vec!["sign_in", "view"]);
    assert!(harness.app.account.view().signed_in);
}

#[test]
fn pending_action_is_visible_as_progress() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(Box::new(FakeAccountPort::signed_out(calls)));
    app.account.request(Action::SignIn);
    let texts = render(&mut app);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("waiting for the account service"), "{texts:?}");
}

#[test]
fn signed_in_shows_available_and_settling_when_held_positive() {
    let view = AccountView { available: Some(999), held: 1, settling: true, ..signed_in_view() };
    assert_eq!(view.available, Some(999));
    assert!(view.settling);
    let texts = render_view(view);
    assert!(texts.iter().any(|(text, _)| text.contains("999")), "{texts:?}");
    assert!(texts.iter().any(|(text, _)| text.to_lowercase().contains("settling")), "{texts:?}");
}

#[test]
fn settling_is_shown_only_when_held_is_positive_and_explained_once() {
    let no_held = AccountView { available: Some(0), held: 0, settling: true, ..signed_in_view() };
    let texts = render_view(no_held);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(!joined.contains("credits settling"), "zero held is not settling: {texts:?}");

    let held = AccountView { available: Some(0), held: 2, settling: false, tool_blocked: Some(RefusalCode::CreditsExhausted), ..signed_in_view() };
    let texts = render_view(held);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("credits settling: 2"), "{texts:?}");
    assert!(!joined.contains("no credits are available for billed tools"), "both explanations: {texts:?}");
}

#[test]
fn zero_credits_still_allows_plain_chat_but_blocks_billable_tools() {
    let view = AccountView { available: Some(0), chat_available: true, tool_blocked: Some(RefusalCode::CreditsExhausted), ..signed_in_view() };
    assert!(view.chat_available);
    assert_eq!(view.tool_blocked, Some(RefusalCode::CreditsExhausted));
    let texts = render_view(view);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("plain chat"), "{texts:?}");
}

#[test]
fn insufficient_credits_explains_and_has_no_retry_or_payment_entry() {
    let view = AccountView { available: Some(0), chat_available: true, tool_blocked: Some(RefusalCode::CreditsExhausted), ..signed_in_view() };
    let texts = render_view(view);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    for forbidden in ["retry", "buy", "purchase", "payment", "upgrade", "add credits"] {
        assert!(!joined.contains(forbidden), "`{forbidden}` appears in {texts:?}");
    }
}

#[test]
fn chat_unavailable_is_explained_even_with_credits() {
    let view = AccountView { available: Some(500), chat_available: false, tool_blocked: None, ..signed_in_view() };
    let texts = render_view(view);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("plain chat needs an online authorized session"), "{texts:?}");
}

#[test]
fn own_model_login_is_shown_separately_from_product_account() {
    let view = AccountView { own_model_label: Some("Anthropic · signed in".into()), ..signed_in_view() };
    let texts = render_view(view);
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
    let texts = render_port(Box::new(UnavailableAccount));
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("not available"), "{texts:?}");
    assert!(!joined.contains("sign in"), "the unavailable build offers a login: {texts:?}");
    assert!(!joined.contains("available credits"), "{texts:?}");
}

#[test]
fn unknown_quota_chip_is_neutral_labelled_text_not_colour_only() {
    let app = crate::test_support::sample_app();
    let view = AccountView { available: None, settling: false, ..signed_in_view() };
    assert_eq!(super::panel::chip_tone(&view), ChipTone::Neutral);
    assert_eq!(super::panel::chip_label(&app, &view).as_deref(), Some("Credits unknown"));
    let texts = render_chip(app, view);
    assert!(texts.iter().any(|text| text == "Credits unknown"), "{texts:?}");
}

#[test]
fn chip_labels_cover_available_zero_and_settling() {
    let app = crate::test_support::sample_app();
    let available = AccountView { available: Some(999), ..signed_in_view() };
    assert_eq!(super::panel::chip_label(&app, &available).as_deref(), Some("999 credits"));
    assert_eq!(super::panel::chip_tone(&available), ChipTone::Available);
    let zero = AccountView { available: Some(0), ..signed_in_view() };
    assert_eq!(super::panel::chip_label(&app, &zero).as_deref(), Some("No credits"));
    assert_eq!(super::panel::chip_tone(&zero), ChipTone::Blocked);
    let settling = AccountView { available: None, held: 2, settling: false, ..signed_in_view() };
    assert_eq!(super::panel::chip_label(&app, &settling).as_deref(), Some("Credits settling"));
    assert_eq!(super::panel::chip_tone(&settling), ChipTone::Neutral);
}

#[test]
fn switching_refreshes_status_and_delete_guidance_without_stale_text() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(signed_in_view(), calls);
    port.switch_label = Some("bob@example.com");
    port.switch_guide = Some("https://example.invalid/account/delete/bob");
    let mut state = State::with_port(Box::new(port));
    state.request(Action::SwitchAccount);
    state.process_actions();
    assert_eq!(state.view().account_label.as_deref(), Some("bob@example.com"));
    assert_eq!(state.delete_account_guide().map(|guide| guide.help_url), Some("https://example.invalid/account/delete/bob"));

    let mut app = crate::test_support::sample_app();
    app.account = state;
    let texts = render(&mut app);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(!joined.contains("alice@example.com"), "the old account text survived: {texts:?}");
    assert!(joined.contains("bob@example.com"), "{texts:?}");
}

struct TestCommitAccount;

impl solidcraft_engine::entitlement::AccountService for TestCommitAccount {
    fn status(&self) -> Result<solidcraft_engine::entitlement::Availability, Refusal> {
        Ok(solidcraft_engine::entitlement::Availability {
            online: true,
            version_supported: true,
            model_available: true,
            balance: 10,
            held: 0,
            reasons: Vec::new(),
        })
    }

    fn authorize(
        &mut self,
        _scope: &AuthenticatedHostScope,
        _capability: solidcraft_engine::entitlement::Capability,
    ) -> Result<solidcraft_engine::entitlement::Grant, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "test authority"))
    }

    fn begin(
        &mut self,
        _scope: &AuthenticatedHostScope,
        _operation: &solidcraft_engine::entitlement::OperationId,
        _capability: solidcraft_engine::entitlement::Capability,
        _entry: solidcraft_engine::entitlement::EntryCategory,
    ) -> Result<solidcraft_engine::entitlement::Admission, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "test authority"))
    }

    fn finalize(
        &mut self,
        _scope: &AuthenticatedHostScope,
        _operation: &solidcraft_engine::entitlement::OperationId,
        _result: solidcraft_engine::entitlement::BillingResult,
    ) -> Result<solidcraft_engine::entitlement::OperationStatus, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "test authority"))
    }

    fn operation(
        &self,
        _scope: &AuthenticatedHostScope,
        _operation: &solidcraft_engine::entitlement::OperationId,
    ) -> Result<solidcraft_engine::entitlement::OperationStatus, Refusal> {
        Ok(solidcraft_engine::entitlement::OperationStatus::Held)
    }
}

struct TestCommitClock;

impl solidcraft_engine::entitlement::MonotonicClock for TestCommitClock {
    fn now(&self) -> solidcraft_engine::entitlement::MonotonicInstant {
        solidcraft_engine::entitlement::MonotonicInstant::from_millis(0)
    }

    fn wall_now(&self) -> std::time::SystemTime {
        std::time::SystemTime::UNIX_EPOCH
    }
}

fn install_test_commit_authority(app: &mut crate::SolveApp) {
    app.install_commit_authority(
        AuthenticatedHostScope::new(
            solidcraft_engine::entitlement::AccountId::new("alice"),
            solidcraft_engine::entitlement::InstallationId::new("installation"),
            solidcraft_engine::entitlement::HostSessionBinding::new("session"),
        ),
        Box::new(TestCommitAccount),
        Box::new(TestCommitClock),
    );
}

#[test]
fn sign_out_and_switch_end_the_old_sessions_authority() {
    for action in [Action::SignOut, Action::SwitchAccount] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut app = crate::test_support::sample_app();
        app.account = State::with_port(Box::new(FakeAccountPort::with_view(signed_in_view(), calls)));
        install_test_commit_authority(&mut app);
        app.account.request(action);
        app.account.process_actions();
        assert!(!app.account.view().signed_in, "the port's new signed-out view is shown");
        app.drain_account_lifecycle();
        assert!(app.commit_scope.is_none(), "{action:?} left the old scope");
        assert!(app.commit_account.is_none(), "{action:?} left the old account status");
        assert!(app.commit_clock.is_none(), "{action:?} left the old clock");
    }
}

#[test]
fn an_ordinary_refresh_keeps_the_current_sessions_authority() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(Box::new(FakeAccountPort::with_view(signed_in_view(), calls)));
    install_test_commit_authority(&mut app);
    app.account.request(Action::RefreshStatus);
    app.account.process_actions();
    app.drain_account_lifecycle();
    assert!(app.commit_scope.is_some(), "a status refresh is not a sign-out");
    assert!(app.commit_account.is_some());
    assert!(app.commit_clock.is_some());
}

#[test]
fn delete_steps_are_localized_through_the_catalog() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let guide = DeleteAccountGuide { help_url: "https://example.invalid/account/delete", steps: &["account.delete_step_confirm"] };
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(Box::new(FakeAccountPort::with_guide(signed_in_view(), guide, calls)));
    let texts = render(&mut app);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(joined.contains("Confirm the deletion and keep the confirmation."), "{texts:?}");
    assert!(!joined.contains("account.delete_step_confirm"), "{texts:?}");
}

fn render(view: &mut crate::SolveApp) -> Vec<(String, Pos2)> {
    let ctx = crate::fonts::test_context();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))), ..Default::default() };
    crate::scenario::clear_handles();
    let output = crate::test_support::frame(&ctx, input, |ui| {
        crate::account::show_summary(view, ui);
    });
    crate::test_support::text_shapes(&output).map(|shape| (shape.galley.text().to_string(), shape.pos)).collect()
}

fn render_view(view: AccountView) -> Vec<(String, Pos2)> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(Box::new(FakeAccountPort::with_view(view, calls)));
    render(&mut app)
}

fn render_port(port: Box<dyn AccountPort>) -> Vec<(String, Pos2)> {
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(port);
    render(&mut app)
}

fn render_chip(mut app: crate::SolveApp, view: AccountView) -> Vec<String> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    app.account = State::with_port(Box::new(FakeAccountPort::with_view(view, calls)));
    let ctx = crate::fonts::test_context();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))), ..Default::default() };
    crate::scenario::clear_handles();
    let output = crate::test_support::frame(&ctx, input, |ui| {
        crate::account::status_chip(&app, ui, egui::Rect::from_min_size(Pos2::new(700.0, 8.0), egui::vec2(90.0, 18.0)));
    });
    crate::test_support::text_shapes(&output).map(|shape| shape.galley.text().to_string()).collect()
}
