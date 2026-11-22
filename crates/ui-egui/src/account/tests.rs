//! Scripted, in-memory account port tests. Nothing here reaches a provider, keychain or network.

use std::sync::atomic::{AtomicBool, Ordering};
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
    startup_waiting: Arc<AtomicBool>,
    refused_renewal: Option<(AuthenticatedHostScope, solidcraft_engine::entitlement::OperationId)>,
    calls: Arc<Mutex<Vec<String>>>,
    browser: Arc<Mutex<Vec<String>>>,
    poll_view: Option<AccountView>,
    cancel_refusal: Option<Refusal>,
    sign_out_refusal: Option<Refusal>,
}

/// A port whose bounded startup check completes right after `view()` takes its snapshot, so a
/// live wake probe after that view would report no work while the cached view is still stale.
struct StartupCompletionPort {
    state: Arc<Mutex<StartupCompletion>>,
    calls: Arc<Mutex<Vec<String>>>,
}

struct StartupCompletion {
    waiting: bool,
    view: AccountView,
    complete_after_view: bool,
}

impl StartupCompletionPort {
    fn new() -> (Self, Arc<Mutex<StartupCompletion>>) {
        let state = Arc::new(Mutex::new(StartupCompletion { waiting: true, view: AccountView::signed_out(), complete_after_view: false }));
        let port = Self { state: Arc::clone(&state), calls: Arc::new(Mutex::new(Vec::new())) };
        (port, state)
    }

    fn log(&self, name: &str) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(name.to_string());
        }
    }
}

impl AccountPort for StartupCompletionPort {
    fn view(&self) -> AccountView {
        self.log("view");
        let mut state = self.state.lock().unwrap();
        let view = state.view.clone();
        if state.complete_after_view {
            state.complete_after_view = false;
            state.waiting = false;
            state.view = signed_in_view();
        }
        view
    }

    fn refresh_status(&mut self) -> Result<AccountView, Refusal> {
        Ok(self.view())
    }

    fn sign_in(&mut self) -> Result<(), Refusal> {
        Ok(())
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        Ok(())
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        Ok(())
    }

    fn delete_account_guide(&self) -> DeleteAccountGuide {
        DeleteAccountGuide { help_url: "", steps: &[] }
    }

    fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        Err(Refusal::new(RefusalCode::AuthRequired, "not in this fake"))
    }

    fn own_model_view(&self) -> Option<OwnModelView> {
        None
    }

    fn startup_waiting(&self) -> bool {
        self.log("startup_waiting");
        self.state.lock().unwrap().waiting
    }
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
            startup_waiting: Arc::new(AtomicBool::new(false)),
            refused_renewal: None,
            calls,
            browser: Arc::new(Mutex::new(Vec::new())),
            poll_view: None,
            cancel_refusal: None,
            sign_out_refusal: None,
        }
    }

    fn with_guide(view: AccountView, guide: DeleteAccountGuide, calls: Arc<Mutex<Vec<String>>>) -> Self {
        Self { guide, ..Self::with_view(view, calls) }
    }

    pub(crate) fn with_refused_renewal(mut self, scope: AuthenticatedHostScope, operation: solidcraft_engine::entitlement::OperationId) -> Self {
        self.refused_renewal = Some((scope, operation));
        self
    }

    fn log(&self, name: &str) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(name.to_string());
        }
    }
}

impl AccountPort for FakeAccountPort {
    fn renewal_refusal(&self, scope: &AuthenticatedHostScope, operation: &solidcraft_engine::entitlement::OperationId) -> Option<RefusalCode> {
        self.refused_renewal.as_ref().filter(|(expected, id)| expected == scope && id == operation).map(|_| RefusalCode::AuthRequired)
    }

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

    fn open_sign_in_browser(&mut self) -> Result<(), Refusal> {
        self.log("open_sign_in_browser");
        let url = self.view.sign_in_prompt.as_ref().map(|prompt| prompt.verification_uri.clone());
        if let Some(url) = url {
            self.browser.lock().unwrap().push(url);
            Ok(())
        } else {
            Err(Refusal::new(RefusalCode::AuthRequired, "no sign-in is waiting for a browser"))
        }
    }

    fn cancel_sign_in(&mut self) -> Result<(), Refusal> {
        self.log("cancel_sign_in");
        if let Some(refusal) = &self.cancel_refusal {
            return Err(refusal.clone());
        }
        self.view = AccountView::signed_out();
        Ok(())
    }

    fn poll(&mut self) -> Option<AccountView> {
        self.poll_view.take()
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        self.log("sign_out");
        self.view = AccountView { sign_out_error: Some(RefusalCode::AuthUnavailable), ..AccountView::signed_out() };
        self.lifecycle = Some(LifecycleEvent::SignedOut);
        if let Some(refusal) = &self.sign_out_refusal {
            return Err(refusal.clone());
        }
        self.view = AccountView::signed_out();
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

    fn startup_waiting(&self) -> bool {
        self.startup_waiting.load(Ordering::SeqCst)
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
            self.change(|state| {
                state.view = AccountView {
                    sign_in_state: SignInState::Failed,
                    sign_in_error: Some(RefusalCode::AuthUnavailable),
                    ..AccountView::signed_out()
                };
            });
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "the test sign-in failed"));
        }
        self.change(|state| {
            state.sign_ins += 1;
            state.view = AccountView { sign_in_state: SignInState::Starting, ..AccountView::signed_out() };
        });
        Ok(())
    }

    fn poll(&mut self) -> Option<AccountView> {
        self.with_state(|state| state.view.sign_in_state == SignInState::Starting && state.challenge.is_some()).filter(|pending| *pending)?;
        self.change(|state| {
            if let Some(code) = &state.challenge {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                if code.expires_at.0 <= now {
                    state.view.sign_in_state = SignInState::Expired;
                } else {
                    state.view.sign_in_state = SignInState::Waiting;
                    state.view.sign_in_prompt = Some(code.clone());
                }
            }
        });
        Some(self.view())
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

    fn open_sign_in_browser(&mut self) -> Result<(), Refusal> {
        self.log("open_sign_in_browser");
        let available = self.with_state(|state| state.view.sign_in_state == SignInState::Waiting).unwrap_or(false);
        if !available {
            return Err(Refusal::new(RefusalCode::AuthRequired, "no sign-in is waiting for browser approval"));
        }
        self.change(|state| state.opens += 1);
        Ok(())
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
        sign_in_state: SignInState::Idle,
        sign_in_prompt: None,
        sign_in_error: None,
        sign_out_pending: false,
        sign_out_recovery: false,
        sign_out_error: None,
        account_label: Some("alice@example.com".into()),
        available: Some(999),
        held: 0,
        settling: false,
        chat_available: true,
        tool_blocked: None,
        own_model_label: None,
    }
}

fn native_click(harness: &mut crate::scenario::Harness, handle: &str) {
    let pos = crate::scenario::handle_at(handle).unwrap_or_else(|| panic!("{handle} is not drawn"));
    let button = egui::PointerButton::Primary;
    harness.frame_with(vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button, pressed: true, modifiers: egui::Modifiers::NONE },
    ]);
    harness.frame_with(vec![egui::Event::PointerButton { pos, button, pressed: false, modifiers: egui::Modifiers::NONE }]);
}

fn frame_input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))), events, ..Default::default() }
}

fn pointer(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

fn platform_key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

#[test]
fn a_native_press_retained_by_a_logic_only_hook_stays_trusted() {
    let mut app = crate::test_support::sample_app();
    let rect = egui::Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(120.0, 30.0));
    let at = rect.center();
    let mut first = frame_input(vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    app.raw_input_hook(&mut first);

    // eframe keeps the post-hook batch for the next logic-only call, then appends new input.
    let mut second = first.clone();
    second.events.push(pointer(at, false));
    app.raw_input_hook(&mut second);
    assert!(
        app.input_trust.pointer_trusted(egui::PointerButton::Primary, rect),
        "a native press retained across a logic-only hook must keep its provenance"
    );
}

#[test]
fn a_synthetic_press_retained_by_a_logic_only_hook_stays_tainted() {
    let mut app = crate::test_support::sample_app();
    let rect = egui::Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(120.0, 30.0));
    let at = rect.center();
    app.synthetic.push(pointer(at, true));
    let mut first = frame_input(Vec::new());
    app.raw_input_hook(&mut first);

    let mut second = first.clone();
    second.events.push(pointer(at, false));
    app.raw_input_hook(&mut second);
    assert!(
        !app.input_trust.pointer_trusted(egui::PointerButton::Primary, rect),
        "a retained synthetic press must not be relabelled as platform input"
    );
}

#[test]
fn a_visible_ui_pass_consumes_retained_taint_before_the_next_native_click() {
    let mut app = crate::test_support::sample_app();
    let rect = egui::Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(120.0, 30.0));
    let at = rect.center();
    app.synthetic.push(pointer(at, true));
    let mut first = frame_input(Vec::new());
    app.raw_input_hook(&mut first);

    // A real UI pass (unlike eframe's logic-only pass) consumes the retained input.
    let ctx = crate::fonts::test_context();
    let mut visible = frame_input(Vec::new());
    app.raw_input_hook(&mut visible);
    let _ = crate::test_support::frame(&ctx, visible, |ui| app.ui(ui));
    let mut old_release = frame_input(vec![pointer(at, false)]);
    app.raw_input_hook(&mut old_release);

    let mut press = frame_input(vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    app.raw_input_hook(&mut press);
    app.input_trust.input_consumed_by_ui();
    let mut release = frame_input(vec![pointer(at, false)]);
    app.raw_input_hook(&mut release);
    assert!(app.input_trust.pointer_trusted(egui::PointerButton::Primary, rect), "a fresh native click after visible consumption must stay trusted");
}

#[test]
fn a_platform_keyboard_activation_is_trusted_only_on_the_focused_control() {
    let mut app = crate::test_support::sample_app();
    let ctx = crate::fonts::test_context();
    let target = egui::Id::new("account:keyboard_target");
    let other = egui::Id::new("account:keyboard_other");
    let rect = egui::Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(120.0, 30.0));
    let other_rect = egui::Rect::from_min_size(Pos2::new(300.0, 100.0), egui::vec2(120.0, 30.0));

    let mut focus = frame_input(Vec::new());
    app.raw_input_hook(&mut focus);
    let _ = crate::test_support::frame(&ctx, focus, |ui| {
        ui.interact(rect, target, egui::Sense::click()).request_focus();
    });

    let mut raw = frame_input(vec![platform_key(egui::Key::Space)]);
    app.raw_input_hook(&mut raw);
    let mut trusted_target = false;
    let mut trusted_other = false;
    let _ = crate::test_support::frame(&ctx, raw, |ui| {
        trusted_target = app.trusted_click(&ui.interact(rect, target, egui::Sense::click()));
        trusted_other = app.trusted_click(&ui.interact(other_rect, other, egui::Sense::click()));
    });
    assert!(trusted_target, "the focused control did not accept its platform keyboard activation");
    assert!(!trusted_other, "another control borrowed the keyboard activation");
}

#[test]
fn a_platform_accesskit_click_is_trusted_only_for_its_target() {
    let mut app = crate::test_support::sample_app();
    let ctx = crate::fonts::test_context();
    ctx.enable_accesskit();
    let target = egui::Id::new("account:accesskit_target");
    let other = egui::Id::new("account:accesskit_other");
    let rect = egui::Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(120.0, 30.0));
    let other_rect = egui::Rect::from_min_size(Pos2::new(300.0, 100.0), egui::vec2(120.0, 30.0));
    let request = |id: egui::Id| {
        egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: id.accesskit_id(),
            data: None,
        })
    };

    let mut raw = frame_input(vec![request(target)]);
    app.raw_input_hook(&mut raw);
    let mut trusted_target = false;
    let mut trusted_other = false;
    let _ = crate::test_support::frame(&ctx, raw, |ui| {
        trusted_target = app.trusted_click(&ui.interact(rect, target, egui::Sense::click()));
        trusted_other = app.trusted_click(&ui.interact(other_rect, other, egui::Sense::click()));
    });
    assert!(trusted_target, "the AccessKit request did not activate its target");
    assert!(!trusted_other, "another control borrowed the AccessKit activation");

    let mut wrong = frame_input(vec![request(other)]);
    app.raw_input_hook(&mut wrong);
    let mut target_with_wrong_request = false;
    let _ = crate::test_support::frame(&ctx, wrong, |ui| {
        target_with_wrong_request = app.trusted_click(&ui.interact(rect, target, egui::Sense::click()));
    });
    assert!(!target_with_wrong_request, "a request for another control authorized this one");
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
    assert_eq!(calls, vec!["view", "view", "sign_in", "view"]);
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
    assert_eq!(log, vec!["view", "sign_in", "view"]);
    assert!(harness.app.account.view().signed_in);
}

#[test]
fn waiting_sign_in_shows_the_code_and_launches_the_browser_only_on_an_explicit_click() {
    let browser = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(
        AccountView {
            sign_in_state: SignInState::Waiting,
            sign_in_prompt: Some(DeviceCode {
                user_code: "BCDF-GHJK".into(),
                verification_uri: "https://auth.example/device".into(),
                expires_at: Timestamp(9_999),
            }),
            ..AccountView::signed_out()
        },
        calls.clone(),
    );
    port.browser = browser.clone();
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = true;
    harness.app.account = State::with_port(Box::new(port));
    harness.frames(2);
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    assert!(browser.lock().unwrap().is_empty(), "rendering a pending sign-in launched a browser");
    assert!(crate::scenario::handle_at("account:sign_in_browser").is_some());
    native_click(&mut harness, "account:sign_in_browser");
    harness.app.account.process_actions();
    assert_eq!(browser.lock().unwrap().as_slice(), ["https://auth.example/device"]);
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(log, vec!["view", "open_sign_in_browser", "view"]);
}

#[test]
fn a_synthetic_control_click_cannot_launch_the_sign_in_browser() {
    let browser = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(
        AccountView {
            sign_in_state: SignInState::Waiting,
            sign_in_prompt: Some(DeviceCode {
                user_code: "BCDF-GHJK".into(),
                verification_uri: "https://auth.example/device".into(),
                expires_at: Timestamp(9_999),
            }),
            ..AccountView::signed_out()
        },
        calls.clone(),
    );
    port.browser = browser.clone();
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = true;
    harness.app.account = State::with_port(Box::new(port));
    harness.frames(2);
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    assert!(crate::scenario::handle_at("account:sign_in_browser").is_some());
    // The control channel's click is app-injected automation, not platform provenance. It must
    // fail closed instead of opening a browser for an agent.
    crate::test_support::click(&mut harness, "account:sign_in_browser");
    harness.app.account.process_actions();
    assert!(browser.lock().unwrap().is_empty(), "automation launched the browser");
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert!(!log.iter().any(|call| call == "open_sign_in_browser"), "automation reached the browser: {log:?}");
}

#[test]
fn expired_sign_in_explains_and_offers_one_explicit_retry() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = true;
    harness.app.account = State::with_port(Box::new(FakeAccountPort::with_view(
        AccountView { sign_in_state: SignInState::Expired, ..AccountView::signed_out() },
        calls.clone(),
    )));
    harness.frames(2);
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    assert!(crate::scenario::handle_at("account:sign_in").is_some(), "retry handle");
    crate::test_support::click(&mut harness, "account:sign_in");
    assert_eq!(harness.app.account.pending_action(), Some(Action::SignIn));
    harness.app.account.process_actions();
    assert!(harness.app.account.view().signed_in, "the explicit retry ran one sign-in");
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(log, vec!["view", "sign_in", "view"]);
}

#[test]
fn a_refused_cancel_is_surfaced_instead_of_claiming_an_action() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(AccountView::signed_out(), calls.clone());
    port.cancel_refusal = Some(Refusal::new(RefusalCode::AuthUnavailable, "the sign-in is completing"));
    let mut state = State::with_port(Box::new(port));
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    state.request(Action::CancelSignIn);
    state.process_actions();
    assert_eq!(state.notice().map(|refusal| refusal.code), Some(RefusalCode::AuthUnavailable));
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(log, vec!["view", "cancel_sign_in", "view"], "a refusal adopts the host's actual view");
}

#[test]
fn a_sign_out_cleanup_failure_adopts_the_hosts_no_authority_view_and_offers_retry() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(signed_in_view(), calls.clone());
    port.sign_out_refusal = Some(Refusal::new(RefusalCode::AuthUnavailable, "the durable sign-out marker failed"));
    let mut state = State::with_port(Box::new(port));
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    state.request(Action::SignOut);
    state.process_actions();
    assert!(!state.view().signed_in, "a signed-in view survived a revoked authority");
    assert_eq!(state.view().sign_out_error, Some(RefusalCode::AuthUnavailable));
    assert_eq!(state.notice().map(|refusal| refusal.code), Some(RefusalCode::AuthUnavailable));
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert_eq!(log, vec!["view", "sign_out", "view"], "the host's actual view was adopted");
}

#[test]
fn sign_out_pending_and_cleanup_failure_are_visible_with_a_retry() {
    let pending = render_view(AccountView { sign_out_pending: true, ..AccountView::signed_out() });
    let joined = pending.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n").to_lowercase();
    assert!(joined.contains("signing out"), "{pending:?}");
    assert!(crate::scenario::handle_at("account:sign_in").is_none(), "pending cleanup must hide sign-in");
    assert!(crate::scenario::handle_at("account:retry_sign_out").is_none(), "cleanup is still running");
    let failed = AccountView { sign_out_error: Some(RefusalCode::AuthUnavailable), ..AccountView::signed_out() };
    let texts = render_view(failed);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(joined.contains("cannot check authorization"), "{texts:?}");
    assert!(crate::scenario::handle_at("account:retry_sign_out").is_some(), "no actionable retry");
    assert!(crate::scenario::handle_at("account:sign_in").is_none(), "failed cleanup must hide sign-in");
}

#[test]
fn inline_waiting_sign_in_offers_the_shared_controls_and_completes_on_a_native_click() {
    let browser = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(
        AccountView {
            sign_in_state: SignInState::Waiting,
            sign_in_prompt: Some(DeviceCode {
                user_code: "BCDF-GHJK".into(),
                verification_uri: "https://auth.example/device".into(),
                expires_at: Timestamp(9_999),
            }),
            ..AccountView::signed_out()
        },
        calls.clone(),
    );
    port.browser = browser.clone();
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(Box::new(port));
    let ctx = crate::fonts::test_context();
    let input = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))),
        events: Vec::new(),
        ..Default::default()
    };
    let mut raw = input();
    app.raw_input_hook(&mut raw);
    crate::scenario::clear_handles();
    let output = crate::test_support::frame(&ctx, raw, |ui| {
        super::show_inline_sign_in(&mut app, ui, "account.sign_in_for_chat", "agent_chat:sign_in");
    });
    let text = crate::test_support::text_shapes(&output).map(|shape| shape.galley.text().to_string()).collect::<Vec<_>>().join("\n");
    assert!(text.contains("BCDF-GHJK"), "the inline prompt lost the code: {text}");
    assert!(text.contains("https://auth.example/device"), "the inline prompt lost the address: {text}");
    assert!(browser.lock().unwrap().is_empty(), "appearing launched a browser");
    assert!(crate::scenario::handle_at("agent_chat:sign_in:browser").is_some(), "no browser action: {:?}", crate::scenario::handles(""));
    assert!(crate::scenario::handle_at("agent_chat:sign_in:cancel").is_some(), "no cancel action");

    let pos = crate::scenario::handle_at("agent_chat:sign_in:browser").unwrap();
    let button = egui::PointerButton::Primary;
    for pressed in [true, false] {
        let mut raw = input();
        raw.events.push(egui::Event::PointerMoved(pos));
        raw.events.push(egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE });
        app.raw_input_hook(&mut raw);
        let _ = crate::test_support::frame(&ctx, raw, |ui| {
            super::show_inline_sign_in(&mut app, ui, "account.sign_in_for_chat", "agent_chat:sign_in");
        });
        app.input_trust.input_consumed_by_ui();
    }
    assert_eq!(app.account.pending_action(), Some(Action::OpenSignInBrowser), "the native click was not accepted");
    app.account.process_actions();
    assert_eq!(browser.lock().unwrap().as_slice(), ["https://auth.example/device"]);
}

fn inline_waiting_app(browser: Arc<Mutex<Vec<String>>>) -> crate::SolveApp {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(
        AccountView {
            sign_in_state: SignInState::Waiting,
            sign_in_prompt: Some(DeviceCode {
                user_code: "BCDF-GHJK".into(),
                verification_uri: "https://auth.example/device".into(),
                expires_at: Timestamp(9_999),
            }),
            ..AccountView::signed_out()
        },
        calls,
    );
    port.browser = browser;
    let mut app = crate::test_support::sample_app();
    app.account = State::with_port(Box::new(port));
    app
}

fn inline_frame(app: &mut crate::SolveApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let mut raw = frame_input(events);
    app.raw_input_hook(&mut raw);
    let output = crate::test_support::frame(ctx, raw, |ui| {
        super::show_inline_sign_in(app, ui, "account.sign_in_for_chat", "agent_chat:sign_in");
    });
    // This direct panel frame is a real UI pass, so model the app pass's consumption claim.
    app.input_trust.input_consumed_by_ui();
    output
}

#[test]
fn the_inline_panel_accepts_a_platform_keyboard_activation_on_its_browser_button() {
    let browser = Arc::new(Mutex::new(Vec::new()));
    let mut app = inline_waiting_app(browser.clone());
    let ctx = crate::fonts::test_context();
    let _ = inline_frame(&mut app, &ctx, Vec::new());
    let _ = inline_frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key { key: egui::Key::Tab, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
    );
    let _ = inline_frame(&mut app, &ctx, vec![platform_key(egui::Key::Space)]);
    assert_eq!(app.account.pending_action(), Some(Action::OpenSignInBrowser));
    assert!(browser.lock().unwrap().is_empty(), "the panel must only enqueue the fake port action");
}

#[test]
fn the_inline_panel_accepts_a_platform_accesskit_activation_for_its_browser_button() {
    let browser = Arc::new(Mutex::new(Vec::new()));
    let mut app = inline_waiting_app(browser.clone());
    let ctx = crate::fonts::test_context();
    ctx.enable_accesskit();
    let output = inline_frame(&mut app, &ctx, Vec::new());
    let label = app.tr("account.sign_in_browser").to_string();
    let node = output
        .platform_output
        .accesskit_update
        .as_ref()
        .and_then(|update| update.nodes.iter().find(|(_, node)| node.label() == Some(label.as_str())).map(|(id, _)| *id))
        .expect("the browser button should be in the accessibility tree");
    let _ = inline_frame(
        &mut app,
        &ctx,
        vec![egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: node,
            data: None,
        })],
    );
    assert_eq!(app.account.pending_action(), Some(Action::OpenSignInBrowser));
    assert!(browser.lock().unwrap().is_empty(), "the panel must only enqueue the fake port action");
}

#[test]
fn inline_expired_or_failed_sign_in_offers_one_explicit_retry() {
    for state in [SignInState::Expired, SignInState::Failed] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut app = crate::test_support::sample_app();
        app.account = State::with_port(Box::new(FakeAccountPort::with_view(
            AccountView { sign_in_state: state, sign_in_error: Some(RefusalCode::AuthRequired), ..AccountView::signed_out() },
            calls,
        )));
        let ctx = crate::fonts::test_context();
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(900.0, 700.0))), ..Default::default() };
        crate::scenario::clear_handles();
        let _ = crate::test_support::frame(&ctx, input, |ui| {
            super::show_inline_sign_in(&mut app, ui, "account.sign_in_for_chat", "agent_chat:sign_in");
        });
        assert!(crate::scenario::handle_at("agent_chat:sign_in").is_some(), "{state:?} lost the retry");
    }
}

#[test]
fn a_readable_startup_tombstone_offers_a_recovery_action_and_unreadable_stays_closed() {
    let recovery = AccountView { sign_out_recovery: true, ..AccountView::unavailable() };
    let texts = render_view(recovery);
    let joined = texts.iter().map(|(text, _)| text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(joined.contains("previous sign-out"), "{texts:?}");
    assert!(crate::scenario::handle_at("account:finish_sign_out").is_some(), "no recovery action");
    assert!(crate::scenario::handle_at("account:sign_in").is_none(), "recovery must not offer sign-in");
    let unreadable = AccountView::unavailable();
    let _ = render_view(unreadable.clone());
    assert!(crate::scenario::handle_at("account:finish_sign_out").is_none());
    assert!(crate::scenario::handle_at("account:sign_in").is_none());
    assert!(!unreadable.sign_out_recovery);
}

#[test]
fn inline_sign_in_is_hidden_during_pending_or_failed_cleanup() {
    for (sign_out_pending, sign_out_error) in [(true, None), (false, Some(RefusalCode::AuthUnavailable))] {
        let mut app = crate::test_support::sample_app();
        let view = AccountView { sign_out_pending, sign_out_error, ..AccountView::signed_out() };
        app.account = State::with_port(Box::new(FakeAccountPort::with_view(view, Arc::new(Mutex::new(Vec::new())))));
        crate::scenario::clear_handles();
        let ctx = crate::fonts::test_context();
        let _ = crate::test_support::frame(&ctx, egui::RawInput::default(), |ui| {
            super::show_inline_sign_in(&mut app, ui, "account.chat_review_sign_in", "account:test_inline_sign_in");
        });
        assert!(crate::scenario::handle_at("account:test_inline_sign_in").is_none());
    }
}

#[test]
fn a_host_poll_update_is_adopted_without_replaying_an_action() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut port = FakeAccountPort::with_view(AccountView::signed_out(), calls.clone());
    port.poll_view = Some(signed_in_view());
    let mut state = State::with_port(Box::new(port));
    if let Ok(mut calls) = calls.lock() {
        calls.clear();
    }
    state.process_actions();
    assert!(state.view().signed_in, "the worker's verified view was shown");
    let log = calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    assert!(log.is_empty(), "a poll is not an explicit action: {log:?}");
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
fn a_700_by_600_window_keeps_the_compact_account_row_on_screen_and_actionable() {
    let flow = FlowPort::signed_out(None);
    let mut harness = crate::scenario::Harness::new();
    harness.app.home.open = false;
    harness.app.account = State::with_port(Box::new(flow));
    harness.call("ui.resize", json!({"width":700.0,"height":600.0}));
    harness.frames(3);
    assert!(crate::scenario::handle_at("titlebar:account_status").is_none(), "the chip needs 900 px");
    crate::test_support::click(&mut harness, "appbar:file");
    let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(700.0, 600.0));
    let label = compact_account_label();
    let at = harness.widget(&label).expect("the compact account row is drawn");
    let row = crate::scenario::rect_of("account:overflow").expect("the drawn row reports its own rect");
    assert!(screen.contains_rect(row), "the account row is clipped at 700x600: {row:?}");
    assert!((at[1] as f32) < 600.0, "the account row's text is below the window: {at:?}");
    harness.step(&json!({"widget": label})).unwrap();
    harness.frames(3);
    let sign_in = crate::scenario::rect_of("account:sign_in").expect("the shared menu offers sign-in");
    assert!(screen.contains_rect(sign_in), "the compact menu's sign-in is clipped: {sign_in:?}");
    crate::test_support::click(&mut harness, "account:sign_in");
    assert_eq!(harness.app.account.pending_action(), Some(Action::SignIn));
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
    assert!(crate::scenario::handle_at("account:sign_in_browser").is_some(), "handles: {:?}", crate::scenario::handles("account:"));
    assert!(!calls.iter().any(|call| call == "open_sign_in_browser"), "a compact frame opened the browser: {calls:?}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0));
    native_click(&mut harness, "account:sign_in_browser");
    assert_eq!(harness.app.account.pending_action(), Some(Action::OpenSignInBrowser));
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "a rendered click opened the browser before processing");
    harness.app.account.process_actions();
    assert_eq!(flow.with_state(|state| state.opens), Some(1));
    assert_eq!(flow.calls().iter().filter(|call| call.as_str() == "open_sign_in_browser").count(), 1);
}

#[test]
fn the_cached_challenge_shows_progress_code_and_expiry_without_a_port_call_in_paint() {
    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    let state = start_flow(&flow);
    assert_eq!(state.view().sign_in_state, SignInState::Waiting);
    assert!(state.sign_in_waiting());
    assert!(state.view().sign_in_prompt.is_some());
    let calls = flow.calls();
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let texts = render(&mut app);
    let text = joined(&texts);
    assert!(text.contains(&crate::i18n::tr("account.sign_in_waiting").to_lowercase()), "{texts:?}");
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
    assert!(crate::scenario::handle_at("account:sign_in_browser").is_some());
    assert!(!calls.iter().any(|call| call == "open_sign_in_browser"), "a frame opened the browser: {calls:?}");
    native_click(&mut harness, "account:sign_in_browser");
    assert_eq!(harness.app.account.pending_action(), Some(Action::OpenSignInBrowser));
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "a rendered click opened the browser before processing");
    harness.app.account.process_actions();
    let calls = flow.calls();
    assert_eq!(flow.with_state(|state| state.opens), Some(1), "{calls:?}");
    assert_eq!(calls.iter().filter(|call| call.as_str() == "open_sign_in_browser").count(), 1, "{calls:?}");
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
    assert_eq!(state.view().sign_in_state, SignInState::Failed);
    assert!(state.view().sign_in_prompt.is_none(), "a failed flow kept its challenge");
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let text = joined(&render(&mut app));
    assert!(text.contains(&crate::i18n::tr("account.sign_in_retry").to_lowercase()), "{text}");
    assert!(!text.contains("bcdf-ghjk"), "a failed flow showed its old code: {text}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0), "a refusal opened a browser");
}

#[test]
fn completion_and_failure_clear_the_old_prompt_and_only_an_explicit_retry_starts_again() {
    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    let mut state = start_flow(&flow);
    assert_eq!(state.view().sign_in_state, SignInState::Waiting);
    flow.change(|state| state.view = signed_in_view());
    state.process_actions();
    assert!(state.view().signed_in);
    assert!(state.view().sign_in_prompt.is_none(), "the completed flow kept a stale prompt");
    assert_eq!(state.view().sign_in_state, SignInState::Idle);
    assert!(!state.sign_in_waiting());
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let text = joined(&render(&mut app));
    assert!(!text.contains("bcdf-ghjk"), "the completed flow kept its old code: {text}");
    assert_eq!(flow.with_state(|state| state.opens), Some(0));

    let flow = FlowPort::signed_out(Some(future_device_code(300)));
    flow.change(|state| state.fail_sign_in = true);
    let mut state = start_flow(&flow);
    assert_eq!(state.view().sign_in_state, SignInState::Failed);
    let count_sign_ins = || flow.calls().iter().filter(|call| call.as_str() == "sign_in").count();
    assert_eq!(count_sign_ins(), 1);
    state.process_actions();
    assert_eq!(count_sign_ins(), 1, "a refusal replayed sign-in by itself");
    flow.change(|state| state.fail_sign_in = false);
    state.request(Action::SignIn);
    state.process_actions();
    state.process_actions();
    assert_eq!(count_sign_ins(), 2, "the explicit retry starts one new flow");
    assert_eq!(state.view().sign_in_state, SignInState::Waiting);
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
    assert_eq!(state.view().sign_in_state, SignInState::Expired);
    assert!(!state.sign_in_waiting(), "an expired code keeps the window waking");
    let mut app = crate::test_support::sample_app();
    app.account = state;
    let text = joined(&render(&mut app));
    assert!(text.contains("sign-in code expired"), "{text}");
    assert!(text.contains(&crate::i18n::tr("account.sign_in_retry").to_lowercase()), "{text}");
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
    assert!(crate::scenario::handle_at("agent_chat:sign_in").is_some(), "handles: {:?}", crate::scenario::handles("agent_chat:"));
    assert!(crate::scenario::handle_at("account:chat_sign_in").is_none(), "AI Chat rendered a duplicate account prompt");
    assert!(harness.widget("Sign in").is_some(), "the product prompt has a visible sign-in action");
}

#[test]
fn the_native_assistant_without_a_web_view_shows_the_product_prompt() {
    use crate::agent_chat::web;

    let mut harness = crate::scenario::Harness::new();
    harness.app.agent_chat.open = true;
    web::set_host(&mut harness.app, false);
    web::set_problem(&mut harness.app, web::Problem::Failed);
    harness.app.account = State::with_port(Box::new(FlowPort::signed_out(None)));
    harness.frames(3);

    assert!(crate::scenario::handle_at("agent_chat:web_retry").is_some(), "the native fallback must be visible");
    assert!(harness.widget("Sign in").is_some(), "the native fallback must offer product sign-in");
    assert!(crate::scenario::handle_at("agent_chat:sign_in").is_some(), "the shared Chat sign-in action must be available");
}

#[test]
fn the_web_assistant_keeps_the_product_prompt_above_the_page() {
    let flow = FlowPort::signed_out(None);
    let mut harness = crate::scenario::Harness::new();
    harness.app.agent_chat.open = true;
    harness.app.agent_chat.web_host = true;
    harness.app.account = State::with_port(Box::new(flow));
    harness.frames(3);
    assert!(crate::scenario::handle_at("agent_chat:sign_in").is_some(), "handles: {:?}", crate::scenario::handles("agent_chat:"));
    assert!(crate::scenario::handle_at("account:chat_sign_in").is_none(), "the web shell rendered a duplicate account prompt");
    let prompt = crate::scenario::rect_of("agent_chat:sign_in");
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

#[test]
fn a_bounded_startup_check_keeps_the_host_waking_until_the_port_finishes() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let waiting = Arc::new(AtomicBool::new(true));
    let mut port = FakeAccountPort::signed_out(calls);
    port.startup_waiting = Arc::clone(&waiting);
    let mut state = State::with_port(Box::new(port));
    assert!(state.startup_waiting(), "a running bounded check asks the host for one more wake");
    waiting.store(false, Ordering::SeqCst);
    state.process_actions();
    assert!(!state.startup_waiting(), "a finished check stops the bounded wake");
}

#[test]
fn a_completion_between_the_view_and_wake_read_still_requests_a_wake() {
    let (port, state) = StartupCompletionPort::new();
    let mut app = State::with_port(Box::new(port));

    // Arm the completion to happen just after the next view snapshot is taken, while the new
    // view it produces is not adopted on this tick.
    state.lock().unwrap().complete_after_view = true;
    app.process_actions();
    assert!(!app.view().signed_in, "the just-taken view is still the cached signed-out snapshot");
    assert!(app.startup_waiting(), "the host must wake once more because work existed before the view");

    app.process_actions();
    assert!(app.view().signed_in, "the next logic tick adopts the completed view");
    assert!(!app.startup_waiting(), "a finished check stops the bounded wake");
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
