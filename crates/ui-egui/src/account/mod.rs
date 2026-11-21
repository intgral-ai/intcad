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

use solidcraft_engine::entitlement::{Refusal, RefusalCode};

pub(crate) use panel::status_chip_width;
pub use panel::{show_summary, status_chip};

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
    SignOut,
    SwitchAccount,
}

/// A host event that ends the authority of the account session that admitted controlled work.
///
/// The desktop composition root raises it after an explicit sign-out or account switch. The app
/// then clears commit authority and invalidates queued admissions before a new session can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleEvent {
    SignedOut,
    AccountSwitched,
}

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
    /// Show the explicit sign-in action; false in a build with no account support.
    pub sign_in_visible: bool,
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
}

/// The only seam between the account UI and the host.
///
/// Task 11 implements it. Methods are snapshots or explicit actions and must not block the
/// window thread; network and keyring work belongs to the host, never to the render path. No
/// method accepts an account, origin or trust flag from the caller.
pub trait AccountPort: Send + Sync {
    fn view(&self) -> AccountView;
    fn refresh_status(&mut self) -> Result<AccountView, Refusal>;
    fn sign_in(&mut self) -> Result<(), Refusal>;
    fn sign_out(&mut self) -> Result<(), Refusal>;
    fn switch_account(&mut self) -> Result<(), Refusal>;
    fn delete_account_guide(&self) -> DeleteAccountGuide;
    fn begin_device_code(&self) -> Result<DeviceCode, Refusal>;
    fn own_model_view(&self) -> Option<OwnModelView>;

    /// Open the trusted verification page in the system browser. Only an explicit browser action
    /// calls this; nothing here opens a browser on its own.
    fn open_verification(&mut self) -> Result<(), Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "no browser verification is available"))
    }

    /// Take the latest asynchronous sign-in failure, once, for an in-place prompt.
    fn take_sign_in_notice(&mut self) -> Option<Refusal> {
        None
    }

    /// Take the host event that ends the old session's authority, if one occurred.
    ///
    /// The default is no event, so a port that never signs out needs no extra method. The desktop
    /// composition root reports sign-out and account switch here; the app drains it outside a
    /// frame and closes the old authority before another action runs.
    fn take_lifecycle_event(&mut self) -> Option<LifecycleEvent> {
        None
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
        Err(refusal) => *notice = Some(refusal),
    }
}

impl State {
    /// Install the host's port; this is the composition root's only entry point.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_port(port: Box<dyn AccountPort>) -> Self {
        let view = port.view();
        let guide = port.delete_account_guide();
        Self { port: Some(port), view, guide: Some(guide), pending: Vec::new(), notice: None }
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
    pub fn process_actions(&mut self) {
        let actions = std::mem::take(&mut self.pending);
        let Some(port) = self.port.as_mut() else { return };
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

    /// Wait for the host's account cleanup on a normal exit. Called after authority is closed,
    /// never from a frame.
    pub fn shutdown(&mut self) {
        if let Some(port) = self.port.as_mut() {
            port.shutdown();
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

    /// An explicit browser action. Rendering enqueues a plain click; this call is made outside a
    /// frame, so a slow system opener never runs in a paint pass.
    pub fn open_verification(&mut self) -> Result<(), Refusal> {
        let Some(port) = self.port.as_mut() else {
            return Err(Refusal::new(RefusalCode::AuthUnavailable, "no account port is installed"));
        };
        let result = port.open_verification();
        if let Err(refusal) = &result {
            self.notice = Some(refusal.clone());
        }
        result
    }

    /// Drain a failed asynchronous sign-in into the account notice. Call outside a frame.
    pub fn take_sign_in_notice(&mut self) -> Option<Refusal> {
        let refusal = self.port.as_mut()?.take_sign_in_notice()?;
        self.notice = Some(refusal.clone());
        Some(refusal)
    }

    pub fn own_model_view(&self) -> Option<OwnModelView> {
        self.port.as_ref().and_then(|port| port.own_model_view())
    }
}
