//! Scripted, in-memory account port tests. Nothing here reaches a provider, keychain or network.

use std::sync::{Arc, Mutex};

use egui::Pos2;
use serde_json::json;
use solidcraft_engine::entitlement::{AuthenticatedHostScope, Refusal, RefusalCode};

use super::panel::{ChipTone, chip_label, chip_tone};
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

/// An explicit device-flow fake with shared state, so a test can complete or fail the worker
/// between logic ticks without another browser, network or credential.
#[derive(Clone)]
pub(crate) struct FlowPort {
    state: Arc<Mutex<FlowAccount>>,
    calls: Arc<Mutex<Vec<String>>>,
}

pub(crate) struct FlowAccount {
    pub(crate) view: AccountView,
    pub(crate) challenge: Option<DeviceCode>,
    pub(crate) notice: Option<Refusal>,
    pub(crate) opens: usize,
    pub(crate) sign_ins: usize,
    pub(crate) fail_sign_in: bool,
    lifecycle: Option<LifecycleEvent>,
}

impl FlowPort {
    pub(crate) fn signed_out(challenge: Option<DeviceCode>) -> Self {
        Self {
            state: Arc::new(Mutex::new(FlowAccount {
                view: AccountView::signed_out(),
                challenge,
                notice: None,
                opens: 0,
                sign_ins: 0,
                fail_sign_in: false,
                lifecycle: None,
            })),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn log(&self, name: &str) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(name.to_string());
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().map(|calls| calls.clone()).unwrap_or_default()
    }

    fn with_state<T>(&self, read: impl FnOnce(&FlowAccount) -> T) -> Option<T> {
        self.state.lock().ok().map(|state| read(&state))
    }

    fn change(&self, edit: impl FnOnce(&mut FlowAccount)) {
        if let Ok(mut state) = self.state.lock() {
            edit(&mut state);
        }
    }
}

impl AccountPort for FlowPort {
    fn view(&self) -> AccountView {
        self.log("view");
        self.with_state(|state| state.view.clone()).unwrap_or_else(AccountView::unavailable)
    }

    fn refresh_status(&mut self) -> Result<AccountView, Refusal> {
        self.log("refresh_status");
        Ok(self.view())
    }

    fn sign_in(&mut self) -> Result<(), Refusal> {
        self.log("sign_in");
        let fail = self.with_state(|state| state.fail_sign_in).unwrap_or(true);
        if fail {
            let refusal = Refusal::new(RefusalCode::AuthUnavailable, "the test sign-in failed");
            self.change(|state| state.notice = Some(refusal));
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the test sign-in failed"));
        }
        self.change(|state| state.sign_ins += 1);
        Ok(())
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        self.log("sign_out");
        self.change(|state| {
            state.view = AccountView::signed_out();
            state.challenge = None;
            state.lifecycle = Some(LifecycleEvent::SignedOut);
        });
        Ok(())
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        self.log("switch_account");
        self.change(|state| {
            state.view = AccountView::signed_out();
            state.challenge = None;
            state.lifecycle = Some(LifecycleEvent::AccountSwitched);
        });
        Ok(())
    }

    fn delete_account_guide(&self) -> DeleteAccountGuide {
        DeleteAccountGuide { help_url: "", steps: &[] }
    }

    fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        self.log("begin_device_code");
        self.with_state(|state| state.challenge.clone())
            .flatten()
            .ok_or_else(|| Refusal::new(RefusalCode::AuthRequired, "the sign-in prompt is not ready"))
    }

    fn open_verification(&mut self) -> Result<(), Refusal> {
        self.log("open_verification");
        let available = self.with_state(|state| state.challenge.is_some()).unwrap_or(false);
        if !available {
            return Err(Refusal::new(RefusalCode::AuthRequired, "no sign-in is waiting for browser approval"));
        }
        self.change(|state| state.opens += 1);
        Ok(())
    }

    fn take_sign_in_notice(&mut self) -> Option<Refusal> {
        self.state.lock().ok().and_then(|mut state| state.notice.take())
    }

    fn own_model_view(&self) -> Option<OwnModelView> {
        None
    }

    fn take_lifecycle_event(&mut self) -> Option<LifecycleEvent> {
        self.state.lock().ok().and_then(|mut state| state.lifecycle.take())
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
    assert_eq!(calls, vec!["view", "sign_in", "view", "view"]);
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
    assert_eq!(log, vec!["sign_in", "view", "view"]);
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
        crate::account::status_chip(&mut app, ui, egui::Rect::from_min_size(Pos2::new(700.0, 8.0), egui::vec2(90.0, 18.0)));
    });
    crate::test_support::text_shapes(&output).map(|shape| shape.galley.text().to_string()).collect()
}

fn future_device_code(after_seconds: u64) -> DeviceCode {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    DeviceCode {
        user_code: "BCDF-GHJK".into(),
        verification_uri: "https://auth.example/device".into(),
        expires_at: Timestamp(now.saturating_add(after_seconds)),
    }
}

fn start_flow(flow: &FlowPort) -> State {
    let mut state = State::with_port(Box::new(flow.clone()));
    state.request(Action::SignIn);
    state.process_actions();
    state
}

fn joined(texts: &[(String, Pos2)]) -> String {
    texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase()
}

#[test]
fn the_permanent_toolbar_entry_says_sign_in_when_signed_out() {
    let app = crate::test_support::sample_app();
    let signed_out = AccountView::signed_out();
    assert_eq!(chip_label(&app, &signed_out).as_deref(), Some("Sign in"));
    assert_eq!(chip_tone(&signed_out), ChipTone::Neutral);
    let texts = render_chip(crate::test_support::sample_app(), signed_out);
    assert!(texts.iter().any(|text| text == "Sign in"), "{texts:?}");
}

#[test]
fn the_shared_toolbar_account_menu_offers_login_and_queues_one_sign_in() {
    let flow = FlowPort::signed_out(None);
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = false;
    harness.app.account = State::with_port(Box::new(flow.clone()));
    harness.frames(3);
    assert!(crate::scenario::handle_at("titlebar:account_status").is_some());
    crate::test_support::click(&mut harness, "titlebar:account_status");
    assert!(harness.widget("Sign in").is_some(), "handles: {:?}", crate::scenario::handles(""));
    assert_eq!(harness.app.account.pending_action(), None);
    crate::test_support::click(&mut harness, "account:sign_in");
    assert_eq!(harness.app.account.pending_action(), Some(Action::SignIn));
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "the menu never opens a browser");
    harness.app.account.process_actions();
    assert_eq!(flow.calls().iter().filter(|call| call.as_str() == "sign_in").count(), 1);
}

fn compact_account_label() -> String {
    format!("{}…", crate::i18n::tr("account.title"))
}

fn open_compact_account_menu(harness: &mut crate::scenario::Harness) {
    crate::test_support::click(harness, "appbar:file");
    assert!(harness.widget(&compact_account_label()).is_some(), "the File menu has the compact account entry: {:?}", crate::scenario::handles(""));
    harness.step(&json!({"widget": compact_account_label()})).unwrap();
    harness.frames(3);
}

#[test]
fn below_900_the_file_menu_offers_the_same_sign_in_without_home() {
    let flow = FlowPort::signed_out(None);
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = false;
    harness.app.account = State::with_port(Box::new(flow.clone()));
    harness.call("ui.resize", json!({"width":700.0,"height":720}));
    harness.frames(3);
    assert!(crate::scenario::handle_at("titlebar:account_status").is_none(), "the chip needs 900 px");
    open_compact_account_menu(&mut harness);
    assert!(crate::scenario::handle_at("account:sign_in").is_some(), "handles: {:?}", crate::scenario::handles("account:"));
    assert_eq!(harness.app.account.pending_action(), None);
    crate::test_support::click(&mut harness, "account:sign_in");
    assert_eq!(harness.app.account.pending_action(), Some(Action::SignIn));
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "the compact menu never opens a browser");
    harness.app.account.process_actions();
    assert_eq!(flow.calls().iter().filter(|call| call.as_str() == "sign_in").count(), 1);
}

#[test]
fn the_normal_width_chip_stays_the_only_account_entry() {
    let flow = FlowPort::signed_out(None);
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = false;
    harness.app.account = State::with_port(Box::new(flow));
    harness.call("ui.resize", json!({"width":1200.0,"height":720}));
    harness.frames(3);
    assert!(crate::scenario::handle_at("titlebar:account_status").is_some());
    crate::test_support::click(&mut harness, "appbar:file");
    assert!(harness.widget(&compact_account_label()).is_none(), "the File menu duplicates the visible chip");
}

#[test]
fn the_compact_menu_keeps_signed_in_sign_out_and_switch() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = false;
    harness.app.account = State::with_port(Box::new(FakeAccountPort::with_view(signed_in_view(), calls)));
    harness.call("ui.resize", json!({"width":700.0,"height":720}));
    harness.frames(3);
    assert!(crate::scenario::handle_at("titlebar:account_status").is_none());
    open_compact_account_menu(&mut harness);
    assert!(crate::scenario::handle_at("account:sign_out").is_some(), "handles: {:?}", crate::scenario::handles("account:"));
    assert!(crate::scenario::handle_at("account:switch_account").is_some());
    crate::test_support::click(&mut harness, "account:sign_out");
    assert_eq!(harness.app.account.pending_action(), Some(Action::SignOut));
}

#[test]
fn unavailable_or_no_port_builds_keep_the_narrow_file_menu_without_an_account_entry() {
    let cases: Vec<State> =
        vec![State::default(), State::with_port(Box::new(FakeAccountPort::with_view(AccountView::unavailable(), Arc::new(Mutex::new(Vec::new())))))];
    for state in cases {
        let mut harness = crate::scenario::Harness::new();
        harness.app.home.open = false;
        harness.app.account = state;
        harness.call("ui.resize", json!({"width":700.0,"height":720}));
        harness.frames(3);
        assert!(crate::scenario::handle_at("titlebar:account_status").is_none());
        crate::test_support::click(&mut harness, "appbar:file");
        assert!(harness.widget(&compact_account_label()).is_none(), "an unavailable build offers an account entry");
        assert!(harness.widget("Sign in").is_none(), "an unavailable build offers a sign-in");
    }
}

#[test]
fn the_compact_challenge_keeps_the_explicit_browser_gesture_and_never_opens_by_itself() {
    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    let state = start_flow(&flow);
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = false;
    harness.app.account = state;
    harness.call("ui.resize", json!({"width":700.0,"height":720}));
    harness.frames(3);
    assert!(crate::scenario::handle_at("titlebar:account_status").is_none());
    open_compact_account_menu(&mut harness);
    let calls = flow.calls();
    assert!(crate::scenario::handle_at("account:open_browser").is_some(), "handles: {:?}", crate::scenario::handles("account:"));
    assert!(!calls.iter().any(|call| call == "open_verification"), "a compact frame opened the browser: {calls:?}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0));
    crate::test_support::click(&mut harness, "account:open_browser");
    assert_eq!(harness.app.account.pending_action(), Some(Action::OpenVerification));
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "a rendered click opened the browser before processing");
    harness.app.account.process_actions();
    assert_eq!(flow.with_state(|state| state.opens), Some(1));
    assert_eq!(flow.calls().iter().filter(|call| call.as_str() == "open_verification").count(), 1);
}

#[test]
fn the_cached_challenge_shows_progress_code_and_expiry_without_a_port_call_in_paint() {
    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    let state = start_flow(&flow);
    assert_eq!(state.sign_in_status(), SignInStatus::Waiting);
    assert!(state.sign_in_waiting());
    assert!(state.challenge().is_some());
    let calls = flow.calls();
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let texts = render(&mut app);
    let text = joined(&texts);
    assert!(text.contains("waiting for approval"), "{texts:?}");
    assert!(text.contains("bcdf-ghjk"), "{texts:?}");
    assert!(text.contains("expires in"), "{texts:?}");
    assert_eq!(flow.calls(), calls, "a paint pass called the account port");
    assert_eq!(flow.with_state(|state| state.opens), Some(0));
}

#[test]
fn one_explicit_browser_click_queues_and_opens_exactly_once_but_rendering_never_opens() {
    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    let state = start_flow(&flow);
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = true;
    harness.app.account = state;
    harness.frames(3);
    let calls = flow.calls();
    assert!(crate::scenario::handle_at("account:open_browser").is_some());
    assert!(!calls.iter().any(|call| call == "open_verification"), "a frame opened the browser: {calls:?}");
    crate::test_support::click(&mut harness, "account:open_browser");
    assert_eq!(harness.app.account.pending_action(), Some(Action::OpenVerification));
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "a rendered click opened the browser before processing");
    harness.app.account.process_actions();
    let calls = flow.calls();
    assert_eq!(flow.with_state(|state| state.opens), Some(1), "{calls:?}");
    assert_eq!(calls.iter().filter(|call| call.as_str() == "open_verification").count(), 1, "{calls:?}");
}

#[test]
fn start_and_refusal_never_open_a_browser_and_a_failure_offers_a_retry() {
    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    let state = start_flow(&flow);
    assert_eq!(flow.with_state(|state| state.opens), Some(0));
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let text = joined(&render(&mut app));
    assert!(text.contains("bcdf-ghjk"), "a started flow shows its challenge: {text}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0));

    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    flow.change(|state| state.fail_sign_in = true);
    let state = start_flow(&flow);
    assert_eq!(state.sign_in_status(), SignInStatus::Failed);
    assert!(state.challenge().is_none(), "a failed flow kept its challenge");
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let text = joined(&render(&mut app));
    assert!(text.contains("try sign-in again"), "{text}");
    assert!(!text.contains("bcdf-ghjk"), "a failed flow showed its old code: {text}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "a refusal opened a browser");
}

#[test]
fn completion_and_failure_clear_the_old_prompt_and_only_an_explicit_retry_starts_again() {
    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    let mut state = start_flow(&flow);
    assert_eq!(state.sign_in_status(), SignInStatus::Waiting);
    flow.change(|state| state.view = signed_in_view());
    state.process_actions();
    assert!(state.view().signed_in);
    assert!(state.challenge().is_none(), "the completed flow kept a stale prompt");
    assert_eq!(state.sign_in_status(), SignInStatus::Idle);
    assert!(!state.sign_in_waiting());
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let text = joined(&render(&mut app));
    assert!(!text.contains("bcdf-ghjk"), "the completed flow kept its old code: {text}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0));

    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    flow.change(|state| state.fail_sign_in = true);
    let mut state = start_flow(&flow);
    assert_eq!(state.sign_in_status(), SignInStatus::Failed);
    let count_sign_ins = || flow.calls().iter().filter(|call| call.as_str() == "sign_in").count();
    assert_eq!(count_sign_ins(), 1);
    state.process_actions();
    assert_eq!(count_sign_ins(), 1, "a refusal replayed sign-in by itself");
    flow.change(|state| state.fail_sign_in = false);
    state.request(Action::SignIn);
    state.process_actions();
    assert_eq!(count_sign_ins(), 2, "the explicit retry starts one new flow");
    assert_eq!(state.sign_in_status(), SignInStatus::Waiting);
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "a retry opened a browser");
}

#[test]
fn an_expired_challenge_hides_the_code_and_offers_a_localized_retry() {
    let flow = FlowPort::signed_out(Some(DeviceCode {
        user_code: "BCDF-GHJK".into(),
        verification_uri: "https://auth.example/device".into(),
        expires_at: Timestamp(0),
    }));
    let state = start_flow(&flow);
    assert!(state.sign_in_expired());
    assert!(!state.sign_in_waiting(), "an expired code keeps the window waking");
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let text = joined(&render(&mut app));
    assert!(text.contains("sign-in code expired"), "{text}");
    assert!(text.contains("try sign-in again"), "{text}");
    assert!(!text.contains("bcdf-ghjk"), "the expired code stayed visible: {text}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0));
}

#[test]
fn the_ai_chat_panel_shows_the_product_prompt() {
    let flow = FlowPort::signed_out(None);
    let mut harness = crate::scenario::Harness::new();
    harness.app.agent_chat.open = true;
    harness.app.account = State::with_port(Box::new(flow));
    harness.frames(3);
    assert!(crate::scenario::handle_at("account:chat_sign_in").is_some(), "handles: {:?}", crate::scenario::handles("account:"));
    assert!(harness.widget("Sign in").is_some(), "the product prompt has a visible sign-in action");
}

#[test]
fn the_web_assistant_keeps_the_product_prompt_above_the_page() {
    let flow = FlowPort::signed_out(None);
    let mut harness = crate::scenario::Harness::new();
    harness.app.agent_chat.open = true;
    harness.app.agent_chat.web_host = true;
    harness.app.account = State::with_port(Box::new(flow));
    harness.frames(3);
    assert!(crate::scenario::handle_at("account:chat_sign_in").is_some(), "handles: {:?}", crate::scenario::handles("account:"));
    let prompt = crate::scenario::rect_of("account:chat_sign_in")
        .or_else(|| crate::scenario::handle_at("account:chat_sign_in").map(|p| egui::Rect::from_center_size(p, egui::Vec2::ZERO)));
    if let (Some(prompt), Some(web)) = (prompt, crate::scenario::rect_of("agent_chat:web")) {
        assert!(!prompt.intersects(web), "the product prompt covers the Assistant page");
    }
}

#[test]
fn ai_chat_and_review_prompts_are_product_account_context_not_model_auth() {
    let mut chat = crate::test_support::sample_app();
    chat.account = State::with_port(Box::new(FlowPort::signed_out(None)));
    let texts = render_prompt(&mut chat, PromptContext::Chat);
    let text = joined(&texts);
    assert!(text.contains("use ai chat"), "{texts:?}");
    assert!(text.contains("model's own sign-in stays separate"), "{texts:?}");
    assert!(text.contains("sign in"), "{texts:?}");

    let mut review = crate::test_support::sample_app();
    review.account = State::with_port(Box::new(FlowPort::signed_out(None)));
    let texts = render_prompt(&mut review, PromptContext::Review);
    let text = joined(&texts);
    assert!(text.contains("run review checks"), "{texts:?}");
    assert!(!text.contains("model's own sign-in"), "{texts:?}");

    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut signed_in = crate::test_support::sample_app();
    signed_in.account = State::with_port(Box::new(FakeAccountPort::with_view(signed_in_view(), calls)));
    let texts = render_prompt(&mut signed_in, PromptContext::Chat);
    assert!(texts.is_empty(), "a signed-in account shows no sign-in prompt: {texts:?}");

    let mut unavailable = crate::test_support::sample_app();
    let calls = Arc::new(Mutex::new(Vec::new()));
    unavailable.account = State::with_port(Box::new(FakeAccountPort::with_view(AccountView::unavailable(), calls)));
    let texts = render_prompt(&mut unavailable, PromptContext::Review);
    assert!(texts.is_empty(), "an unavailable build offers no product sign-in: {texts:?}");
}

fn render_prompt(app: &mut crate::SolveApp, context: PromptContext) -> Vec<(String, Pos2)> {
    let ctx = crate::fonts::test_context();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))), ..Default::default() };
    crate::scenario::clear_handles();
    let output = crate::test_support::frame(&ctx, input, |ui| {
        crate::account::show_product_prompt(app, ui, context);
    });
    crate::test_support::text_shapes(&output).map(|shape| (shape.galley.text().to_string(), shape.pos)).collect()
}
