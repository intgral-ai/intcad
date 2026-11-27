//! The Intcad product account and available-credit UI.
//!
//! The UI reads an [`AccountView`] snapshot and calls an injected [`AccountPort`] only on an
//! explicit user action. It never opens a keyring, HTTP client or account service, and no port
//! is installed by default, so the home page and title bar stay unchanged until the desktop
//! composition root injects one (Task 11). The browser build stays anonymous and offline: it
//! either has no port or the [`UnavailableAccount`] fallback, which offers no login or credits.

mod panel;

#[cfg(test)]
pub(crate) mod tests;

use solidcraft_engine::entitlement::{AccountService, AuthenticatedHostScope, MonotonicClock, Refusal, RefusalCode};

pub use panel::{PromptContext, show_inline_sign_in, show_product_prompt, show_summary, status_chip};
pub(crate) use panel::{compact_menu_entry, refusal_text, status_chip_width};

/// A point in time for a device-code login, in seconds since the Unix epoch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(pub u64);

/// An explicit account action. Rendering only enqueues it; [`State::process_actions`] calls the
/// port outside a frame. Task 11's runtime must keep networking asynchronous and call the processing
/// step from its own loop, never from a paint pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    RefreshStatus,
    SignIn,
    /// Open the system browser at the verification address the provider just returned. Only a
    /// fresh explicit click may do this; a pending login never launches a browser by itself.
    OpenSignInBrowser,
    CancelSignIn,
    SignOut,
    SwitchAccount,
}

/// Where an explicit desktop sign-in stands. The host fills this from its own worker; rendering
/// only reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SignInState {
    #[default]
    Idle,
    Starting,
    Waiting,
    Expired,
    Failed,
    Recovering,
    PendingVerification,
}

impl SignInState {
    /// A worker is starting or polling: one gesture is still in progress.
    pub fn pending(self) -> bool {
        matches!(self, Self::Starting | Self::Waiting)
    }

    /// A persisted identity is being checked, not an explicit device-code attempt.
    pub fn recovering(self) -> bool {
        matches!(self, Self::Recovering | Self::PendingVerification)
    }
}

/// A host event that ends the authority of the account session that admitted controlled work.
///
/// The app clears commit authority and invalidates queued admissions before a new session can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    SignedOut,
    AccountSwitched,
    SignInRefused,
}

/// The host's one-time handover for a verified session.
pub type CommitAuthority = (AuthenticatedHostScope, Box<dyn AccountService + Send>, Box<dyn MonotonicClock + Send>);

/// Guidance for deleting an account by hand: where to ask and the steps to follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeleteAccountGuide {
    pub help_url: &'static str,
    pub steps: &'static [&'static str],
}

/// A started device-code login, shown by a terminal or the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: Timestamp,
}

/// An own-model (bring-your-own-provider) sign-in, kept apart from the Intcad product account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnModelView {
    pub provider: String,
    pub signed_in: bool,
    pub label: Option<String>,
}

/// What the account UI shows. Only a host-composed [`AccountPort`] makes these values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountView {
    pub signed_in: bool,
    /// Show the explicit sign-in action; false without account support or during unconfirmed cleanup.
    pub sign_in_visible: bool,
    /// Where an explicit desktop sign-in stands. It is idle unless the host started one.
    pub sign_in_state: SignInState,
    /// The provider's code and verification address while a login waits for the person.
    pub sign_in_prompt: Option<DeviceCode>,
    /// The stable reason a sign-in failed, if it did.
    pub sign_in_error: Option<RefusalCode>,
    /// A sign-out's durable marker/delete is still running; authority is already gone in process.
    pub sign_out_pending: bool,
    /// A readable durable sign-out marker from a previous process remains. The UI offers a
    /// recovery action to finish the cleanup; no prior identity or authority is restored.
    pub sign_out_recovery: bool,
    /// The latest sign-out cleanup failed. The process stays fail-closed until a retry confirms it.
    pub sign_out_error: Option<RefusalCode>,
    pub account_label: Option<String>,
    /// Credits a new billed operation may use; `None` when not known.
    pub available: Option<u64>,
    /// Credits held for operations still settling.
    pub held: u64,
    /// Held credits are still settling; say so beside the available amount.
    pub settling: bool,
    /// Plain chat may run (an online, authorized session and an available model).
    pub chat_available: bool,
    /// Why billed tools are refused now, if they are.
    pub tool_blocked: Option<RefusalCode>,
    /// The own-model label, displayed separately from the product account.
    pub own_model_label: Option<String>,
}

impl AccountView {
    /// Signed out: offer the explicit sign-in action and show no quota.
    pub fn signed_out() -> Self {
        Self {
            signed_in: false,
            sign_in_visible: true,
            sign_in_state: SignInState::Idle,
            sign_in_prompt: None,
            sign_in_error: None,
            sign_out_pending: false,
            sign_out_recovery: false,
            sign_out_error: None,
            account_label: None,
            available: None,
            held: 0,
            settling: false,
            chat_available: false,
            tool_blocked: Some(RefusalCode::AuthRequired),
            own_model_label: None,
        }
    }

    /// No account support in this build: no login, no credits.
    pub fn unavailable() -> Self {
        Self {
            signed_in: false,
            sign_in_visible: false,
            sign_in_state: SignInState::Idle,
            sign_in_prompt: None,
            sign_in_error: None,
            sign_out_pending: false,
            sign_out_recovery: false,
            sign_out_error: None,
            account_label: None,
            available: None,
            held: 0,
            settling: false,
            chat_available: false,
            tool_blocked: Some(RefusalCode::AuthUnavailable),
            own_model_label: None,
        }
    }

    /// A new billed operation has no credits right now.
    pub fn no_credits(&self) -> bool {
        self.available == Some(0) || matches!(self.tool_blocked, Some(RefusalCode::CreditsExhausted))
    }

    /// This build offers a product sign-in and the person is not signed in yet. Contextual
    /// product prompts read this instead of matching refusal text from a paid action.
    pub fn product_sign_in_required(&self) -> bool {
        !self.signed_in && self.sign_in_visible && !self.sign_out_pending && !self.sign_out_recovery && self.sign_out_error.is_none()
    }
}

/// The only seam between the account UI and the host.
///
/// Task 11 implements it. Methods are snapshots or explicit actions and must not block the
/// window thread; network and keyring work belongs to the host, never to the render path. No
/// method accepts an account, origin or trust flag from the caller.
pub trait AccountPort: Send + Sync {
    fn control_admitted(
        &mut self,
        _scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
        _operation: &solidcraft_engine::entitlement::OperationId,
        _grant: &solidcraft_engine::entitlement::Grant,
    ) {
    }

    fn control_finished(&mut self, _operation: &solidcraft_engine::entitlement::OperationId) {}

    fn renewed_control_cutoff(
        &self,
        _scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
        _operation: &solidcraft_engine::entitlement::OperationId,
        _grant: &solidcraft_engine::entitlement::Grant,
    ) -> Option<solidcraft_engine::entitlement::MonotonicInstant> {
        None
    }

    fn renewal_refusal(
        &self,
        _scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
        _operation: &solidcraft_engine::entitlement::OperationId,
    ) -> Option<RefusalCode> {
        None
    }

    fn view(&self) -> AccountView;
    fn refresh_status(&mut self) -> Result<AccountView, Refusal>;
    fn sign_in(&mut self) -> Result<(), Refusal>;
    /// Open the browser at the current provider-issued verification address. Default: no browser.
    fn open_sign_in_browser(&mut self) -> Result<(), Refusal> {
        Err(Refusal::new(RefusalCode::AuthRequired, "no sign-in is waiting for a browser"))
    }
    /// Stop the current attempt. A refusal means the sign-in is already completing and could not
    /// be canceled; the caller must show that rather than imply cancellation succeeded.
    fn cancel_sign_in(&mut self) -> Result<(), Refusal> {
        Ok(())
    }
    /// Take a view the host worker produced, if it changed. Default: no worker.
    fn poll(&mut self) -> Option<AccountView> {
        None
    }
    fn sign_out(&mut self) -> Result<(), Refusal>;
    fn switch_account(&mut self) -> Result<(), Refusal>;
    fn delete_account_guide(&self) -> DeleteAccountGuide;
    fn begin_device_code(&self) -> Result<DeviceCode, Refusal>;
    fn own_model_view(&self) -> Option<OwnModelView>;

    /// Take the host event that ends the old session's authority, if one occurred.
    ///
    /// The default is no event, so a port that never signs out needs no extra method. The desktop
    /// composition root reports sign-out, account switch and sign-in refusal here; the app
    /// drains it before controlled dispatch, outside the paint pass.
    fn take_lifecycle_event(&mut self) -> Option<LifecycleEvent> {
        None
    }

    /// Move the verified session's authority to the app once, after polling and lifecycle
    /// handling. This only takes already-prepared state and must not perform I/O.
    fn take_commit_authority(&mut self) -> Option<CommitAuthority> {
        None
    }

    /// A bounded startup check (such as restart verification) is still running, so the host
    /// should wake the window within a bounded interval and read the cached view again. The
    /// default has no startup work.
    fn startup_waiting(&self) -> bool {
        false
    }

    /// Wait for host cleanup that must not be lost on a normal exit. The default has no work.
    fn shutdown(&mut self) {}
}

/// Browser and other no-account fallback: no keyring, HTTP, login action or quota.
pub struct UnavailableAccount;

impl AccountPort for UnavailableAccount {
    fn view(&self) -> AccountView {
        AccountView::unavailable()
    }

    fn refresh_status(&mut self) -> Result<AccountView, Refusal> {
        Ok(AccountView::unavailable())
    }

    fn sign_in(&mut self) -> Result<(), Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "accounts are not available in this build"))
    }

    fn sign_out(&mut self) -> Result<(), Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "accounts are not available in this build"))
    }

    fn switch_account(&mut self) -> Result<(), Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "accounts are not available in this build"))
    }

    fn delete_account_guide(&self) -> DeleteAccountGuide {
        DeleteAccountGuide { help_url: "", steps: &[] }
    }

    fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "accounts are not available in this build"))
    }

    fn own_model_view(&self) -> Option<OwnModelView> {
        None
    }
}

/// The app's account UI state. No port is installed by default.
#[derive(Default)]
pub struct State {
    port: Option<Box<dyn AccountPort>>,
    view: AccountView,
    /// Cached at [`State::with_port`], so no frame asks the host for static guidance text.
    guide: Option<DeleteAccountGuide>,
    /// Explicit actions awaiting [`State::process_actions`] outside a frame.
    pending: Vec<Action>,
    notice: Option<Refusal>,
    /// Sampled before the cached view is adopted on each logic tick, so a bounded startup check
    /// that finishes with the new view still requests one more wake.
    startup_waiting: bool,
}

impl Default for AccountView {
    /// A build with no installed port, including the browser, shows no login and no quota.
    fn default() -> Self {
        Self::unavailable()
    }
}

/// Adopt a successful explicit action's fresh view and guidance, or keep its refusal.
fn apply(
    port: &dyn AccountPort,
    result: Result<(), Refusal>,
    view: &mut AccountView,
    guide: &mut Option<DeleteAccountGuide>,
    notice: &mut Option<Refusal>,
) {
    match result {
        Ok(()) => {
            *view = port.view();
            *guide = Some(port.delete_account_guide());
            *notice = None;
        }
        Err(refusal) => {
            // A refusal may still follow a partial host action: show the host's actual view, not
            // the stale one from before the click.
            *view = port.view();
            *guide = Some(port.delete_account_guide());
            *notice = Some(refusal);
        }
    }
}

impl State {
    /// Install the host's port; this is the composition root's only entry point.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_port(port: Box<dyn AccountPort>) -> Self {
        // Sample before the view snapshot. If the check finishes in between, the stale
        // signed-out view is still followed by a wake that adopts the completed one.
        let startup_waiting = port.startup_waiting();
        let view = port.view();
        let guide = port.delete_account_guide();
        Self { port: Some(port), view, guide: Some(guide), pending: Vec::new(), notice: None, startup_waiting }
    }

    /// The browser stays anonymous and offline even if a desktop composition root is reused.
    #[cfg(target_arch = "wasm32")]
    pub fn with_port(port: Box<dyn AccountPort>) -> Self {
        drop(port);
        Self::default()
    }

    /// Replace the host's port, e.g. after a host restart.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_port(&mut self, port: Box<dyn AccountPort>) {
        // Same order as `with_port`: the waiting sample belongs to the view it precedes.
        self.startup_waiting = port.startup_waiting();
        self.view = port.view();
        self.guide = Some(port.delete_account_guide());
        self.port = Some(port);
        self.pending.clear();
        self.notice = None;
    }

    /// No web build may install an account port, even through a shared host entry point.
    #[cfg(target_arch = "wasm32")]
    pub fn set_port(&mut self, port: Box<dyn AccountPort>) {
        drop(port);
    }

    pub fn is_installed(&self) -> bool {
        self.port.is_some()
    }

    /// The cached snapshot the render path reads; it never calls the port.
    pub fn view(&self) -> &AccountView {
        &self.view
    }

    pub fn notice(&self) -> Option<&Refusal> {
        self.notice.as_ref()
    }

    /// Enqueue an explicit action. Rendering calls this and nothing else; repeated clicks on a
    /// pending action are ignored so one gesture starts one request.
    pub fn request(&mut self, action: Action) {
        if self.port.is_none() || self.pending.contains(&action) {
            return;
        }
        self.pending.push(action);
    }

    /// The action shown as progress; the render path reads this cached value.
    pub fn pending_action(&self) -> Option<Action> {
        self.pending.first().copied()
    }

    /// Run every queued action on the port, outside a frame. Task 11 must make the network work
    /// asynchronous: this call may block the host loop, never the window thread's paint pass.
    ///
    /// The host calls this once per logic tick. It also adopts the port's cached view, so a
    /// completed, failed or retired worker becomes visible without any paint pass calling the
    /// port. A caller schedules a bounded wake while
    /// [`State::sign_in_waiting`] or [`State::startup_waiting`] says a worker is still running.
    pub fn process_actions(&mut self) {
        let actions = std::mem::take(&mut self.pending);
        let Some(port) = self.port.as_mut() else { return };
        // Sample before reading a snapshot. Completion between the two calls still gets a final
        // wake, even if the snapshot has not caught up. Poll before actions so an already-finished
        // login cannot be opened or canceled from the previous frame's prompt.
        self.startup_waiting = port.startup_waiting();
        self.view = port.poll().unwrap_or_else(|| port.view());
        self.guide = Some(port.delete_account_guide());
        for action in actions {
            match action {
                Action::RefreshStatus => match port.refresh_status() {
                    Ok(view) => {
                        self.view = view;
                        self.guide = Some(port.delete_account_guide());
                        self.notice = None;
                    }
                    Err(refusal) => self.notice = Some(refusal),
                },
                Action::SignIn => {
                    let result = port.sign_in();
                    apply(&**port, result, &mut self.view, &mut self.guide, &mut self.notice);
                }
                Action::OpenSignInBrowser => {
                    let result = port.open_sign_in_browser();
                    apply(&**port, result, &mut self.view, &mut self.guide, &mut self.notice);
                }
                Action::CancelSignIn => {
                    let result = port.cancel_sign_in();
                    apply(&**port, result, &mut self.view, &mut self.guide, &mut self.notice);
                }
                Action::SignOut => {
                    let result = port.sign_out();
                    apply(&**port, result, &mut self.view, &mut self.guide, &mut self.notice);
                }
                Action::SwitchAccount => {
                    let result = port.switch_account();
                    apply(&**port, result, &mut self.view, &mut self.guide, &mut self.notice);
                }
            }
        }
    }

    /// A worker is still running, so the host should wake the window within a bounded interval
    /// and read its cached state again. False after success, refusal, cancellation or expiry.
    pub fn sign_in_waiting(&self) -> bool {
        self.view.sign_in_state.pending()
    }

    /// A bounded startup check (restart verification) was still running when the cached view was
    /// sampled, so the host should wake the window once more and read again. No paint pass or
    /// wake starts the check.
    pub fn startup_waiting(&self) -> bool {
        self.startup_waiting
    }

    /// Drain the host's end-of-session event after [`State::process_actions`], never from a
    /// frame. `None` means the current account session still owns any authority it admitted.
    ///
    /// The host reports the event before it finishes deleting stored credentials, so a failed
    /// delete still closes authority. Re-reading the port's view here keeps the panel truthful
    /// even when the action itself surfaced a storage refusal.
    pub fn take_lifecycle_event(&mut self) -> Option<LifecycleEvent> {
        let event = self.port.as_mut()?.take_lifecycle_event()?;
        if let Some(port) = self.port.as_ref() {
            self.view = port.view();
            self.guide = Some(port.delete_account_guide());
        }
        Some(event)
    }

    /// Drain the verified session's authority outside the paint pass.
    pub fn take_commit_authority(&mut self) -> Option<CommitAuthority> {
        self.port.as_mut()?.take_commit_authority()
    }

    /// Wait for the host's account cleanup on a normal exit. Called after authority is closed,
    /// never from a frame.
    pub fn shutdown(&mut self) {
        if let Some(port) = self.port.as_mut() {
            port.shutdown();
        }
    }

    pub(crate) fn renewal_refusal(
        &self,
        scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
        operation: &solidcraft_engine::entitlement::OperationId,
    ) -> Option<Refusal> {
        self.port.as_ref()?.renewal_refusal(scope, operation).map(|code| Refusal::new(code, "the operation's renewal was refused"))
    }

    pub(crate) fn renewed_control_cutoff(
        &self,
        scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
        operation: &solidcraft_engine::entitlement::OperationId,
        grant: &solidcraft_engine::entitlement::Grant,
    ) -> Option<solidcraft_engine::entitlement::MonotonicInstant> {
        self.port.as_ref()?.renewed_control_cutoff(scope, operation, grant)
    }

    pub(crate) fn control_admitted(
        &mut self,
        scope: &solidcraft_engine::entitlement::AuthenticatedHostScope,
        operation: &solidcraft_engine::entitlement::OperationId,
        grant: &solidcraft_engine::entitlement::Grant,
    ) {
        if let Some(port) = self.port.as_mut() {
            port.control_admitted(scope, operation, grant);
        }
    }

    pub(crate) fn control_finished(&mut self, operation: &solidcraft_engine::entitlement::OperationId) {
        if let Some(port) = self.port.as_mut() {
            port.control_finished(operation);
        }
    }

    pub fn delete_account_guide(&self) -> Option<DeleteAccountGuide> {
        self.guide
    }

    pub fn begin_device_code(&self) -> Result<DeviceCode, Refusal> {
        let Some(port) = self.port.as_ref() else {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "no account port is installed"));
        };
        port.begin_device_code()
    }

    pub fn own_model_view(&self) -> Option<OwnModelView> {
        self.port.as_ref().and_then(|port| port.own_model_view())
    }
}
