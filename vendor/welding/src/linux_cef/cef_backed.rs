// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

//! CEF handler vtables for the Linux producer: render handler (accelerated
//! OSR paint), life-span, client, load, and display handlers.
//!
//! Split out of `linux_cef/mod.rs`; the producer itself stays there.

use super::*;
use cef::*;

fn push_navigation(events: &WebEventQueue, event: crate::surface::NavigationEvent) {
    events.push(CefSurfaceEvent::Navigation(event));
}

cef::wrap_render_handler! {
    pub(super) struct WeldRenderHandler {
        handler: WeldRenderHandlerInner,
    }

    impl RenderHandler {
        fn view_rect(
            &self,
            _browser: Option<&mut cef::Browser>,
            rect: Option<&mut cef::Rect>,
        ) {
            if let Some(rect) = rect {
                // GetViewRect is answered in DIP, not physical pixels.
                let (w, h) = self.handler.metrics.lock().unwrap().logical();
                log::trace!("view_rect -> {w}x{h} dip");
                rect.width = w;
                rect.height = h;
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut cef::Browser>,
            screen_info: Option<&mut cef::ScreenInfo>,
        ) -> ::std::os::raw::c_int {
            if let Some(info) = screen_info {
                info.device_scale_factor = self.handler.metrics.lock().unwrap().scale();
                return 1;
            }
            0
        }

        fn screen_point(
            &self,
            _browser: Option<&mut cef::Browser>,
            _view_x: ::std::os::raw::c_int,
            _view_y: ::std::os::raw::c_int,
            _screen_x: Option<&mut ::std::os::raw::c_int>,
            _screen_y: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            0
        }

        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut cef::Browser>,
            type_: cef::PaintElementType,
            _dirty_rects: Option<&[cef::Rect]>,
            info: Option<&cef::AcceleratedPaintInfo>,
        ) {
            // CEF paints the popup widget (select dropdowns, autocomplete) as
            // its own element rather than compositing it into the view, so it
            // lands here with PET_POPUP and goes to a separate slot.
            let is_popup = type_ == cef::PaintElementType::POPUP;
            let Some(info) = info else { return };

            let plane_count = info.plane_count as usize;
            log::debug!(
                "on_accelerated_paint: planes={}, format={:?}, modifier=0x{:x}, coded_size={}x{}",
                plane_count,
                info.format,
                info.modifier,
                info.extra.coded_size.width,
                info.extra.coded_size.height,
            );
            if plane_count == 0 || plane_count > info.planes.len() {
                return;
            }

            // SolidCraft: reject signed/empty allocations before any conversion
            // or fd duplication. Full-surface admission is a separate host check.
            let (Ok(width), Ok(height)) = (
                u32::try_from(info.extra.coded_size.width),
                u32::try_from(info.extra.coded_size.height),
            ) else { return };
            if width == 0 || height == 0 {
                return;
            }
            // Obtain the slot before dup so a poisoned lock cannot strand raw
            // descriptors. A callback cannot return Result to CEF: log and stop.
            let slot = if is_popup { &self.handler.popup_slot } else { &self.handler.frame_slot };
            let mut slot = match lock_frame_slot(slot) {
                Ok(slot) => slot,
                Err(error) => {
                    log::error!("failed to lock CEF capture slot: {error}");
                    return;
                }
            };
            let Some(generation) = slot.try_next_generation() else {
                log::error!("CEF capture generation exhausted");
                return;
            };
            let metadata = capture_metadata(
                &info.extra,
                if is_popup { crate::frame_geometry::PaintElement::Popup }
                else { crate::frame_geometry::PaintElement::View },
                generation,
            );

            // Map CEF color type → wgpu sRGB texture format. CEF emits
            // sRGB pixel data; importing as UNORM would double-apply
            // gamma at sample time. See cef#3687 thread.
            let format = match *info.format.as_ref() {
                cef::sys::cef_color_type_t::CEF_COLOR_TYPE_RGBA_8888 => {
                    wgpu::TextureFormat::Rgba8UnormSrgb
                }
                _ => wgpu::TextureFormat::Bgra8UnormSrgb,
            };

            // dup(2) every plane fd. CEF closes the originals after the
            // callback returns; Vulkan will close ours on successful
            // import. On dup failure we close any fds we already duped.
            let mut raw_planes: Vec<(std::os::fd::RawFd, u64, u64, u32)> =
                Vec::with_capacity(plane_count);
            for src in &info.planes[..plane_count] {
                let duped = unsafe { libc::dup(src.fd) };
                if duped < 0 {
                    for (fd, ..) in &raw_planes {
                        unsafe { libc::close(*fd) };
                    }
                    return;
                }
                raw_planes.push((duped, src.offset, src.size, src.stride));
            }

            // SAFETY: every fd in raw_planes is an owned dup made above;
            // the constructor consumes all of them on success or failure.
            let image = unsafe {
                crate::native_frame::DmaBufImage::from_owned_raw_planes(
                    raw_planes,
                    PhysicalSize::new(width, height),
                    format,
                    // drm_format is unused by the Vulkan import path
                    // (which derives vk::Format from wgpu::TextureFormat
                    // directly). Left zeroed for now.
                    0,
                    info.modifier,
                    generation,
                )
            };
            match image {
                Ok(image) => {
                    slot.store(crate::native_frame::NativeFrame::DmaBufImage(
                        image.with_capture_metadata(metadata),
                    ));
                }
                Err(error) => {
                    log::error!("failed to wrap CEF DMABUF image: {error}");
                }
            }
        }

        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut cef::Browser>,
            selected_range: Option<&cef::Range>,
            character_bounds: Option<&[cef::Rect]>,
        ) {
            let rects: Vec<crate::surface::PopupRect> = character_bounds
                .unwrap_or(&[])
                .iter()
                .map(|r| crate::surface::PopupRect {
                    x: r.x,
                    y: r.y,
                    width: r.width.max(0) as u32,
                    height: r.height.max(0) as u32,
                })
                .collect();
            let Some(bounds) = crate::ime::bounds_union(&rects) else { return };
            // CEF reports these in DIP, like popup geometry.
            let bounds = self.handler.metrics.lock().unwrap().rect_to_physical(bounds);
            let (start, end) = selected_range
                .map(|r| (r.from, r.to))
                .unwrap_or((0, 0));
            self.handler.ime.set(crate::surface::ImeComposition {
                bounds,
                selection_start: start,
                selection_end: end,
            });
        }

        fn on_popup_show(&self, _browser: Option<&mut cef::Browser>, show: ::std::os::raw::c_int) {
            let showing = show != 0;
            log::debug!("on_popup_show({showing})");
            self.handler.popup.set_visible(showing);
            if !showing {
                // A hidden popup never paints again; drop the stale surface so
                // acquire_popup cannot hand back a dropdown that is gone.
                // DmaBufImage's owned buffer table closes duped plane fds.
                // SolidCraft: discard keeps the popup capture watermark.
                match lock_frame_slot(&self.handler.popup_slot) {
                    Ok(mut slot) => { slot.discard(); }
                    Err(error) => log::error!("failed to discard CEF popup: {error}"),
                }
            }
        }

        fn on_popup_size(&self, _browser: Option<&mut cef::Browser>, rect: Option<&cef::Rect>) {
            let Some(rect) = rect else { return };
            log::debug!(
                "on_popup_size({}x{} at {},{})",
                rect.width, rect.height, rect.x, rect.y
            );
            self.handler.popup.set_rect(crate::surface::PopupRect {
                x: rect.x,
                y: rect.y,
                width: rect.width.max(0) as u32,
                height: rect.height.max(0) as u32,
            });
        }

        fn start_dragging(
            &self,
            _browser: Option<&mut cef::Browser>,
            drag_data: Option<&mut cef::DragData>,
            allowed_ops: cef::DragOperationsMask,
            x: ::std::os::raw::c_int,
            y: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let Some(drag_data) = drag_data else { return 0 };
            // CEF gives callback-scoped drag data and DIP coordinates. Copy
            // both into welding-owned values before the host starts its native
            // drag loop, which may outlive this callback by arbitrarily long.
            let scale = self.handler.metrics.lock().unwrap().scale();
            push_navigation(&self.handler.events,
                crate::surface::NavigationEvent::DragStarted {
                    payload: crate::drag::payload_from_cef(drag_data),
                    allowed_operations: crate::DragOperations(allowed_ops.as_ref().0 as u32),
                    x: (x as f32 * scale).round() as i32,
                    y: (y as f32 * scale).round() as i32,
                }
            );
            1 // host now owns the system drag loop
        }
    }
}

// SolidCraft: copy only the actual common CEF ABI fields. No callback pointer,
// app placement or synthetic renderer-commit epoch survives this function.
fn capture_metadata(
    extra: &cef::AcceleratedPaintInfoCommon,
    element: crate::frame_geometry::PaintElement,
    generation: u64,
) -> crate::frame_geometry::CaptureMetadata {
    use crate::frame_geometry::{CaptureMetadata, FrameGeometry, FrameRect};

    let rect = |r: &cef::Rect| FrameRect { x: r.x, y: r.y, width: r.width, height: r.height };
    CaptureMetadata {
        element,
        geometry: FrameGeometry {
            coded_size: [extra.coded_size.width, extra.coded_size.height],
            visible_rect: rect(&extra.visible_rect),
            content_rect: rect(&extra.content_rect),
            source_size: (extra.has_source_size != 0).then_some([extra.source_size.width, extra.source_size.height]),
        },
        timestamp_micros: extra.timestamp,
        capture_counter: (extra.has_capture_counter != 0).then_some(extra.capture_counter),
        generation,
    }
}

impl WeldRenderHandler {
    pub fn build(inner: WeldRenderHandlerInner) -> cef::RenderHandler {
        Self::new(inner)
    }
}

cef::wrap_client! {
    pub(super) struct WeldClient {
        render_handler: cef::RenderHandler,
        load_handler: cef::LoadHandler,
        display_handler: cef::DisplayHandler,
        life_span_handler: cef::LifeSpanHandler,
        request_handler: cef::RequestHandler,
        download_handler: cef::DownloadHandler,
        permission_handler: cef::PermissionHandler,
        context_menu_handler: cef::ContextMenuHandler,
        find_handler: cef::FindHandler,
        scripts: Arc<crate::app::PendingScripts>,
        events: Arc<WebEventQueue>,
    }

    impl Client {
        fn render_handler(&self) -> Option<cef::RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn load_handler(&self) -> Option<cef::LoadHandler> {
            Some(self.load_handler.clone())
        }

        fn life_span_handler(&self) -> Option<cef::LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }

        fn request_handler(&self) -> Option<cef::RequestHandler> {
            Some(self.request_handler.clone())
        }

        fn download_handler(&self) -> Option<cef::DownloadHandler> {
            Some(self.download_handler.clone())
        }

        fn permission_handler(&self) -> Option<cef::PermissionHandler> {
            Some(self.permission_handler.clone())
        }

        fn context_menu_handler(&self) -> Option<cef::ContextMenuHandler> {
            Some(self.context_menu_handler.clone())
        }

        fn find_handler(&self) -> Option<cef::FindHandler> {
            Some(self.find_handler.clone())
        }

        fn display_handler(&self) -> Option<cef::DisplayHandler> {
            Some(self.display_handler.clone())
        }

        fn on_process_message_received(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            source_process: cef::ProcessId,
            message: Option<&mut cef::ProcessMessage>,
        ) -> ::std::os::raw::c_int {
            if source_process != cef::ProcessId::RENDERER { return 0; }
            let Some(msg) = message else { return 0 };
            let name = cef::CefString::from(&msg.name()).to_string();
            if name == crate::app::EVAL_RESULT {
                // The renderer answering a request_script_result.
                if let Some(args) = msg.argument_list() {
                    let id = cef::CefString::from(&args.string(0)).to_string();
                    let ok = args.int(1) != 0;
                    let payload = cef::CefString::from(&args.string(2)).to_string();
                    if let Ok(id) = id.parse::<u64>() {
                        self.scripts.complete(
                            WebRequestId::new(id),
                            if ok { Ok(payload) } else { Err(payload) },
                        );
                    }
                }
                return 1;
            }
            if name != "weld.message" { return 0; }
            if let Some(args) = msg.argument_list() {
                let text = cef::CefString::from(&args.string(0)).to_string();
                self.events.push(CefSurfaceEvent::WebMessage(text));
            }
            1
        }
    }
}

impl WeldClient {
    pub fn build(
        render_handler: cef::RenderHandler,
        load_handler: cef::LoadHandler,
        display_handler: cef::DisplayHandler,
        life_span_handler: cef::LifeSpanHandler,
        request_handler: cef::RequestHandler,
        download_handler: cef::DownloadHandler,
        permission_handler: cef::PermissionHandler,
        context_menu_handler: cef::ContextMenuHandler,
        find_handler: cef::FindHandler,
        scripts: Arc<crate::app::PendingScripts>,
        events: Arc<WebEventQueue>,
    ) -> cef::Client {
        Self::new(
            render_handler,
            load_handler,
            display_handler,
            life_span_handler,
            request_handler,
            download_handler,
            permission_handler,
            context_menu_handler,
            find_handler,
            scripts,
            events,
        )
    }
}

// ── Life-span handler: popup policy ───────────────────────────────────────

cef::wrap_life_span_handler! {
    pub(super) struct WeldLifeSpanHandler {
        events: Arc<WebEventQueue>,
        scripts: Arc<crate::app::PendingScripts>,
        cookies: Arc<crate::cookies::CookieJar>,
        policy: Option<crate::request_policy::RequestPolicy>,
        notified: Arc<std::sync::atomic::AtomicBool>,
    }

    impl LifeSpanHandler {
        // SolidCraft: OSR is requested before creation; this is an additional
        // invariant check, not a substitute for the request/popup policy.
        fn on_after_created(&self, browser: Option<&mut cef::Browser>) {
            let Some(browser) = browser else { return };
            let browser_id = browser.identifier();
            let host = browser.host();
            let windowless = host.as_ref().is_some_and(|host| host.is_window_rendering_disabled() != 0);
            let permitted = windowless && crate::request_policy::native::created(self.policy.as_ref(), browser_id);
            if self.policy.is_none() || !self.notified.swap(true, std::sync::atomic::Ordering::AcqRel) {
                push_navigation(&self.events,
                crate::surface::NavigationEvent::BrowserCreated { browser_id, windowless }
                );
            }
            if !permitted && let Some(host) = host {
                host.close_browser(1);
            }
        }

        // Popup browsers are denied and reported. welding renders one surface
        // per producer, so a second browser here would be invisible to the
        // host. The host decides what to do with the URL.
        #[allow(clippy::too_many_arguments)]
        fn on_before_popup(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&cef::CefString>,
            _target_frame_name: Option<&cef::CefString>,
            _target_disposition: cef::WindowOpenDisposition,
            user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&cef::PopupFeatures>,
            _window_info: Option<&mut cef::WindowInfo>,
            _client: Option<&mut Option<cef::Client>>,
            _settings: Option<&mut cef::BrowserSettings>,
            _extra_info: Option<&mut Option<cef::DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let url = if self.policy.is_some() { String::new() } else { target_url.map(|u| u.to_string()).unwrap_or_default() };
            push_navigation(&self.events,
                crate::surface::NavigationEvent::NewWindowRequested {
                    url,
                    user_gesture: user_gesture != 0,
                }
            );
            1 // cancel popup creation
        }

        fn on_before_close(&self, _browser: Option<&mut cef::Browser>) {
            log::debug!("CEF on_before_close");
            self.scripts.fail_all("browser closed");
            self.cookies.fail_active("browser closed");
        }
    }
}

impl WeldLifeSpanHandler {
    pub fn build(
        events: Arc<WebEventQueue>,
        scripts: Arc<crate::app::PendingScripts>,
        cookies: Arc<crate::cookies::CookieJar>,
        policy: Option<crate::request_policy::RequestPolicy>,
    ) -> cef::LifeSpanHandler {
        Self::new(events, scripts, cookies, policy, Arc::new(std::sync::atomic::AtomicBool::new(false)))
    }
}

// SolidCraft: real callback entrypoints, not replacement native types. These
// tests require the vendor's cef-runtime build; direct-source tests cannot run
// them or establish native window creation behavior.
#[cfg(test)]
mod osr_boundary_tests {
    use super::*;

    #[test]
    fn private_request_callbacks_cancel_missing_context_and_other_tab_navigation() {
        let events = Arc::new(WebEventQueue::default());
        let policy: crate::request_policy::RequestPolicy = Arc::new(|_| crate::request_policy::RequestDecision::Allow);
        let handler = WeldRequestHandler::build(
            events.clone(),
            Arc::new(crate::auth::AuthChallenges::default()),
            Arc::new(crate::app::PendingScripts::new(events)),
            Some(policy),
        );
        assert_eq!(handler.on_before_browse(None, None, None, 0, 0), 1);
        assert_eq!(handler.on_open_urlfrom_tab(None, None, None, cef::WindowOpenDisposition::NEW_FOREGROUND_TAB, 1), 1);
        let resource = handler.resource_request_handler(None, None, None, 0, 0, None, None).unwrap();
        assert_eq!(resource.on_before_resource_load(None, None, None, None), cef::ReturnValue::CANCEL);
    }

    // SolidCraft: exercise the real CEF binding, including valid zero timestamp
    // and optional zeros, rather than inventing an ABI-shaped substitute.
    #[test]
    fn capture_scalars_respect_cef_validity_flags_and_are_owned() {
        use crate::frame_geometry::{FrameRect, PaintElement};

        let mut extra = cef::AcceleratedPaintInfoCommon {
            timestamp: 0,
            coded_size: cef::Size { width: 463, height: 1123 },
            visible_rect: cef::Rect { x: 0, y: 0, width: 463, height: 1123 },
            content_rect: cef::Rect { x: 326, y: 0, width: 248, height: 600 },
            source_size: cef::Size { width: 463, height: 1122 },
            capture_counter: 81,
            ..Default::default()
        };
        let absent = capture_metadata(&extra, PaintElement::View, 7);
        assert_eq!(absent.timestamp_micros, 0);
        assert_eq!(absent.geometry.source_size, None);
        assert_eq!(absent.capture_counter, None);
        assert_eq!(absent.geometry.coded_size, [463, 1123]);
        assert_eq!(absent.geometry.visible_rect, FrameRect { x: 0, y: 0, width: 463, height: 1123 });
        assert_eq!(absent.geometry.content_rect, FrameRect { x: 326, y: 0, width: 248, height: 600 });
        assert_eq!(absent.element, PaintElement::View);
        assert_eq!(absent.generation, 7);

        extra.has_source_size = 1;
        extra.has_capture_counter = 1;
        extra.capture_counter = 0;
        extra.timestamp = 123;
        let present = capture_metadata(&extra, PaintElement::Popup, 8);
        extra.source_size.width = -1;
        extra.timestamp = 999;
        drop(extra);
        assert_eq!(present.geometry.source_size, Some([463, 1122]));
        assert_eq!(present.capture_counter, Some(0));
        assert_eq!(present.timestamp_micros, 123);
        assert_eq!(present.element, PaintElement::Popup);
        assert_eq!(present.generation, 8);
    }

    #[test]
    fn window_open_and_target_blank_are_cancelled_and_reported() {
        let events = Arc::new(WebEventQueue::default());
        let handler = WeldLifeSpanHandler::build(
            events.clone(),
            Arc::new(crate::app::PendingScripts::new(events.clone())),
            Arc::new(crate::cookies::CookieJar::new(events.clone())),
            None,
        );
        for disposition in [
            cef::WindowOpenDisposition::NEW_POPUP,
            cef::WindowOpenDisposition::NEW_FOREGROUND_TAB,
        ] {
            let url: cef::CefString = "https://example.invalid/private".into();
            assert_eq!(
                handler.on_before_popup(
                    None,
                    None,
                    1,
                    Some(&url),
                    None,
                    disposition,
                    1,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None
                ),
                1
            );
            assert!(matches!(
                events.poll(),
                Some(CefSurfaceEvent::Navigation(
                    crate::surface::NavigationEvent::NewWindowRequested {
                        user_gesture: true,
                        ..
                    }
                ))
            ));
        }
        assert!(events.poll().is_none());
    }

    #[test]
    fn pet_popup_remains_an_osr_widget_not_a_browser_request() {
        let events = Arc::new(WebEventQueue::default());
        let popup = Arc::new(crate::popup::PopupState::default());
        let handler = WeldRenderHandler::build(WeldRenderHandlerInner {
            frame_slot: Arc::new(Mutex::new(PendingFrameSlot::default())),
            popup_slot: Arc::new(Mutex::new(PendingFrameSlot::default())),
            popup: popup.clone(),
            cursor: Arc::new(crate::cursor::LatestCursor::default()),
            ime: Arc::new(crate::ime::LatestComposition::default()),
            events: events.clone(),
            metrics: Arc::new(Mutex::new(crate::view::ViewMetrics::new(
                PhysicalSize::new(900, 600),
                1.0,
            ))),
        });
        handler.on_popup_size(
            None,
            Some(&cef::Rect {
                x: 10,
                y: 20,
                width: 100,
                height: 50,
            }),
        );
        handler.on_popup_show(None, 1);
        handler.on_accelerated_paint(None, cef::PaintElementType::POPUP, None, None);
        assert_eq!(
            popup.rect_if_visible(),
            Some(crate::surface::PopupRect {
                x: 10,
                y: 20,
                width: 100,
                height: 50
            })
        );
        assert!(events.poll().is_none());
        handler.on_popup_show(None, 0);
        assert_eq!(popup.rect_if_visible(), None);
    }
}

// ── Request handler ───────────────────────────────────────────────────────

cef::wrap_request_handler! {
    pub(super) struct WeldRequestHandler {
        events: Arc<WebEventQueue>,
        auth: Arc<crate::auth::AuthChallenges>,
        scripts: Arc<crate::app::PendingScripts>,
        policy: Option<crate::request_policy::RequestPolicy>,
    }

    impl RequestHandler {
        fn on_open_urlfrom_tab(
            &self, _browser: Option<&mut cef::Browser>, _frame: Option<&mut cef::Frame>,
            _target_url: Option<&cef::CefString>, _target_disposition: cef::WindowOpenDisposition,
            _user_gesture: i32,
        ) -> i32 {
            i32::from(self.policy.is_some())
        }
        fn on_before_browse(
            &self, browser: Option<&mut cef::Browser>, frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>, _user_gesture: i32, is_redirect: i32,
        ) -> i32 {
            crate::request_policy::native::before_browse(self.policy.as_ref(), browser, frame, request, is_redirect)
        }
        fn resource_request_handler(
            &self, browser: Option<&mut cef::Browser>, frame: Option<&mut cef::Frame>,
            request: Option<&mut cef::Request>, _is_navigation: i32, is_download: i32,
            _request_initiator: Option<&cef::CefString>, _disable_default_handling: Option<&mut i32>,
        ) -> Option<cef::ResourceRequestHandler> {
            crate::request_policy::native::resource_handler(self.policy.as_ref(), browser, frame, request, is_download)
        }
        fn auth_credentials(
            &self,
            _browser: Option<&mut cef::Browser>,
            origin_url: Option<&cef::CefString>,
            is_proxy: ::std::os::raw::c_int,
            host: Option<&cef::CefString>,
            port: ::std::os::raw::c_int,
            realm: Option<&cef::CefString>,
            scheme: Option<&cef::CefString>,
            callback: Option<&mut cef::AuthCallback>,
        ) -> ::std::os::raw::c_int {
            if self.policy.is_some() {
                if let Some(callback) = callback { callback.cancel(); }
                return 1;
            }
            let id = self.auth.next_id();
            let event = crate::surface::NavigationEvent::AuthChallenged {
                id,
                origin_url: origin_url.map(|s| s.to_string()).unwrap_or_default(),
                host: host.map(|s| s.to_string()).unwrap_or_default(),
                port: port.clamp(0, u16::MAX as _) as u16,
                realm: realm.map(|s| s.to_string()).unwrap_or_default(),
                scheme: scheme.map(|s| s.to_string()).unwrap_or_default(),
                is_proxy: is_proxy != 0,
            };
            push_navigation(&self.events, event);

            let Some(callback) = callback else { return 0 };
            if self.auth.is_enabled() {
                // Held for the host to answer. CEF's auth callback is
                // reference-counted and may be answered later, unlike the
                // download destination.
                self.auth.hold(id, callback.clone());
            } else {
                // Declined now rather than held: a host that never answers
                // would otherwise keep this request open forever.
                callback.cancel();
            }
            1
        }

        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut cef::Browser>,
            _status: cef::TerminationStatus,
            error_code: ::std::os::raw::c_int,
            error_string: Option<&cef::CefString>,
        ) {
            let status = crate::surface::termination_status(_status);
            let error_string = if self.policy.is_some() { "private renderer terminated".into() }
                else { error_string.map(|s| s.to_string()).unwrap_or_default() };
            log::error!(
                "weld: CEF render process terminated ({status:?}, code {error_code}, {error_string})"
            );
            push_navigation(&self.events,
                crate::surface::NavigationEvent::ContentProcessTerminated {
                    status,
                    error_code,
                    error_string,
                }
            );
            self.scripts.fail_all("renderer process terminated");
        }
    }
}

impl WeldRequestHandler {
    pub fn build(
        events: Arc<WebEventQueue>,
        auth: Arc<crate::auth::AuthChallenges>,
        scripts: Arc<crate::app::PendingScripts>,
        policy: Option<crate::request_policy::RequestPolicy>,
    ) -> cef::RequestHandler {
        Self::new(events, auth, scripts, policy)
    }
}

// ── Load handler ─────────────────────────────────────────────────────────

cef::wrap_load_handler! {
    pub(super) struct WeldLoadHandler {
        inner: WeldRenderHandlerInner,
    }

    impl LoadHandler {
        fn on_load_start(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _transition_type: cef::TransitionType,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            let url = frame.map(|f| cef::CefString::from(&f.url()).to_string()).unwrap_or_default();
            push_navigation(&self.inner.events,
                crate::surface::NavigationEvent::LoadStart { url }
            );
        }

        fn on_load_end(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            http_status_code: ::std::os::raw::c_int,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            let url = frame.map(|f| cef::CefString::from(&f.url()).to_string()).unwrap_or_default();
            push_navigation(&self.inner.events,
                crate::surface::NavigationEvent::LoadEnd {
                    url,
                    http_status: http_status_code,
                }
            );
        }

        fn on_load_error(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            _error_code: cef::Errorcode,
            error_text: Option<&cef::CefString>,
            failed_url: Option<&cef::CefString>,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            let url = failed_url.map(|u| u.to_string()).unwrap_or_default();
            let text = error_text.map(|t| t.to_string()).unwrap_or_default();
            // Safety: Errorcode wraps cef_errorcode_t which wraps c_int;
            // both are #[repr(transparent)] around a 4-byte integer.
            let code: i32 = unsafe { std::mem::transmute(_error_code) };
            push_navigation(&self.inner.events,
                crate::surface::NavigationEvent::LoadError {
                    url,
                    error_code: code,
                    error_text: text,
                }
            );
        }
    }
}

impl WeldLoadHandler {
    pub fn build(inner: WeldRenderHandlerInner) -> cef::LoadHandler {
        Self::new(inner)
    }
}

// ── Display handler ───────────────────────────────────────────────────────

cef::wrap_display_handler! {
    pub(super) struct WeldDisplayHandler {
        inner: WeldRenderHandlerInner,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            frame: Option<&mut cef::Frame>,
            url: Option<&cef::CefString>,
        ) {
            let is_main = frame.as_ref().map(|f| f.is_main() != 0).unwrap_or(false);
            if !is_main { return; }
            let url = url.map(|u| u.to_string()).unwrap_or_default();
            push_navigation(&self.inner.events,
                crate::surface::NavigationEvent::AddressChanged { url }
            );
        }

        // CEF puts OnCursorChange on the display handler, not the render
        // handler, and types the handle differently per platform (::std::os::raw::c_ulong here).
        fn on_cursor_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            _cursor: ::std::os::raw::c_ulong,
            type_: cef::CursorType,
            _custom_cursor_info: Option<&cef::CursorInfo>,
        ) -> ::std::os::raw::c_int {
            self.inner.cursor.set(crate::cursor::from_cef(type_));
            1 // handled: under OSR the host owns the pointer
        }

        fn on_console_message(
            &self,
            _browser: Option<&mut cef::Browser>,
            level: cef::LogSeverity,
            message: Option<&cef::CefString>,
            source: Option<&cef::CefString>,
            line: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            push_navigation(&self.inner.events,
                crate::surface::NavigationEvent::ConsoleMessage {
                    // cef_log_severity_t is repr(u32) here and repr(i32) on
                    // Windows, so go through the reference rather than From.
                    level: *level.as_ref() as i32,
                    message: message.map(|m| m.to_string()).unwrap_or_default(),
                    source: source.map(|s| s.to_string()).unwrap_or_default(),
                    line,
                }
            );
            0 // let CEF log it as well
        }

        fn on_title_change(
            &self,
            _browser: Option<&mut cef::Browser>,
            title: Option<&cef::CefString>,
        ) {
            let title = title.map(|t| t.to_string()).unwrap_or_default();
            push_navigation(&self.inner.events,
                crate::surface::NavigationEvent::TitleChanged { title }
            );
        }
    }
}

impl WeldDisplayHandler {
    pub fn build(inner: WeldRenderHandlerInner) -> cef::DisplayHandler {
        Self::new(inner)
    }
}

// ── Download handler ──────────────────────────────────────────────────────

cef::wrap_download_handler! {
    pub(super) struct WeldDownloadHandler {
        events: Arc<WebEventQueue>,
        downloads: Arc<crate::downloads::Downloads>,
    }

    impl DownloadHandler {
        fn can_download(
            &self,
            _browser: Option<&mut cef::Browser>,
            _url: Option<&cef::CefString>,
            _request_method: Option<&cef::CefString>,
        ) -> ::std::os::raw::c_int {
            // Refuse before anything is created when there is nowhere to put it,
            // rather than starting a transfer and cancelling it later.
            self.downloads.is_enabled() as _
        }

        fn on_before_download(
            &self,
            _browser: Option<&mut cef::Browser>,
            download_item: Option<&mut cef::DownloadItem>,
            suggested_name: Option<&cef::CefString>,
            callback: Option<&mut cef::BeforeDownloadCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(item), Some(callback)) = (download_item, callback) else {
                return 0;
            };
            let suggested = suggested_name.map(|s| s.to_string()).unwrap_or_default();
            let Some(destination) = self.downloads.destination_for(&suggested) else {
                // Answering with nothing cancels it, which is what a host
                // without a download directory asked for.
                return 0;
            };
            let id = item.id();
            let url = cef_string(item.url());
            let total = item.total_bytes();
            self.downloads.mark_started(id);
            push_navigation(&self.events,
                crate::surface::NavigationEvent::DownloadStarted {
                    id,
                    url,
                    suggested_filename: suggested,
                    destination_path: destination.clone(),
                    total_bytes_expected: (total > 0).then_some(total as u64),
                }
            );
            let path: cef::CefString = destination.to_string_lossy().as_ref().into();
            // false: no system save dialog. The destination is already decided,
            // and a machine nobody is sitting at cannot answer one.
            callback.cont(Some(&path), 0);
            1
        }

        fn on_download_updated(
            &self,
            _browser: Option<&mut cef::Browser>,
            download_item: Option<&mut cef::DownloadItem>,
            callback: Option<&mut cef::DownloadItemCallback>,
        ) {
            let Some(item) = download_item else { return };
            let id = item.id();
            // CEF updates an item before it asks where to put it. Staying quiet
            // until the download has been accepted keeps DownloadStarted first.
            if !self.downloads.has_started(id) {
                return;
            }
            let path = std::path::PathBuf::from(cef_string(item.full_path()));

            // The host's cancel/pause/resume lands here: this callback is the
            // only place CEF offers one, and only for the length of this call.
            if let (Some(callback), Some(op)) = (callback, self.downloads.take_pending(id)) {
                match op {
                    crate::downloads::DownloadOp::Cancel => callback.cancel(),
                    crate::downloads::DownloadOp::Pause => callback.pause(),
                    crate::downloads::DownloadOp::Resume => callback.resume(),
                }
            }

            let total = item.total_bytes();
            let total_bytes_expected = (total > 0).then_some(total as u64);
            let received = item.received_bytes().max(0) as u64;
            let complete = item.is_complete() != 0;
            let canceled = item.is_canceled() != 0;

            if complete || canceled || self.downloads.due_for_progress(id, std::time::Instant::now()) {
                push_navigation(&self.events, crate::surface::NavigationEvent::DownloadProgress {
                    id,
                    bytes_received: received,
                    total_bytes_expected,
                });
            }
            if canceled {
                push_navigation(&self.events, crate::surface::NavigationEvent::DownloadCancelled {
                    id,
                    destination_path: path,
                });
            } else if complete {
                let interrupted = item.is_interrupted() != 0;
                push_navigation(&self.events, crate::surface::NavigationEvent::DownloadFinished {
                    id,
                    destination_path: path,
                    error: interrupted.then(|| {
                        format!("interrupted: {:?}", *item.interrupt_reason().as_ref() as i32)
                    }),
                });
            }
            if complete || canceled {
                self.downloads.forget(id);
            }
        }
    }
}

/// CEF hands strings back as `CefStringUserfree`, which is empty when unset.
fn cef_string(s: cef::CefStringUserfree) -> String {
    let raw: Option<&cef::sys::_cef_string_utf16_t> = (&s).into();
    raw.map(|r| cef::CefStringUtf16::from(*r).to_string())
        .unwrap_or_default()
}

impl WeldDownloadHandler {
    pub fn build(
        events: Arc<WebEventQueue>,
        downloads: Arc<crate::downloads::Downloads>,
    ) -> cef::DownloadHandler {
        Self::new(events, downloads)
    }
}

// ── Permission handler ────────────────────────────────────────────────────

cef::wrap_permission_handler! {
    pub(super) struct WeldPermissionHandler {
        events: Arc<WebEventQueue>,
        permissions: Arc<crate::permissions::Permissions>,
    }

    impl PermissionHandler {
        fn on_show_permission_prompt(
            &self,
            _browser: Option<&mut cef::Browser>,
            _prompt_id: u64,
            requesting_origin: Option<&cef::CefString>,
            requested_permissions: u32,
            callback: Option<&mut cef::PermissionPromptCallback>,
        ) -> ::std::os::raw::c_int {
            let id = self.permissions.next_id();
            push_navigation(&self.events,
                crate::surface::NavigationEvent::PermissionRequested {
                    id,
                    origin: requesting_origin.map(|s| s.to_string()).unwrap_or_default(),
                    permissions: crate::permissions::decode(requested_permissions),
                    raw: requested_permissions,
                }
            );
            let Some(callback) = callback else { return 0 };
            if self.permissions.is_enabled() {
                self.permissions.hold(id, crate::permissions::Pending::Prompt(callback.clone()));
            } else {
                // Denied now rather than held: an unanswered prompt leaves the
                // page waiting forever.
                callback.cont(cef::PermissionRequestResult::DENY);
            }
            1
        }

        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            requesting_origin: Option<&cef::CefString>,
            requested_permissions: u32,
            callback: Option<&mut cef::MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            let id = self.permissions.next_id();
            push_navigation(&self.events,
                crate::surface::NavigationEvent::PermissionRequested {
                    id,
                    origin: requesting_origin.map(|s| s.to_string()).unwrap_or_default(),
                    // Media capture arrives on its own callback with its own
                    // bits, so it is decoded from the media set rather than the
                    // prompt set.
                    permissions: crate::permissions::decode_media(requested_permissions),
                    raw: requested_permissions,
                }
            );
            let Some(callback) = callback else { return 0 };
            if self.permissions.is_enabled() {
                self.permissions.hold(
                    id,
                    crate::permissions::Pending::Media(callback.clone(), requested_permissions),
                );
            } else {
                callback.cont(0);
            }
            1
        }
    }
}

impl WeldPermissionHandler {
    pub fn build(
        events: Arc<WebEventQueue>,
        permissions: Arc<crate::permissions::Permissions>,
    ) -> cef::PermissionHandler {
        Self::new(events, permissions)
    }
}

// ── Context menu handler ──────────────────────────────────────────────────

cef::wrap_context_menu_handler! {
    pub(super) struct WeldContextMenuHandler {
        events: Arc<WebEventQueue>,
        metrics: Arc<Mutex<crate::view::ViewMetrics>>,
    }

    impl ContextMenuHandler {
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut cef::ContextMenuParams>,
            model: Option<&mut cef::MenuModel>,
        ) {
            if let Some(params) = params {
                // CEF reports DIP; every coordinate this API hands out or takes
                // is physical, so convert on the way out.
                let scale = self.metrics.lock().unwrap().scale();
                let flags = params.type_flags().as_ref().0 as u32;
                push_navigation(&self.events,
                    crate::surface::NavigationEvent::ContextMenuRequested {
                        x: (params.xcoord() as f32 * scale).round() as i32,
                        y: (params.ycoord() as f32 * scale).round() as i32,
                        targets: crate::surface::context_menu_targets(flags),
                        link_url: cef_string(params.link_url()),
                        source_url: cef_string(params.source_url()),
                        page_url: cef_string(params.page_url()),
                        selection_text: cef_string(params.selection_text()),
                    }
                );
            }
            // Empty the menu. CEF's own has nowhere to draw itself under
            // windowless rendering, so leaving it populated only invites CEF to
            // try; the host draws its own from the event above.
            if let Some(model) = model {
                model.clear();
            }
        }

        fn run_context_menu(
            &self,
            _browser: Option<&mut cef::Browser>,
            _frame: Option<&mut cef::Frame>,
            _params: Option<&mut cef::ContextMenuParams>,
            _model: Option<&mut cef::MenuModel>,
            callback: Option<&mut cef::RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            // Claim it and dismiss immediately: the event has already gone out.
            if let Some(callback) = callback {
                callback.cancel();
            }
            1
        }
    }
}

impl WeldContextMenuHandler {
    pub fn build(
        events: Arc<WebEventQueue>,
        metrics: Arc<Mutex<crate::view::ViewMetrics>>,
    ) -> cef::ContextMenuHandler {
        Self::new(events, metrics)
    }
}

// ── DevTools protocol observer ────────────────────────────────────────────

cef::wrap_dev_tools_message_observer! {
    pub(super) struct WeldDevToolsObserver {
        channel: Arc<crate::devtools::DevToolsChannel>,
        snapshots: Arc<crate::snapshot::SnapshotChannel>,
    }

    impl DevToolsMessageObserver {
        fn on_dev_tools_message(
            &self,
            _browser: Option<&mut cef::Browser>,
            message: Option<&[u8]>,
        ) -> ::std::os::raw::c_int {
            // The raw wire format, results and events alike. Taking it here
            // rather than from the parsed callbacks keeps what a host sees
            // identical to what the protocol documents.
            if self.channel.is_enabled()
                && let Some(bytes) = message
            {
                self.channel.push(String::from_utf8_lossy(bytes).into_owned());
            }
            // 0: not consumed, so CEF still runs its own parsed callbacks for
            // anything else that wants them.
            0
        }

        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut cef::Browser>,
            message_id: ::std::os::raw::c_int,
            success: ::std::os::raw::c_int,
            result: Option<&[u8]>,
        ) {
            self.snapshots.complete(message_id, success != 0, result);
        }
    }
}

impl WeldDevToolsObserver {
    pub fn build(
        channel: Arc<crate::devtools::DevToolsChannel>,
        snapshots: Arc<crate::snapshot::SnapshotChannel>,
    ) -> cef::DevToolsMessageObserver {
        Self::new(channel, snapshots)
    }
}

// ── Find handler ──────────────────────────────────────────────────────────

cef::wrap_find_handler! {
    pub(super) struct WeldFindHandler {
        events: Arc<WebEventQueue>,
    }

    impl FindHandler {
        fn on_find_result(
            &self,
            _browser: Option<&mut cef::Browser>,
            _identifier: ::std::os::raw::c_int,
            count: ::std::os::raw::c_int,
            _selection_rect: Option<&cef::Rect>,
            active_match_ordinal: ::std::os::raw::c_int,
            final_update: ::std::os::raw::c_int,
        ) {
            push_navigation(&self.events,
                crate::surface::NavigationEvent::FindResult {
                    count,
                    active_match: active_match_ordinal,
                    final_update: final_update != 0,
                }
            );
        }
    }
}

impl WeldFindHandler {
    pub fn build(events: Arc<WebEventQueue>) -> cef::FindHandler {
        Self::new(events)
    }
}

// ── PDF print callback ────────────────────────────────────────────────────

cef::wrap_pdf_print_callback! {
    pub(super) struct WeldPdfCallback {
        events: Arc<WebEventQueue>,
    }

    impl PdfPrintCallback {
        fn on_pdf_print_finished(
            &self,
            path: Option<&cef::CefString>,
            ok: ::std::os::raw::c_int,
        ) {
            push_navigation(&self.events,
                crate::surface::NavigationEvent::PdfPrintFinished {
                    path: path.map(|p| p.to_string()).unwrap_or_default().into(),
                    ok: ok != 0,
                }
            );
        }
    }
}

impl WeldPdfCallback {
    pub fn build(events: Arc<WebEventQueue>) -> cef::PdfPrintCallback {
        Self::new(events)
    }
}
