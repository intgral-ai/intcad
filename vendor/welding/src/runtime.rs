// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use crate::error::WeldError;

// ── Shared config / log-severity (always available) ──────────────────────────

/// Configuration passed to [`CefRuntime::initialize`].
///
/// # Subprocess Tax — Read This First
///
/// CEF re-executes this binary as its renderer/GPU/utility subprocesses. You
/// must call [`CefRuntime::execute_process_from`] **as the first line of
/// `main()`** and `std::process::exit` immediately if it returns `Some(code)`:
///
/// ```no_run
/// fn main() {
///     let cef_path = std::env::var("CEF_PATH").expect("CEF_PATH required");
///     let sandbox = welding::CefSandboxMode::UnsandboxedTrustedContent;
///     if let Some(code) = welding::CefRuntime::execute_process_from(cef_path.as_ref(), sandbox)
///         .expect("failed to probe CEF subprocess role")
///     {
///         std::process::exit(code);
///     }
///     let config = welding::CefRuntimeConfig::new(cef_path, sandbox);
///     // now safe to call CefRuntime::initialize(config) and create browsers
/// }
/// ```
#[derive(Clone, Debug)]
pub struct CefRuntimeConfig {
    /// Directory containing the CEF binary distribution.
    ///
    /// - Windows: folder with `libcef.dll`, `icudtl.dat`, locale files, etc.
    ///   Under `cef-runtime` this is added to the DLL search path via
    ///   `SetDllDirectory` when CEF is not adjacent to the executable.
    /// - macOS: folder containing `Chromium Embedded Framework.framework`.
    ///   Under `cef-runtime` this is passed to `cef::load_library`.
    /// - Linux: folder with `libcef.so`, `icudtl.dat`, locale files, etc.
    pub cef_path: PathBuf,
    /// Override the whole `User-Agent` string.
    ///
    /// Process-wide, not per browser: CEF takes it in `CefSettings`, so every
    /// producer under this runtime shares it. For a smaller change, see
    /// [`Self::user_agent_product`], which alters only the product token and
    /// leaves the rest of Chromium's string intact.
    pub user_agent: Option<String>,
    /// Override only the product token, e.g. `MyApp/1.0`, keeping the rest of
    /// the `User-Agent` Chromium would have sent.
    pub user_agent_product: Option<String>,
    /// Chromium command-line switches, applied before CEF processes its own.
    ///
    /// `("disable-popup-blocking", None)` for a bare flag,
    /// `("lang", Some("en-GB".into()))` for one with a value. This is the only
    /// way to reach a great many Chromium behaviours, which have no CEF API.
    pub command_line_switches: Vec<(String, Option<String>)>,

    // SolidCraft: opt-in browser argv suppression; subprocess dispatch retains
    // CEF's real process arguments. Approved switches still use WeldApp.
    /// Ignore standard CEF/Chromium browser-process command-line arguments.
    ///
    /// Configuration via CefSettings and OnBeforeCommandLineProcessing still
    /// applies. Defaults to false to preserve existing callers' behavior.
    pub command_line_args_disabled: bool,

    /// Chromium process sandbox policy.
    ///
    /// Trusted-content mode passes null `sandbox_info` to CEF's process entry
    /// points and sets `CefSettings.no_sandbox = 1`. Sandboxed mode enables
    /// Chromium's platform process sandbox and may impose platform-specific
    /// packaging requirements.
    pub sandbox: CefSandboxMode,

    /// Path to the subprocess helper executable. `None` = re-use this binary
    /// (requires calling [`CefRuntime::execute_process_from`] at `main()` start).
    pub browser_subprocess_path: Option<PathBuf>,

    /// Persistent cache directory. `None` = in-memory / temporary.
    pub cache_path: Option<PathBuf>,

    /// **Not for production.** Single-process mode; skips subprocess spawning.
    pub single_process: bool,

    pub log_severity: CefLogSeverity,
}

impl CefRuntimeConfig {
    pub fn new(cef_path: impl Into<PathBuf>, sandbox: CefSandboxMode) -> Self {
        CefRuntimeConfig {
            cef_path: cef_path.into(),
            user_agent: None,
            user_agent_product: None,
            command_line_switches: Vec::new(),
            command_line_args_disabled: false, // SolidCraft: opt-in only.
            sandbox,
            browser_subprocess_path: None,
            cache_path: None,
            single_process: false,
            log_severity: CefLogSeverity::Default,
        }
    }
}

/// Chromium process sandbox policy for a CEF runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CefSandboxMode {
    /// Run CEF without Chromium's process sandbox.
    ///
    /// This is the historical welding behaviour, now made explicit so hosts do
    /// not accidentally treat the current runtime as a hardened browser
    /// boundary. It is intended for trusted-content embedding and demos.
    UnsandboxedTrustedContent,

    /// Enable Chromium's process sandbox for renderer, GPU, and utility
    /// subprocesses.
    ///
    /// Linux uses CEF's native namespace/setuid sandbox selection. macOS
    /// helper processes initialize `libcef_sandbox.dylib` before entering CEF.
    /// Windows requires CEF's `bootstrap.exe` plus a client DLL entry point;
    /// the ordinary re-executed-binary entry point rejects this mode until a
    /// bootstrap context is supplied explicitly.
    Sandboxed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CefLogSeverity {
    #[default]
    Default,
    Verbose,
    Info,
    Warning,
    Error,
    Fatal,
    Disable,
}

// ── cef-runtime: cef crate–backed implementation ─────────────────────────────

#[cfg(feature = "cef-runtime")]
mod cef_backed {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cef::args::Args;
    use cef::*;

    /// Live users of one runtime. A producer or the app holds a
    /// [`RuntimeLease`]; checked shutdown requires all of them released.
    #[derive(Debug, Default)]
    struct LeaseState {
        live: AtomicUsize,
    }

    /// One live user of this runtime, released before checked shutdown.
    ///
    /// Not a CEF handle by itself: dropping it only ends the Rust-side claim,
    /// so the owner thread must first release its browser/producer.
    #[derive(Debug)]
    pub struct RuntimeLease {
        state: Arc<LeaseState>,
    }

    impl Drop for RuntimeLease {
        fn drop(&mut self) {
            self.state.live.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// A successfully initialized runtime that has not attached a browser yet.
    ///
    /// One opaque, non-Clone owner carried from the pre-document gate to the
    /// later Assistant attachment; initialization is never repeated.
    pub struct PreparedCefRuntime {
        runtime: CefRuntime,
    }

    impl PreparedCefRuntime {
        /// Consume the prepared owner for the ordinary runtime/browser lifecycle.
        pub fn into_runtime(self) -> CefRuntime {
            self.runtime
        }
    }

    /// CEF global runtime under `cef-runtime`.
    ///
    /// Process-global CEF state has exactly one live owner. Dropping it does
    /// **not** call into CEF: an abnormal drop retains containment (no native
    /// shutdown with unresolved browser creation/references) instead of
    /// pretending teardown succeeded.
    pub struct CefRuntime {
        pub(super) config: CefRuntimeConfig,
        leases: Arc<LeaseState>,
        init_thread: std::thread::ThreadId,
        shut_down: bool,
    }

    /// Borrowed sandbox context supplied by CEF's Windows bootstrap executable.
    ///
    /// CEF 151 creates this value before loading the application's client DLL.
    /// The DLL entry point must pass the same context to both
    /// [`execute_process`](Self::execute_process) and
    /// [`initialize`](Self::initialize). The bootstrap owns and destroys it.
    #[cfg(target_os = "windows")]
    pub struct CefWindowsSandboxContext<'a> {
        instance: cef::sys::HINSTANCE,
        sandbox_info: std::ptr::NonNull<u8>,
        _borrow: std::marker::PhantomData<&'a mut u8>,
    }

    #[cfg(target_os = "windows")]
    impl<'a> CefWindowsSandboxContext<'a> {
        /// Borrow the process instance and sandbox context passed to the
        /// client-DLL `RunWinMain` entry point by CEF's `bootstrap.exe`.
        ///
        /// # Safety
        ///
        /// `instance` and `sandbox_info` must be the unmodified values received
        /// by the active bootstrap callback. The bootstrap must keep the
        /// sandbox context alive through both calls on this value.
        pub unsafe fn from_raw(
            instance: *mut std::ffi::c_void,
            sandbox_info: *mut u8,
        ) -> Result<Self, WeldError> {
            let instance = std::ptr::NonNull::new(instance).ok_or_else(|| {
                WeldError::SandboxSetup(
                    "CEF bootstrap entered the client DLL without a process instance".into(),
                )
            })?;
            let sandbox_info = std::ptr::NonNull::new(sandbox_info).ok_or_else(|| {
                WeldError::SandboxSetup(
                    "CEF bootstrap entered the client DLL without a sandbox context".into(),
                )
            })?;
            Ok(Self {
                instance: cef::sys::HINSTANCE(instance.as_ptr().cast()),
                sandbox_info,
                _borrow: std::marker::PhantomData,
            })
        }

        /// Execute a renderer/GPU/utility process under the bootstrap-owned
        /// sandbox. Returns `None` only for the browser process.
        pub fn execute_process(
            &self,
            cef_path: &std::path::Path,
        ) -> Result<Option<i32>, WeldError> {
            maybe_load_library(cef_path)?;
            pin_cef_api_version();
            let args = cef::MainArgs {
                instance: self.instance,
            };
            let mut app = crate::app::WeldApp::build(std::sync::Arc::new(Vec::new()));
            let code =
                cef::execute_process(Some(&args), Some(&mut app), self.sandbox_info.as_ptr());
            if code >= 0 { Ok(Some(code)) } else { Ok(None) }
        }

        /// Initialize the browser process under the bootstrap-owned sandbox.
        pub fn initialize(&self, config: CefRuntimeConfig) -> Result<CefRuntime, WeldError> {
            if config.sandbox != CefSandboxMode::Sandboxed {
                return Err(WeldError::SandboxSetup(
                    "a Windows bootstrap sandbox context requires CefSandboxMode::Sandboxed".into(),
                ));
            }
            maybe_load_library(&config.cef_path)?;
            pin_cef_api_version();
            let args = cef::MainArgs {
                instance: self.instance,
            };
            let mut app = crate::app::WeldApp::build(std::sync::Arc::new(
                config.command_line_switches.clone(),
            ));
            let settings = build_settings(&config);
            let code = cef::initialize(
                Some(&args),
                Some(&settings),
                Some(&mut app),
                self.sandbox_info.as_ptr(),
            );
            if code == 0 {
                return Err(WeldError::InitFailed { code });
            }
            Ok(CefRuntime {
                config,
                leases: Arc::new(LeaseState::default()),
                init_thread: std::thread::current().id(),
                shut_down: false,
            })
        }
    }

    impl CefRuntime {
        /// Load libcef and call `cef_execute_process`. Must be the very first
        /// CEF call in `main()`.
        ///
        /// Returns `Ok(Some(exit_code))` for CEF subprocesses — the caller must
        /// `std::process::exit(exit_code)`. Returns `Ok(None)` for the browser
        /// (host) process.
        pub fn execute_process_from(
            cef_path: &std::path::Path,
            sandbox: CefSandboxMode,
        ) -> Result<Option<i32>, WeldError> {
            validate_direct_entrypoint(sandbox)?;
            maybe_load_library(cef_path)?;
            pin_cef_api_version();
            let args = Args::new();
            // The same app both roles need: its render-process handler is what
            // makes script evaluation possible in the renderer.
            let mut app = crate::app::WeldApp::build(std::sync::Arc::new(Vec::new()));
            let code = cef::execute_process(
                Some(args.as_main_args()),
                Some(&mut app),
                sandbox_info_for(sandbox),
            );
            if code >= 0 { Ok(Some(code)) } else { Ok(None) }
        }

        // SolidCraft: inspect through initialize's loading path without
        // constructing a shutdown owner. Linux keeps the existing linked
        // libcef.so/LD_LIBRARY_PATH loader contract.
        #[cfg(target_os = "linux")]
        pub fn inspect_identity(
            cef_path: &std::path::Path,
        ) -> Result<crate::runtime_identity::RuntimeIdentity, WeldError> {
            maybe_load_library(cef_path)?;
            crate::runtime_identity::inspect_native()
                .map_err(|reason| WeldError::BrowserOp(reason.into()))
        }

        /// Run a CEF subprocess that loads the framework itself.
        ///
        /// macOS launches separate helper executables rather than re-running
        /// the host binary, and those helpers must hand CEF the *same* app, or
        /// the renderer has no handlers and anything needing the render process
        /// (script results) silently never answers. A helper that calls
        /// `cef_execute_process` on its own cannot know that.
        pub fn try_run_subprocess(
            args: &cef::args::Args,
            sandbox: CefSandboxMode,
        ) -> Result<i32, WeldError> {
            pin_cef_api_version();
            #[cfg(target_os = "macos")]
            let _sandbox = MacSandbox::initialize(args.as_main_args(), sandbox)?;
            let mut app = crate::app::WeldApp::build(std::sync::Arc::new(Vec::new()));
            Ok(cef::execute_process(
                Some(args.as_main_args()),
                Some(&mut app),
                sandbox_info_for(sandbox),
            ))
        }

        /// Compatibility wrapper for helpers that cannot surface setup errors.
        /// New helpers should call [`Self::try_run_subprocess`].
        pub fn run_subprocess(args: &cef::args::Args, sandbox: CefSandboxMode) -> i32 {
            match Self::try_run_subprocess(args, sandbox) {
                Ok(code) => code,
                Err(error) => {
                    eprintln!("welding: {error}");
                    1
                },
            }
        }

        /// Initialise CEF for the pre-document Assistant gate through the pinned
        /// `cef::sys::cef_initialize` boundary.
        ///
        /// On native false the complete init-call frame (raw argv backing, owned
        /// settings strings, App reference and raw sys structs) is retained without
        /// running CEF destructors. On success the frame drops normally while the
        /// runtime is live.
        ///
        /// CONFORMANCE GATE, NOT PROVEN: the immediate sys callee/loader return and
        /// each platform's teardown are not shown to make zero post-false CEF calls.
        /// This seam therefore does not by itself authorize startup cancellation.
        pub fn initialize_prepared(config: CefRuntimeConfig) -> Result<PreparedCefRuntime, WeldError> {
            Self::initialize_prepared_inner(config).map(|prepared| PreparedCefRuntime { runtime: prepared })
        }

        fn initialize_prepared_inner(config: CefRuntimeConfig) -> Result<CefRuntime, WeldError> {
            use cef::ImplApp as _;
            use cef::rc::Rc as _;

            validate_direct_entrypoint(config.sandbox)?;
            maybe_load_library(&config.cef_path)?;
            pin_cef_api_version();
            let args = Args::new();
            let mut app = crate::app::WeldApp::build(std::sync::Arc::new(config.command_line_switches.clone()));
            let settings = build_settings(&config);
            // Converted exactly as the generated wrapper does; the borrowed raw
            // settings refer to the owned `settings` strings retained below.
            let main_args: cef::sys::cef_main_args_t = args.as_main_args().clone().into();
            let raw_settings: cef::sys::cef_settings_t = settings.clone().into();
            let raw_app = {
                // Same add-ref the generated wrapper performs; balanced on success by
                // dropping the frame, retained on native false.
                unsafe { app.add_ref() };
                app.get_raw()
            };
            let frame = RetainedInitInputs { args, settings, app, main_args, raw_settings, raw_app };
            let code = unsafe {
                cef::sys::cef_initialize(
                    &frame.main_args,
                    &frame.raw_settings,
                    frame.raw_app,
                    sandbox_info_for(config.sandbox).cast(),
                )
            };
            if code == 0 {
                // Failure-only retention: no CEF destructor may run after native false.
                std::mem::forget(frame);
                return Err(WeldError::InitFailed { code });
            }
            drop(frame);
            Ok(CefRuntime {
                config,
                leases: Arc::new(LeaseState::default()),
                init_thread: std::thread::current().id(),
                shut_down: false,
            })
        }

        /// Initialise CEF. Call after [`Self::execute_process_from`] confirms
        /// this is the browser process.
        pub fn initialize(config: CefRuntimeConfig) -> Result<Self, WeldError> {
            validate_direct_entrypoint(config.sandbox)?;
            maybe_load_library(&config.cef_path)?;
            pin_cef_api_version();
            let args = Args::new();
            let mut app = crate::app::WeldApp::build(std::sync::Arc::new(
                config.command_line_switches.clone(),
            ));
            let settings = build_settings(&config);
            let code = cef::initialize(
                Some(args.as_main_args()),
                Some(&settings),
                Some(&mut app),
                sandbox_info_for(config.sandbox),
            );
            if code == 0 {
                return Err(WeldError::InitFailed { code });
            }
            Ok(CefRuntime {
                config,
                leases: Arc::new(LeaseState::default()),
                init_thread: std::thread::current().id(),
                shut_down: false,
            })
        }

        /// Claim one live user of this runtime. Release it before [`Self::shutdown`].
        pub fn lease(&self) -> RuntimeLease {
            self.leases.live.fetch_add(1, Ordering::AcqRel);
            RuntimeLease { state: Arc::clone(&self.leases) }
        }

        /// Shut CEF down once, on its initialization thread, with no live owners.
        ///
        /// On refusal the runtime is returned to the caller, which must retain
        /// it: dropping it must not become an implicit native shutdown.
        pub fn shutdown(mut self) -> Result<(), (Self, WeldError)> {
            if self.init_thread != std::thread::current().id() {
                return Err((
                    self,
                    WeldError::BrowserOp(
                        "CEF shutdown must run on its initialization thread".into(),
                    ),
                ));
            }
            let live = self.leases.live.load(Ordering::Acquire);
            if live != 0 {
                return Err((
                    self,
                    WeldError::BrowserOp(format!(
                        "CEF shutdown refused: {live} live runtime owner(s)"
                    )),
                ));
            }
            self.shut_down = true;
            cef::shutdown();
            Ok(())
        }

        /// Block on CEF's message loop. Use when CEF owns the main thread.
        pub fn run_message_loop(&self) {
            cef::run_message_loop();
        }

        /// Pump CEF's message loop once on platforms where the host owns it.
        ///
        /// Windows uses CEF's dedicated message-loop thread so this is a no-op.
        pub fn do_message_loop_work(&self) {
            #[cfg(not(target_os = "windows"))]
            cef::do_message_loop_work();
        }

        pub fn config(&self) -> &CefRuntimeConfig {
            &self.config
        }
    }

    impl Drop for CefRuntime {
        fn drop(&mut self) {
            if !self.shut_down {
                // No native call here: unresolved ownership must stay contained,
                // not be released into CEF from an arbitrary drop site.
                log::warn!("CEF runtime dropped without checked shutdown; native state is retained");
            }
        }
    }

    /// Pin the CEF API version for this process.
    ///
    /// CEF 148 introduced API versioning: each call that crosses the libcef
    /// boundary checks the host has pinned a version. Without this, libcef
    /// aborts with "CefApp_0_CToCpp called with invalid version -1" the first
    /// time the App vtable is dispatched. The pin lasts for the process and
    /// only the first call has effect, so it's safe to call at the top of
    /// both `execute_process_from` (subprocess path) and `initialize` (host
    /// path).
    ///
    /// We pin to `CEF_API_VERSION_LAST` — the latest version the cef-dll-sys
    /// bindings know about, which by construction matches the libcef.so we
    /// link against. Cross-platform: required equally on Linux, Windows, and
    /// macOS as of CEF 148.
    fn pin_cef_api_version() {
        let _ = cef::api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    }

    fn build_settings(config: &CefRuntimeConfig) -> cef::Settings {
        let mut s = cef::Settings {
            windowless_rendering_enabled: 1,
            // SolidCraft: clear inherited browser switches before WeldApp adds
            // the host-approved list; never change execute_process_from args.
            command_line_args_disabled: i32::from(config.command_line_args_disabled),
            // Windows uses CEF's supported dedicated UI thread. Other platforms
            // continue to integrate CefDoMessageLoopWork into the host loop.
            multi_threaded_message_loop: cfg!(target_os = "windows") as _,
            external_message_pump: cfg!(not(target_os = "windows")) as _,
            // Keep this in lockstep with sandbox_info_for.
            no_sandbox: no_sandbox_for(config.sandbox),
            log_severity: match config.log_severity {
                CefLogSeverity::Default => LogSeverity::DEFAULT,
                CefLogSeverity::Verbose => LogSeverity::VERBOSE,
                CefLogSeverity::Info => LogSeverity::INFO,
                CefLogSeverity::Warning => LogSeverity::WARNING,
                CefLogSeverity::Error => LogSeverity::ERROR,
                CefLogSeverity::Fatal => LogSeverity::FATAL,
                CefLogSeverity::Disable => LogSeverity::DISABLE,
            },
            ..Default::default()
        };
        // root_cache_path: CEF uses this directory as the parent of all
        // browser cache/session data. Without it, CEF logs a warning about
        // "default value may lead to unintended process singleton behavior"
        // and may share state unexpectedly across welding-using applications.
        if let Some(cache_path) = config.cache_path.as_ref() {
            let path_str = cache_path.to_string_lossy();
            s.root_cache_path = (&*path_str).into();
        }
        if let Some(subprocess_path) = config.browser_subprocess_path.as_ref() {
            let path_str = subprocess_path.to_string_lossy();
            s.browser_subprocess_path = (&*path_str).into();
        }
        if let Some(ua) = config.user_agent.as_deref() {
            s.user_agent = ua.into();
        }
        if let Some(product) = config.user_agent_product.as_deref() {
            s.user_agent_product = product.into();
        }
        s
    }

    // SolidCraft: assert the real native Settings field, not just the app's
    // environment parser. Requires cef-runtime; no CEF initialization here.
    #[cfg(test)]
    mod settings_tests {
        use super::*;

        #[test]
        fn inherited_browser_switches_are_disabled_only_on_request() {
            let mut config = CefRuntimeConfig::new(".", CefSandboxMode::Sandboxed);
            let legacy = build_settings(&config);
            assert_eq!(legacy.command_line_args_disabled, 0);
            assert_eq!(legacy.windowless_rendering_enabled, 1);

            config.command_line_args_disabled = true;
            config.command_line_switches = vec![
                ("ozone-platform".into(), Some("wayland".into())),
                (
                    "disable-features".into(),
                    Some("Translate,WaylandFractionalScaleV1".into()),
                ),
            ];
            let restricted = build_settings(&config);
            assert_eq!(restricted.command_line_args_disabled, 1);
            assert_eq!(restricted.windowless_rendering_enabled, 1);
            assert_eq!(restricted.no_sandbox, legacy.no_sandbox);
        }
    }

    /// Everything the direct sys call borrowed or owned. Retained whole on false.
    /// Fields are not read individually: their Drop/retention is the contract.
    #[allow(dead_code)]
    struct RetainedInitInputs {
        args: Args,
        settings: cef::Settings,
        app: cef::App,
        main_args: cef::sys::cef_main_args_t,
        raw_settings: cef::sys::cef_settings_t,
        raw_app: *mut cef::sys::cef_app_t,
    }

    fn sandbox_info_for(sandbox: CefSandboxMode) -> *mut u8 {
        match sandbox {
            CefSandboxMode::UnsandboxedTrustedContent | CefSandboxMode::Sandboxed => {
                std::ptr::null_mut()
            },
        }
    }

    fn no_sandbox_for(sandbox: CefSandboxMode) -> i32 {
        match sandbox {
            CefSandboxMode::UnsandboxedTrustedContent => 1,
            CefSandboxMode::Sandboxed => 0,
        }
    }

    fn validate_direct_entrypoint(sandbox: CefSandboxMode) -> Result<(), WeldError> {
        #[cfg(target_os = "windows")]
        if sandbox == CefSandboxMode::Sandboxed {
            return Err(WeldError::SandboxSetup(
                "sandboxed Windows CEF requires bootstrap.exe and a client-DLL entry point; \
                 execute_process_from cannot create the required sandbox context"
                    .into(),
            ));
        }
        let _ = sandbox;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    struct MacSandbox {
        library: libloading::Library,
        context: std::ptr::NonNull<std::ffi::c_void>,
    }

    #[cfg(target_os = "macos")]
    impl MacSandbox {
        fn initialize(
            args: &cef::MainArgs,
            mode: CefSandboxMode,
        ) -> Result<Option<Self>, WeldError> {
            if mode == CefSandboxMode::UnsandboxedTrustedContent {
                return Ok(None);
            }

            let executable = std::env::current_exe().map_err(|error| {
                WeldError::SandboxSetup(format!("cannot locate macOS helper executable: {error}"))
            })?;
            let helper_dir = executable.parent().ok_or_else(|| {
                WeldError::SandboxSetup(
                    "macOS helper executable has no containing directory".into(),
                )
            })?;
            let library_path = helper_dir
                .join(
                    "../../../Chromium Embedded Framework.framework/Libraries/libcef_sandbox.dylib",
                )
                .canonicalize()
                .map_err(|error| {
                    WeldError::SandboxSetup(format!(
                        "cannot locate libcef_sandbox.dylib from {}: {error}",
                        executable.display()
                    ))
                })?;

            // SAFETY: the dylib is from the same signed CEF framework bundle as
            // libcef. Symbol signatures are the C API declared by CEF.
            let library = unsafe { libloading::Library::new(&library_path) }.map_err(|error| {
                WeldError::SandboxSetup(format!("cannot load {}: {error}", library_path.display()))
            })?;
            let context = unsafe {
                let initialize = library
                    .get::<unsafe extern "C" fn(
                        std::os::raw::c_int,
                        *mut *mut std::os::raw::c_char,
                    ) -> *mut std::ffi::c_void>(b"cef_sandbox_initialize")
                    .map_err(|error| {
                        WeldError::SandboxSetup(format!(
                            "libcef_sandbox.dylib lacks cef_sandbox_initialize: {error}"
                        ))
                    })?;
                std::ptr::NonNull::new(initialize(args.argc, args.argv)).ok_or_else(|| {
                    WeldError::SandboxSetup("cef_sandbox_initialize returned null".into())
                })?
            };

            Ok(Some(Self { library, context }))
        }
    }

    #[cfg(target_os = "macos")]
    impl Drop for MacSandbox {
        fn drop(&mut self) {
            // SAFETY: context came from this library's initialize function and
            // is destroyed once while the library is still loaded.
            unsafe {
                match self
                    .library
                    .get::<unsafe extern "C" fn(*mut std::ffi::c_void)>(b"cef_sandbox_destroy")
                {
                    Ok(destroy) => destroy(self.context.as_ptr()),
                    Err(error) => eprintln!(
                        "welding: libcef_sandbox.dylib lacks cef_sandbox_destroy: {error}"
                    ),
                }
            }
        }
    }

    /// Platform-specific library loading:
    /// - macOS: `cef::load_library` must be called before any other CEF API.
    /// - Windows: add `cef_path` to the DLL search path so `libcef.dll` is found
    ///   even when not adjacent to the executable.
    /// - Linux: `libcef.so` is expected to be in `LD_LIBRARY_PATH`; no action.
    fn maybe_load_library(cef_path: &std::path::Path) -> Result<(), WeldError> {
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::ffi::OsStrExt;

            const FRAMEWORK: &str =
                "Chromium Embedded Framework.framework/Chromium Embedded Framework";
            let lib_path = cef_path.join(FRAMEWORK);
            // `cef_load_library` takes a plain C string path, not a CefString.
            let c_path = std::ffi::CString::new(lib_path.as_os_str().as_bytes()).map_err(|_| {
                WeldError::CefLoadFailed {
                    path: lib_path.display().to_string(),
                }
            })?;
            let ok = unsafe { cef::load_library(Some(&*c_path.as_ptr().cast())) };
            if ok != 1 {
                return Err(WeldError::CefLoadFailed {
                    path: lib_path.display().to_string(),
                });
            }
        }
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::System::LibraryLoader::SetDllDirectoryW;
            use windows::core::HSTRING;
            let wide = HSTRING::from(cef_path.as_os_str());
            // Ignore failure: if libcef.dll is already in the search path this is
            // a no-op.
            unsafe {
                let _ = SetDllDirectoryW(&wide);
            }
        }
        let _ = cef_path;
        Ok(())
    }
}

#[cfg(feature = "cef-runtime")]
pub use cef_backed::{CefRuntime, PreparedCefRuntime, RuntimeLease};
#[cfg(all(feature = "cef-runtime", target_os = "windows"))]
pub use cef_backed::CefWindowsSandboxContext;

// ── stub: no `cef-runtime` feature ────────────────────────────────────────────
//
// Without the `cef-runtime` feature, the crate still compiles so downstream
// code can reference its types, but the runtime returns errors on every call.
// (The previous libloading-backed scaffold was removed — it was unused and
// duplicated CEF type definitions that drift across CEF versions.)

#[cfg(not(feature = "cef-runtime"))]
mod stub {
    use super::*;

    /// CEF global runtime stub. Construct only via [`initialize`]; every
    /// method returns [`WeldError::FeatureRequired`].
    pub struct CefRuntime {
        pub(super) config: CefRuntimeConfig,
    }

    /// No live native owner exists without `cef-runtime`; the claim is inert.
    pub struct RuntimeLease;

    pub struct PreparedCefRuntime;

    impl PreparedCefRuntime {
        pub fn into_runtime(self) -> CefRuntime {
            CefRuntime { config: CefRuntimeConfig::new(".", CefSandboxMode::UnsandboxedTrustedContent) }
        }
    }

    impl CefRuntime {
        pub fn lease(&self) -> RuntimeLease {
            RuntimeLease
        }

        pub fn shutdown(self) -> Result<(), (Self, WeldError)> {
            Err((self, WeldError::FeatureRequired("cef-runtime")))
        }

        pub fn initialize_prepared(_config: CefRuntimeConfig) -> Result<PreparedCefRuntime, WeldError> {
            Err(WeldError::FeatureRequired("cef-runtime"))
        }

        // SolidCraft: identity querying is unavailable without native CEF.
        #[cfg(target_os = "linux")]
        pub fn inspect_identity(
            _cef_path: &std::path::Path,
        ) -> Result<crate::runtime_identity::RuntimeIdentity, WeldError> {
            Err(WeldError::FeatureRequired("cef-runtime"))
        }

        pub fn execute_process_from(
            _cef_path: &std::path::Path,
            _sandbox: CefSandboxMode,
        ) -> Result<Option<i32>, WeldError> {
            Err(WeldError::FeatureRequired("cef-runtime"))
        }

        pub fn initialize(_config: CefRuntimeConfig) -> Result<Self, WeldError> {
            Err(WeldError::FeatureRequired("cef-runtime"))
        }

        pub fn run_message_loop(&self) {}
        pub fn do_message_loop_work(&self) {}

        pub fn config(&self) -> &CefRuntimeConfig {
            &self.config
        }
    }
}

#[cfg(not(feature = "cef-runtime"))]
pub use stub::{CefRuntime, PreparedCefRuntime, RuntimeLease};
