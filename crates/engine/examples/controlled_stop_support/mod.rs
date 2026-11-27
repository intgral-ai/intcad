//! Shared fixture and reporting for the controlled entry measurement examples.

use std::time::{Duration, Instant};

use serde_json::json;
use solidcraft_engine::Session;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const CUTS: [usize; 3] = [0, 8, 32];

/// The Review rebuild fixture, with prefixes of the same hole sequence for the smaller sizes.
pub fn plate(cuts: usize) -> Result<Session> {
    let mut session = Session::default();
    session.execute(
        "sheet.manage_rules",
        &json!({
            "name":"Budget", "thickness":"2 mm", "k_factor":"0.5", "bend_radius":"2 mm",
            "relief_width":"2 mm", "relief_depth":"1 mm", "active":true,
        }),
    )?;
    session.execute("sketch.create", &json!({"plane":"XY","name":"Base"}))?;
    session.execute("sketch.rectangle.two_point", &json!({"p0":[0,0],"p1":[200,120]}))?;
    session.execute("sketch.finish", &json!({}))?;
    session.execute("sheet.flange", &json!({"sketch":"Base"}))?;
    for i in 0..cuts {
        let x = 15 + (i % 8) * 24;
        let y = 20 + (i / 8) * 25;
        session.execute("solid.hole", &json!({"position":[x,y,2],"diameter":4}))?;
    }
    Ok(session)
}

pub fn header(metric: &str) {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| text.lines().find_map(|line| line.strip_prefix("model name")?.split_once(':').map(|(_, model)| model.trim().to_owned())));
    let logical = std::thread::available_parallelism().map_or(0, |n| n.get());
    println!(
        "{} / {}, {logical} logical CPUs, {}. Shared dev host; not CI.",
        std::env::consts::OS,
        std::env::consts::ARCH,
        cpu.as_deref().unwrap_or("unknown CPU")
    );
    println!("| {metric} | cold ms | median ms | worst warm ms | warm passes (ms) |");
    println!("|---|---:|---:|---:|---|");
}

pub fn sample(label: impl std::fmt::Display, mut pass: impl FnMut() -> Result<Duration>) -> Result<()> {
    let cold = ms(pass()?);
    let mut warm = [0.0; 5];
    for value in &mut warm {
        *value = ms(pass()?);
    }
    let mut ordered = warm;
    ordered.sort_by(f64::total_cmp);
    println!("| {label} | {cold:.3} | {:.3} | {:.3} | {warm:.3?} |", ordered[2], ordered[4]);
    Ok(())
}

#[allow(dead_code)] // The CLI and cancel-latency harnesses time nested phases separately.
pub fn elapsed(f: impl FnOnce() -> Result<()>) -> Result<Duration> {
    let start = Instant::now();
    f()?;
    Ok(start.elapsed())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000.0
}
