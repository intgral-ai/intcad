//! Phase 1/2 probe: real CEF accelerated off-screen rendering into a host wgpu
//! device, with **no window system window of its own**.
//!
//! The point of having no window is that it separates two questions the
//! official demo conflates: "does CEF paint into a GPU texture" and "does this
//! host present that texture on a surface". A frame here can only come from
//! `OnAcceleratedPaint` + a DMA-BUF import, never from a swapchain.
//!
//! Env:
//!   CEF_PATH        CEF binary distribution (required)
//!   PROBE_URL       page to load (default: fixtures/probe.html next to the crate)
//!   PROBE_SECS      how long to collect frames (default 6)
//!   PROBE_FRAMES    stop after this many imported frames (default 0 = no limit)
//!   PROBE_SIZE      WxH physical pixels (default 1024x720)
//!   PROBE_SCALE     scale factor (default 1.0)
//!   PROBE_ADAPTER   substring of the wgpu adapter name to pick
//!   PROBE_EXPECT    RRGGBB the page paints; centre pixel must match it
//!   PROBE_DUMP      write the last imported frame as a PPM here
//!   PROBE_INPUT     "click:X,Y" / "type:abc" / "wheel:-360" after load
//!   PROBE_SWITCHES  extra Chromium switches, comma separated, k=v allowed
//!   PROBE_SANDBOX   "sandboxed" to use CEF's process sandbox
//!   PROBE_TRACE     "1" to log every frame interval

use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use welding::linux_cef::{LinuxCefConfig, LinuxCefProducer};
use welding::wgpu;
use welding::{
    CefRuntime, CefRuntimeConfig, CefSandboxMode, CefSurfaceConfig, CefSurfaceProducer, EventModifiers, FocusDirection, HostWgpuContext, KeyEvent,
    KeyEventKind, MouseAction, MouseButton, MouseEvent, NativeFrame, WgpuTextureImporter,
};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn env_u32(name: &str, fallback: u32) -> u32 {
    env(name).and_then(|value| value.parse().ok()).unwrap_or(fallback)
}

fn receipt(tag: &str, body: &str) {
    println!("RECEIPT {{\"tag\":\"{tag}\",{body}}}");
    let _ = std::io::stdout().flush();
}

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let Some(cef_path) = env("CEF_PATH") else {
        eprintln!("CEF_PATH must point at a CEF binary distribution containing libcef.so");
        return 2;
    };
    let sandbox = match env("PROBE_SANDBOX").as_deref() {
        Some("sandboxed") => CefSandboxMode::Sandboxed,
        _ => CefSandboxMode::UnsandboxedTrustedContent,
    };

    // MUST be first: CEF re-executes this binary for renderer/GPU/utility roles.
    match CefRuntime::execute_process_from(Path::new(&cef_path), sandbox) {
        Ok(Some(code)) => return code,
        Ok(None) => {}
        Err(error) => {
            eprintln!("CEF subprocess probe failed: {error}");
            return 2;
        }
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let (width, height) = env("PROBE_SIZE")
        .and_then(|value| {
            let (w, h) = value.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((1024u32, 720u32));
    let scale = env("PROBE_SCALE").and_then(|value| value.parse::<f32>().ok()).unwrap_or(1.0);

    // ── host wgpu device ────────────────────────────────────────────────────
    let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    instance_descriptor.backends = wgpu::Backends::VULKAN;
    let instance = wgpu::Instance::new(instance_descriptor);
    let adapter = match env("PROBE_ADAPTER") {
        Some(wanted) => {
            let wanted = wanted.to_lowercase();
            pollster::block_on(instance.enumerate_adapters(wgpu::Backends::VULKAN))
                .into_iter()
                .find(|adapter| adapter.get_info().name.to_lowercase().contains(&wanted))
        }
        None => None,
    }
    .or_else(|| {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        }))
        .ok()
    });
    let Some(adapter) = adapter else {
        eprintln!("no wgpu Vulkan adapter matched");
        return 3;
    };
    let info = adapter.get_info();
    let available = adapter.features();
    let dma_buf = wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
    let mut required = wgpu::Features::empty();
    if available.contains(dma_buf) {
        required |= dma_buf;
    }
    receipt(
        "adapter",
        &format!(
            "\"name\":{:?},\"backend\":{:?},\"device_type\":{:?},\"driver\":{:?},\"driver_info\":{:?},\"dma_buf_feature\":{},\"extensions\":{:?}",
            info.name,
            info.backend,
            info.device_type,
            info.driver,
            info.driver_info,
            available.contains(dma_buf),
            info.driver_info
        ),
    );

    let device_descriptor = wgpu::DeviceDescriptor {
        label: Some("cef-osr-probe"),
        required_features: required,
        required_limits: wgpu::Limits::downlevel_defaults(),
        ..Default::default()
    };
    let (device, queue) = match welding::build_dmabuf_capable_device(&adapter, &device_descriptor) {
        Ok(pair) => pair,
        Err(error) => {
            eprintln!("device creation for DMA-BUF import failed: {error}");
            return 3;
        }
    };
    let host = HostWgpuContext::new(device, queue);
    receipt("host", &format!("\"interop\":{:?},\"device_features_dma_buf\":{}", host.backend, host.device.features().contains(dma_buf)));

    // ── CEF runtime ─────────────────────────────────────────────────────────
    let mut config = CefRuntimeConfig::new(&cef_path, sandbox);
    config.cache_path = Some(env("PROBE_CACHE").map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("cef-osr-probe-cache")));
    config.command_line_switches = vec![("no-first-run".into(), None), ("no-default-browser-check".into(), None)];
    if let Some(list) = env("PROBE_SWITCHES") {
        config.command_line_switches.extend(list.split(',').filter(|part| !part.is_empty()).map(|part| match part.split_once('=') {
            Some((key, value)) => (key.to_owned(), Some(value.to_owned())),
            None => (part.to_owned(), None),
        }));
    }
    receipt(
        "switches",
        &format!(
            "\"list\":{:?},\"display_set\":{},\"wayland_display_set\":{},\"session_type\":{:?}",
            config.command_line_switches,
            std::env::var_os("DISPLAY").is_some(),
            std::env::var_os("WAYLAND_DISPLAY").is_some(),
            std::env::var("XDG_SESSION_TYPE").ok()
        ),
    );

    let runtime = match CefRuntime::initialize(config) {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("CEF initialize failed: {error}");
            return 4;
        }
    };

    let default_url = format!("file://{}/fixtures/probe.html", env!("CARGO_MANIFEST_DIR"));
    let url = env("PROBE_URL").unwrap_or(default_url);
    let producer = match LinuxCefProducer::new(
        &runtime,
        LinuxCefConfig {
            surface: CefSurfaceConfig {
                initial_url: url.clone(),
                initial_size: dpi::PhysicalSize::new(width, height),
                scale_factor: scale,
                prefer_accelerated: true,
                ..Default::default()
            },
        },
    ) {
        Ok(producer) => producer,
        Err(error) => {
            eprintln!("CEF browser creation failed: {error}");
            return 5;
        }
    };
    receipt("browser", &format!("\"url\":{url:?},\"size\":\"{width}x{height}\",\"scale\":{scale},\"browser_id\":{}", producer.browser_id()));

    let mut producer = producer;
    // CEF learns the view size from `WasResized`; the constructor's
    // `initial_size` only records it. Without this, input is hit-tested against
    // a zero-sized view.
    if let Err(error) = producer.resize(dpi::PhysicalSize::new(width, height)) {
        eprintln!("resize failed: {error}");
    }
    // A windowless browser starts hidden; Chromium drops input for a hidden
    // widget, so the host has to say the view is on screen.
    if let Err(error) = producer.set_visible(true) {
        eprintln!("set_visible failed: {error}");
    }
    let seconds = env_u32("PROBE_SECS", 6);
    let want_frames = env_u32("PROBE_FRAMES", 0);
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds));
    let trace = env("PROBE_TRACE").as_deref() == Some("1");

    let mut frame_count = 0u32;
    let mut import_errors = 0u32;
    let mut load_end_at: Option<Instant> = None;
    let mut first_frame_at: Option<Instant> = None;
    let mut last_frame: Option<welding::ImportedTexture> = None;
    let mut frame_meta = String::new();
    let mut intervals: Vec<f64> = Vec::new();
    let mut last_frame_instant: Option<Instant> = None;
    let mut nav_events: Vec<String> = Vec::new();
    let mut input_sent = false;
    let mut popup_reported = false;
    let started = Instant::now();

    while Instant::now() < deadline {
        runtime.do_message_loop_work();
        while let Some(event) = producer.poll_web_event() {
            let line = format!("{event:?}");
            if line.starts_with("Navigation(TitleChanged") || line.contains("LoadEnd") {
                nav_events.push(line.chars().take(180).collect());
            }
            if line.contains("LoadEnd") && load_end_at.is_none() {
                load_end_at = Some(Instant::now());
            }
        }

        // CEF's windowless view drops input until it has painted its first
        // frame; gating on the load alone sends the gestures to a view that has
        // no render widget yet.
        if !input_sent && frame_count > 0 {
            if let Some(script) = env("PROBE_INPUT") {
                send_input(&mut producer, &script);
            }
            input_sent = true;
        }

        if let Some(rect) = producer.popup_rect() {
            if !popup_reported {
                popup_reported = true;
                receipt("popup", &format!("\"rect\":{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}", rect.x, rect.y, rect.width, rect.height));
            }
        }
        if let Some(native) = producer.acquire_native_frame() {
            let meta = describe_native_frame(&native);
            match WgpuTextureImporter::import(native, &host) {
                Ok(frame) => {
                    frame_count += 1;
                    let now = Instant::now();
                    if first_frame_at.is_none() {
                        first_frame_at = Some(now);
                        let first_ms = now.duration_since(started).as_secs_f64() * 1000.0;
                        receipt("first_frame_ms", &format!("{first_ms:.1}"));
                    }
                    if let Some(previous) = last_frame_instant {
                        intervals.push(now.duration_since(previous).as_secs_f64() * 1000.0);
                    }
                    last_frame_instant = Some(now);
                    if trace {
                        eprintln!(
                            "frame {frame_count} generation {} {}x{} {:?}",
                            frame.generation, frame.size.width, frame.size.height, frame.format
                        );
                    }
                    frame_meta = meta;
                    last_frame = Some(frame);
                    if want_frames > 0 && frame_count >= want_frames {
                        break;
                    }
                }
                Err(error) => {
                    import_errors += 1;
                    if import_errors <= 3 {
                        eprintln!("import failed: {error}");
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    receipt(
        "frames",
        &format!(
            "\"imported\":{frame_count},\"import_errors\":{import_errors},\"seconds\":{},\"interval_p50_ms\":{},\"interval_max_ms\":{}",
            seconds,
            percentile(&intervals, 0.5),
            intervals.iter().cloned().fold(0.0f64, f64::max)
        ),
    );
    receipt("native_frame", &frame_meta);
    if let Some(load_at) = load_end_at {
        receipt("load_end_ms", &format!("{:.1}", load_at.duration_since(started).as_secs_f64() * 1000.0));
    }
    for event in &nav_events {
        receipt("nav", &format!("\"event\":{}", json_string(event)));
    }

    let mut ok = frame_count > 0;
    if let Some(frame) = last_frame.as_ref() {
        match read_texture(&host, frame) {
            Ok(pixels) => {
                let stats = pixel_stats(&pixels, frame.size.width, frame.size.height);
                receipt(
                    "pixels",
                    &format!(
                        "\"format\":{:?},\"center\":[{},{},{},{}],\"nonzero_byte_ratio\":{:.4},\"distinct\":{}",
                        frame.format, stats.center[0], stats.center[1], stats.center[2], stats.center[3], stats.nonzero_ratio, stats.distinct
                    ),
                );
                if let Some(expected) = env("PROBE_EXPECT") {
                    let want = parse_hex(&expected);
                    let bgra = matches!(frame.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
                    let got =
                        if bgra { [stats.center[2], stats.center[1], stats.center[0]] } else { [stats.center[0], stats.center[1], stats.center[2]] };
                    let matches = want.iter().zip(got.iter()).all(|(a, b)| a.abs_diff(*b) <= 8);
                    receipt("expect", &format!("\"want\":{want:?},\"got\":{got:?},\"match\":{matches}"));
                    ok &= matches;
                }
                if let Some(path) = env("PROBE_DUMP") {
                    match write_ppm(&path, &pixels, frame.size.width, frame.size.height, frame.format) {
                        Ok(()) => receipt("dump", &format!("\"path\":{path:?}")),
                        Err(error) => {
                            eprintln!("dump failed: {error}");
                            ok = false;
                        }
                    }
                }
            }
            Err(error) => {
                eprintln!("readback failed: {error}");
                ok = false;
            }
        }
    }
    receipt("result", &format!("\"ok\":{ok}"));
    producer.close().ok();
    drop(runtime);
    if ok {
        0
    } else {
        1
    }
}

fn send_input(producer: &mut LinuxCefProducer, script: &str) {
    for command in script.split(';') {
        send_one(producer, command.trim());
    }
}

fn send_one(producer: &mut LinuxCefProducer, script: &str) {
    if script == "focus" {
        match producer.move_focus(FocusDirection::Forward) {
            Ok(()) => receipt("input", "\"focus\":true"),
            Err(error) => eprintln!("focus failed: {error}"),
        }
    } else if let Some(rest) = script.strip_prefix("wait:") {
        let millis: u64 = rest.trim().parse().unwrap_or(200);
        std::thread::sleep(Duration::from_millis(millis));
    } else if let Some(rest) = script.strip_prefix("ime:") {
        // What a real input method produces: a composition, then the commit.
        // The page's composition handlers turn it into navigation events, so it
        // is checkable without pixels.
        let _ = producer.ime_set_composition("ni hao", (3, 6));
        std::thread::sleep(Duration::from_millis(150));
        let _ = producer.ime_commit_text(rest);
        receipt("input", &format!("\"ime\":{}", json_string(rest)));
    } else if let Some(rest) = script.strip_prefix("wheel:") {
        let delta: i32 = rest.trim().parse().unwrap_or(0);
        let _ = producer.send_mouse_input(MouseEvent {
            x: 512,
            y: 360,
            button: MouseButton::Left,
            action: MouseAction::WheelScrolled { delta_x: 0, delta_y: delta },
            modifiers: EventModifiers::default(),
        });
        receipt("input", &format!("\"wheel\":{delta}"));
    } else if let Some(rest) = script.strip_prefix("click:") {
        if let Some((x, y)) = rest.split_once(',') {
            let (x, y): (f32, f32) = (x.trim().parse().unwrap_or(0.0), y.trim().parse().unwrap_or(0.0));
            // The sequence welding's own battery uses: focus, hover, press,
            // release. A windowless browser that was never focused routes
            // cursor and key updates nowhere.
            if let Err(error) = producer.move_focus(FocusDirection::Forward) {
                eprintln!("focus failed: {error}");
            }
            for action in [MouseAction::Moved, MouseAction::Pressed, MouseAction::Released] {
                let result = producer.send_mouse_input(MouseEvent {
                    action,
                    button: MouseButton::Left,
                    x: x.round() as i32,
                    y: y.round() as i32,
                    modifiers: EventModifiers::default(),
                });
                if let Err(error) = result {
                    eprintln!("mouse input failed: {error}");
                }
            }
            receipt("input", &format!("\"click\":[{x},{y}]"));
        }
    } else if let Some(text) = script.strip_prefix("type:") {
        for ch in text.chars() {
            let event = KeyEvent {
                kind: KeyEventKind::Char,
                character: Some(ch),
                windows_key_code: 0,
                native_key_code: 0,
                modifiers: EventModifiers::default(),
            };
            if let Err(error) = producer.send_keyboard_input(event) {
                eprintln!("key input failed: {error}");
            }
        }
        receipt("input", &format!("\"type\":{}", json_string(text)));
    }
}

fn describe_native_frame(frame: &NativeFrame) -> String {
    match frame {
        NativeFrame::DmaBufImage(image) => {
            let planes: Vec<String> = image
                .planes()
                .iter()
                .map(|plane| {
                    format!(
                        "{{\"buffer\":{},\"offset\":{},\"size\":{},\"stride\":{}}}",
                        plane.buffer_index(),
                        plane.offset(),
                        plane.size(),
                        plane.stride()
                    )
                })
                .collect();
            format!(
                "\"kind\":\"dmabuf\",\"size\":\"{}x{}\",\"format\":{:?},\"drm_format\":\"0x{:08x}\",\"modifier\":\"0x{:016x}\",\"buffers\":{},\"planes\":[{}]",
                image.size().width,
                image.size().height,
                image.format(),
                image.drm_format(),
                image.modifier(),
                image.planes().len(),
                planes.join(",")
            )
        }
        other => format!("\"kind\":{:?}", other.kind()),
    }
}

struct PixelStats {
    center: [u8; 4],
    nonzero_ratio: f64,
    distinct: usize,
}

fn pixel_stats(pixels: &[u8], width: u32, height: u32) -> PixelStats {
    let row = width as usize * 4;
    let center_index = (height as usize / 2) * row + (width as usize / 2) * 4;
    let mut center = [0u8; 4];
    if center_index + 4 <= pixels.len() {
        center.copy_from_slice(&pixels[center_index..center_index + 4]);
    }
    let nonzero = pixels.iter().filter(|byte| **byte != 0).count();
    let mut seen = std::collections::HashSet::new();
    for chunk in pixels.chunks_exact(4).step_by(97) {
        seen.insert([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    PixelStats { center, nonzero_ratio: nonzero as f64 / pixels.len().max(1) as f64, distinct: seen.len() }
}

fn read_texture(host: &HostWgpuContext, frame: &welding::ImportedTexture) -> Result<Vec<u8>, String> {
    let width = frame.size.width;
    let height = frame.size.height;
    let unpadded = (width * 4) as usize;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded.div_ceil(align) * align;
    let buffer = host.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe-readback"),
        size: (padded * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = host.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("probe") });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: &frame.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded as u32), rows_per_image: Some(height) },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    host.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    host.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None }).map_err(|error| error.to_string())?;
    match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => return Err(format!("map_async: {error}")),
        Err(error) => return Err(format!("map_async timeout: {error}")),
    }
    let mapped = slice.get_mapped_range().map_err(|error| error.to_string())?;
    let mut out = Vec::with_capacity(unpadded * height as usize);
    for row in 0..height as usize {
        let start = row * padded;
        out.extend_from_slice(&mapped[start..start + unpadded]);
    }
    drop(mapped);
    buffer.unmap();
    Ok(out)
}

fn write_ppm(path: &str, pixels: &[u8], width: u32, height: u32, format: wgpu::TextureFormat) -> Result<(), String> {
    let bgra = matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
    let mut file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    use std::io::Write as _;
    write!(file, "P6\n{width} {height}\n255\n").map_err(|error| error.to_string())?;
    let mut rgb = Vec::with_capacity(pixels.len() / 4 * 3);
    for chunk in pixels.chunks_exact(4) {
        let (r, g, b) = if bgra { (chunk[2], chunk[1], chunk[0]) } else { (chunk[0], chunk[1], chunk[2]) };
        rgb.extend_from_slice(&[r, g, b]);
    }
    file.write_all(&rgb).map_err(|error| error.to_string())
}

fn percentile(values: &[f64], p: f64) -> String {
    if values.is_empty() {
        return "null".into();
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    format!("{:.2}", sorted[index])
}

fn parse_hex(value: &str) -> [u8; 3] {
    let clean = value.trim_start_matches('#');
    let parse = |range: std::ops::Range<usize>| u8::from_str_radix(clean.get(range).unwrap_or("00"), 16).unwrap_or(0);
    [parse(0..2), parse(2..4), parse(4..6)]
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if (other as u32) < 0x20 => out.push(' '),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}
