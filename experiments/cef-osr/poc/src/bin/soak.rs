//! Lifecycle soak: create/destroy, resize pressure, visibility, scale changes,
//! renderer crash recovery, and the first-frame input queue — in one process,
//! so leaks and descriptor growth are visible.
//!
//! No window: CEF only needs the Wayland (or X11) connection, and the host
//! device is the same one the panel uses.
//!
//! Env:
//!   CEF_PATH        CEF binary distribution (required)
//!   SOAK_CYCLES     create/destroy iterations (default 100)
//!   SOAK_RESIZES    resizes per iteration (default 8)
//!   SOAK_CRASH      1 to run the renderer-crash recovery case at the end
//!   SOAK_SECS       seconds to pump each iteration besides the first frame
//!   SOAK_SWITCHES   Chromium switches, comma separated
//!   SOAK_SANDBOX    "sandboxed" for CEF's process sandbox
//!   SOAK_OUT        JSONL file for the per-iteration samples

use std::time::{Duration, Instant};

use cef_osr_poc::queue::{char_event, InputQueue};
use welding::linux_cef::{LinuxCefConfig, LinuxCefProducer};
use welding::wgpu;
use welding::{
    CefRuntime, CefRuntimeConfig, CefSandboxMode, CefSurfaceConfig, CefSurfaceProducer, HostWgpuContext, MouseAction, MouseButton, MouseEvent,
    WgpuTextureImporter,
};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_u32(name: &str, fallback: u32) -> u32 {
    env(name).and_then(|value| value.parse().ok()).unwrap_or(fallback)
}

fn receipt(tag: &str, body: &str) {
    println!("RECEIPT {{\"tag\":\"{tag}\",{body}}}");
}

fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|text| text.lines().find(|line| line.starts_with("VmRSS:")).and_then(|line| line.split_whitespace().nth(1)?.parse().ok()))
        .unwrap_or(0)
}

fn fds() -> usize {
    std::fs::read_dir("/proc/self/fd").map(|entries| entries.count()).unwrap_or(0)
}

/// VRAM in MiB for the DRM cards that expose it (amdgpu does, xe does not).
fn vram_mib() -> Option<u64> {
    for card in ["card0", "card1", "card2"] {
        let path = format!("/sys/class/drm/{card}/device/mem_info_vram_used");
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(bytes) = text.trim().parse::<u64>() {
                return Some(bytes / 1048576);
            }
        }
    }
    None
}

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let Some(cef_path) = env("CEF_PATH") else {
        eprintln!("CEF_PATH must point at a CEF binary distribution");
        return 2;
    };
    let sandbox = match env("SOAK_SANDBOX").as_deref() {
        Some("sandboxed") => CefSandboxMode::Sandboxed,
        _ => CefSandboxMode::UnsandboxedTrustedContent,
    };
    match CefRuntime::execute_process_from(std::path::Path::new(&cef_path), sandbox) {
        Ok(Some(code)) => return code,
        Ok(None) => {}
        Err(error) => {
            eprintln!("CEF subprocess probe failed: {error}");
            return 2;
        }
    }
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // ── host device ─────────────────────────────────────────────────────────
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    })) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("no Vulkan adapter: {error}");
            return 3;
        }
    };
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("cef-osr-soak"),
        required_features: adapter.features() & wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF,
        ..Default::default()
    };
    let (device, queue) = match welding::build_dmabuf_capable_device(&adapter, &descriptor) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("DMABUF-capable device failed: {error}");
            return 3;
        }
    };
    let host = HostWgpuContext::new(device, queue);

    // ── CEF runtime ─────────────────────────────────────────────────────────
    let mut config = CefRuntimeConfig::new(&cef_path, sandbox);
    config.cache_path = Some(std::env::temp_dir().join("cef-osr-soak-cache"));
    config.command_line_switches = vec![("no-first-run".into(), None), ("no-default-browser-check".into(), None)];
    if let Some(list) = env("SOAK_SWITCHES") {
        config.command_line_switches.extend(list.split(',').filter(|part| !part.is_empty()).map(|part| match part.split_once('=') {
            Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
            None => (part.to_owned(), None),
        }));
    }
    let runtime = match CefRuntime::initialize(config) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("CEF initialize failed: {error}");
            return 4;
        }
    };

    let cycles = env_u32("SOAK_CYCLES", 100);
    let resizes = env_u32("SOAK_RESIZES", 8);
    let pump_secs = env_u32("SOAK_SECS", 0);
    let url = env("SOAK_URL").unwrap_or_else(|| format!("file://{}/fixtures/probe.html", env!("CARGO_MANIFEST_DIR")));
    let out_path = env("SOAK_OUT");
    let mut out = out_path.as_ref().and_then(|path| std::fs::File::create(path).ok());

    receipt(
        "soak_start",
        &format!("\"cycles\":{cycles},\"resizes\":{resizes},\"rss_kib\":{},\"fds\":{},\"vram_mib\":{}", rss_kib(), fds(), vram_mib().unwrap_or(0)),
    );

    let mut first_frame_seen = 0u32;
    let mut failures = 0u32;
    let mut queue_report = String::from("null");
    let started = Instant::now();

    for cycle in 0..cycles {
        let producer = match LinuxCefProducer::new(
            &runtime,
            LinuxCefConfig {
                surface: CefSurfaceConfig {
                    initial_url: url.clone(),
                    initial_size: dpi::PhysicalSize::new(1024, 720),
                    scale_factor: 1.0,
                    prefer_accelerated: true,
                    ..Default::default()
                },
            },
        ) {
            Ok(producer) => producer,
            Err(error) => {
                failures += 1;
                eprintln!("cycle {cycle}: browser creation failed: {error}");
                continue;
            }
        };
        let mut producer = producer;
        let _ = producer.resize(dpi::PhysicalSize::new(1024, 720));
        let _ = producer.set_visible(true);

        // Cycle 0: prove the first-frame queue, with a control that shows what
        // happens without it. The page mirrors every input into its title.
        let mut queue = InputQueue::new(64);
        if cycle == 0 {
            // Sent straight through, before any frame: CEF drops it.
            if std::env::var_os("SOAK_CONTROL").is_some() {
                let _ = producer.send_keyboard_input(welding::KeyEvent {
                    kind: welding::KeyEventKind::Char,
                    windows_key_code: 0,
                    native_key_code: 0,
                    character: Some('X'),
                    modifiers: welding::EventModifiers::default(),
                });
            }
            // The click is held too: a click sent before the first frame is
            // dropped as well, and without it the page never focuses the field,
            // so the replayed characters would go nowhere.
            queue.push_motion(150, 98, welding::EventModifiers::default());
            for action in [MouseAction::Pressed, MouseAction::Released] {
                queue.push(cef_osr_poc::queue::Input::Mouse(MouseEvent {
                    x: 150,
                    y: 98,
                    button: MouseButton::Left,
                    action,
                    modifiers: welding::EventModifiers::default(),
                }));
            }
            for ch in "queued".chars() {
                queue.push(char_event(ch));
            }
        }

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut got = false;
        let mut titles: Vec<String> = Vec::new();
        while Instant::now() < deadline {
            runtime.do_message_loop_work();
            while let Some(event) = producer.poll_web_event() {
                let text = format!("{event:?}");
                if cycle == 0 && std::env::var_os("SOAK_TRACE").is_some() {
                    eprintln!("EVENT {text}");
                }
                if text.contains("TitleChanged") {
                    if let Some(title) = text.split("title: \"").nth(1) {
                        titles.push(title.chars().take(80).collect());
                    }
                }
            }
            if let Some(native) = producer.acquire_native_frame() {
                match WgpuTextureImporter::import(native, &host) {
                    Ok(_) => {
                        got = true;
                        queue.ready();
                        break;
                    }
                    Err(error) => {
                        failures += 1;
                        eprintln!("cycle {cycle}: import failed: {error}");
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        if got {
            first_frame_seen += 1;
        }

        if cycle == 0 {
            // Pointer first, then a settle, then the characters: the click and
            // the typing must not travel in one burst, and the page needs a
            // moment after its first frame before it hit-tests anything.
            let settle0 = Instant::now() + Duration::from_millis(300);
            while Instant::now() < settle0 {
                runtime.do_message_loop_work();
                while producer.poll_web_event().is_some() {}
                std::thread::sleep(Duration::from_millis(5));
            }
            if std::env::var_os("SOAK_DIRECT").is_some() {
                let _ = producer.move_focus(welding::FocusDirection::Forward);
                for action in [MouseAction::Moved, MouseAction::Pressed, MouseAction::Released] {
                    let _ = producer.send_mouse_input(MouseEvent {
                        x: 150,
                        y: 98,
                        button: MouseButton::Left,
                        action,
                        modifiers: welding::EventModifiers::default(),
                    });
                }
            }
            let pointer = queue.flush_pointer(&mut producer);
            let settle = Instant::now() + Duration::from_millis(150);
            while Instant::now() < settle {
                runtime.do_message_loop_work();
                while producer.poll_web_event().is_some() {}
                std::thread::sleep(Duration::from_millis(5));
            }
            let sent = queue.flush(&mut producer) + pointer;
            let flush_deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < flush_deadline {
                runtime.do_message_loop_work();
                while let Some(event) = producer.poll_web_event() {
                    let text = format!("{event:?}");
                    if text.contains("TitleChanged") {
                        if let Some(title) = text.split("title: \"").nth(1) {
                            titles.push(title.chars().take(80).collect());
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            let typed = titles.iter().any(|title| title.contains("queued"));
            let control = titles.iter().any(|title| title.contains('X'));
            queue_report = format!(
                "\"queue_sent\":{sent},\"queue_held\":{},\"queue_coalesced\":{},\"queue_refused\":{},\"queue_typed\":{typed},\"queue_control_X_delivered\":{control}",
                queue.held, queue.coalesced, queue.refused
            );
            receipt("queue", &queue_report);
        }

        // Resize pressure: odd sizes, then a zero size, then back. CEF must not
        // crash and must keep painting afterwards.
        for index in 0..resizes {
            let width = 320 + (index * 97) % 900;
            let height = 200 + (index * 53) % 600;
            if producer.resize(dpi::PhysicalSize::new(width, height)).is_err() {
                failures += 1;
            }
            runtime.do_message_loop_work();
            while producer.poll_web_event().is_some() {}
            if let Some(native) = producer.acquire_native_frame() {
                let _ = WgpuTextureImporter::import(native, &host);
            }
        }
        let _ = producer.resize(dpi::PhysicalSize::new(0, 0));
        runtime.do_message_loop_work();
        let _ = producer.resize(dpi::PhysicalSize::new(1024, 720));
        runtime.do_message_loop_work();

        // Visibility and fractional scale, both of which CEF handles through
        // the host's WasHidden / screen-info path.
        let _ = producer.set_visible(false);
        std::thread::sleep(Duration::from_millis(10));
        let _ = producer.set_visible(true);
        let _ = producer.set_scale_factor(1.25);
        runtime.do_message_loop_work();
        let _ = producer.set_scale_factor(2.0);
        runtime.do_message_loop_work();
        let _ = producer.set_scale_factor(1.0);

        if pump_secs > 0 {
            let pump_deadline = Instant::now() + Duration::from_secs(u64::from(pump_secs));
            while Instant::now() < pump_deadline {
                runtime.do_message_loop_work();
                while producer.poll_web_event().is_some() {}
                if let Some(native) = producer.acquire_native_frame() {
                    let _ = WgpuTextureImporter::import(native, &host);
                }
                std::thread::sleep(Duration::from_millis(4));
            }
        }

        if let Err(error) = producer.close() {
            eprintln!("cycle {cycle}: close failed: {error}");
        }
        drop(producer);

        let line = format!(
            "{{\"cycle\":{cycle},\"first_frame\":{got},\"rss_kib\":{},\"fds\":{},\"vram_mib\":{}}}",
            rss_kib(),
            fds(),
            vram_mib().unwrap_or(0)
        );
        if let Some(file) = out.as_mut() {
            use std::io::Write as _;
            let _ = writeln!(file, "{line}");
        }
        if cycle % 10 == 0 || cycle + 1 == cycles {
            receipt("cycle", &line.trim_matches(['{', '}']));
        }
    }

    // ── renderer crash and recovery ─────────────────────────────────────────
    let mut crash_report = String::from("null");
    if env("SOAK_CRASH").as_deref() == Some("1") {
        let mut producer = match LinuxCefProducer::new(
            &runtime,
            LinuxCefConfig {
                surface: CefSurfaceConfig {
                    initial_url: url.clone(),
                    initial_size: dpi::PhysicalSize::new(1024, 720),
                    scale_factor: 1.0,
                    prefer_accelerated: true,
                    ..Default::default()
                },
            },
        ) {
            Ok(producer) => producer,
            Err(error) => {
                eprintln!("crash case: browser creation failed: {error}");
                return 5;
            }
        };
        let _ = producer.resize(dpi::PhysicalSize::new(1024, 720));
        let _ = producer.set_visible(true);
        let first = wait_frame(&runtime, &mut producer, &host, 10);
        let _ = producer.navigate_to_url("chrome://crash");
        let mut terminated = false;
        let mut recovery_frame = false;
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            runtime.do_message_loop_work();
            while let Some(event) = producer.poll_web_event() {
                let text = format!("{event:?}");
                if text.contains("ContentProcessTerminated") {
                    terminated = true;
                    let _ = producer.navigate_to_url(&url);
                    let _ = producer.request_repaint();
                }
            }
            if terminated && producer.acquire_native_frame().is_some() {
                recovery_frame = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        crash_report = format!("\"first_frame\":{first},\"terminated_event\":{terminated},\"recovered_frame\":{recovery_frame}");
        receipt("crash", &crash_report);
        let _ = producer.close();
    }

    let elapsed = started.elapsed().as_secs_f64();
    receipt(
        "soak_end",
        &format!(
            "\"cycles\":{cycles},\"first_frames\":{first_frame_seen},\"failures\":{failures},\"seconds\":{elapsed:.1},\"rss_kib\":{},\"fds\":{},\"vram_mib\":{}",
            rss_kib(),
            fds(),
            vram_mib().unwrap_or(0)
        ),
    );
    receipt("soak_queue", &queue_report);
    receipt("crash", &crash_report);
    drop(runtime);
    if failures == 0 && first_frame_seen == cycles && crash_report != "null" {
        0
    } else if failures == 0 && first_frame_seen == cycles {
        0
    } else {
        1
    }
}

fn wait_frame(runtime: &CefRuntime, producer: &mut LinuxCefProducer, host: &HostWgpuContext, seconds: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        runtime.do_message_loop_work();
        while producer.poll_web_event().is_some() {}
        if let Some(native) = producer.acquire_native_frame() {
            if WgpuTextureImporter::import(native, host).is_ok() {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    false
}
