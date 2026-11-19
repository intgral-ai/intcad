// SPDX-License-Identifier: MPL-2.0
//! Optional synchronous producer policy. No policy preserves other consumers.

pub enum RequestCheck<'a> {
    BrowserCreated { browser_id: i32 },
    BeforeBrowse { browser_id: i32, main_frame: bool, target_url: &'a str, is_redirect: bool },
    Resource { browser_id: i32, target_url: &'a str, is_image: bool },
    Redirect { browser_id: i32, target_url: &'a str, is_image: bool },
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RequestDecision {
    Allow,
    Cancel,
    RenewDocument,
}
pub type RequestPolicy = std::sync::Arc<dyn for<'a> Fn(RequestCheck<'a>) -> RequestDecision + Send + Sync>;

#[cfg(feature = "cef-runtime")]
pub(crate) mod native {
    use super::*;
    use cef::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    pub(crate) fn created(policy: Option<&RequestPolicy>, browser_id: i32) -> bool {
        policy.is_none_or(|policy| policy(RequestCheck::BrowserCreated { browser_id }) == RequestDecision::Allow)
    }

    // Borrow the actual UTF-16 storage while its userfree owner stays alive.
    // Private HTTP URLs are short; data-image URLs retain the existing 3.75 MB
    // picture capacity (base64 plus prefix), not a new small attachment limit.
    fn url(request: &Request, image: bool) -> Option<String> {
        let owner = request.url();
        let raw: Option<&cef::sys::_cef_string_utf16_t> = (&owner).into();
        let borrowed = CefString::from(std::ptr::from_ref(raw?));
        let units = borrowed.as_slice()?;
        let max = if image { 5_000_256 } else { 2_048 };
        if units.len() > max {
            return None;
        }
        let mut text = String::new();
        text.try_reserve(units.len().checked_mul(3)?).ok()?;
        for scalar in char::decode_utf16(units.iter().copied()) {
            text.push(scalar.ok()?);
        }
        Some(text)
    }

    pub(crate) fn before_browse(
        policy: Option<&RequestPolicy>,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        request: Option<&mut Request>,
        is_redirect: i32,
    ) -> i32 {
        let Some(policy) = policy else { return 0 };
        let (Some(browser), Some(frame), Some(request)) = (browser, frame, request) else { return 1 };
        let Some(url) = url(request, false) else { return 1 };
        i32::from(
            policy(RequestCheck::BeforeBrowse {
                browser_id: browser.identifier(),
                main_frame: frame.is_main() != 0,
                target_url: &url,
                is_redirect: is_redirect != 0,
            }) != RequestDecision::Allow,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resource_handler(
        policy: Option<&RequestPolicy>,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        request: Option<&mut Request>,
        is_download: i32,
    ) -> Option<ResourceRequestHandler> {
        let policy = policy?.clone();
        // Returning None for absent CEF identity would allow default handling.
        // Keep an explicit cancelling handler for that request instead.
        let denied = is_download != 0 || browser.is_none() || request.is_none() || frame.as_ref().is_none_or(|frame| frame.is_main() == 0);
        Some(PolicyResourceHandler::new(policy, Arc::new(AtomicBool::new(denied))))
    }

    fn check(policy: &RequestPolicy, browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>) -> bool {
        let (Some(browser), Some(frame), Some(request)) = (browser, frame, request) else { return false };
        if frame.is_main() == 0 {
            return false;
        }
        let image = request.resource_type() == ResourceType::IMAGE;
        let Some(url) = url(request, image) else { return false };
        policy(RequestCheck::Resource { browser_id: browser.identifier(), target_url: &url, is_image: image }) == RequestDecision::Allow
    }

    cef::wrap_resource_request_handler! {
        struct PolicyResourceHandler {
            policy: RequestPolicy,
            denied: Arc<AtomicBool>,
        }
        impl ResourceRequestHandler {
            fn on_before_resource_load(
                &self, browser: Option<&mut Browser>, frame: Option<&mut Frame>,
                request: Option<&mut Request>, _callback: Option<&mut Callback>,
            ) -> ReturnValue {
                if self.denied.load(Ordering::Acquire) || !check(&self.policy, browser, frame, request) {
                    ReturnValue::CANCEL
                } else {
                    ReturnValue::CONTINUE
                }
            }
            fn on_resource_redirect(
                &self, browser: Option<&mut Browser>, frame: Option<&mut Frame>,
                request: Option<&mut Request>, _response: Option<&mut Response>,
                new_url: Option<&mut CefString>,
            ) {
                let allow = match (browser, frame, request, new_url.as_ref()) {
                    (Some(browser), Some(frame), Some(request), Some(url)) if frame.is_main() != 0 => {
                        let units = url.as_slice();
                        let target = units.filter(|s| s.len() <= 2_048).and_then(|s| String::from_utf16(s).ok());
                        target.is_some_and(|target| (self.policy)(RequestCheck::Redirect {
                            browser_id: browser.identifier(), target_url: &target,
                            is_image: request.resource_type() == ResourceType::IMAGE,
                        }) == RequestDecision::Allow)
                    }
                    _ => false,
                };
                if !allow {
                    self.denied.store(true, Ordering::Release);
                    // The callback has no cancellation return. Prevent following
                    // an external target and cancel again at pre-load.
                    if let Some(url) = new_url { *url = "about:blank".into(); }
                }
            }
        }
    }
}
