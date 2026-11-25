//! The stress estimate on the model (PIL-294): the result `inspect.stress_estimate` left
//! (`solidcraft_engine::fea`) painted over its body as a colour map, with a legend that says it
//! is an estimate and lists every assumption. The colours run blue to yellow with lightness
//! rising all the way (no red, nothing that reads as a verdict, readable without hue), cut to the
//! stretch that stands out from the theme's view background ([`Scale::for_view`]).

use egui::{Color32, Pos2, Rect, Shape, Stroke, vec2};
use serde_json::json;
use solidcraft_engine::fea::{self, Field, Quantity};
use solidcraft_engine::geom::prelude::*;

use crate::SolveApp;
use crate::viewport::Proj;

/// The full ramp's stops (sRGB 0..1): navy, blue, light blue, light khaki, light yellow. Blue to
/// yellow keeps apart in red-green colour blindness, and the lightness rises at every step, so the
/// order reads even with no hue at all. A dark view takes the light blue to yellow part, a light
/// view the navy to blue part (see [`Scale::for_view`]): both still change hue along the way.
const RAMP: [[f32; 3]; 5] = [
    [16.0 / 255.0, 38.0 / 255.0, 102.0 / 255.0],
    [40.0 / 255.0, 90.0 / 255.0, 190.0 / 255.0],
    [110.0 / 255.0, 160.0 / 255.0, 235.0 / 255.0],
    [215.0 / 255.0, 200.0 / 255.0, 120.0 / 255.0],
    [252.0 / 255.0, 232.0 / 255.0, 80.0 / 255.0],
];

/// The darkest the headlight shades a face turned edge-on (1 faces the eye).
const SHADE_MIN: f32 = 0.8;

/// The least contrast (WCAG ratio) any colour of the scale has with the view's background.
pub(crate) const MIN_CONTRAST: f32 = 3.0;

/// The full ramp at `k` (0..1), straight between its stops.
fn ramp(k: f32) -> [f32; 3] {
    let k = if k.is_finite() { k.clamp(0.0, 1.0) } else { 0.0 };
    let x = k * (RAMP.len() - 1) as f32;
    let i = (x.floor() as usize).min(RAMP.len() - 2);
    let (a, b, u) = (RAMP[i], RAMP[i + 1], x - i as f32);
    [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u]
}

fn rgb(c: [f32; 3], shade: f32) -> Color32 {
    crate::theme::srgb([c[0] * shade, c[1] * shade, c[2] * shade])
}

/// WCAG relative luminance of an sRGB colour.
pub(crate) fn luminance(c: Color32) -> f32 {
    let ch = |v: u8| {
        let v = f32::from(v) / 255.0;
        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
}

/// WCAG contrast ratio of two colours (1 to 21).
pub(crate) fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The colour scale for one view background: the longest stretch `[from, to]` of the full ramp
/// whose every colour, lit full and shaded darkest, has [`MIN_CONTRAST`] with the background's
/// top, middle and bottom. On a dark view that is the light end, on a light view the dark end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scale {
    pub from: f32,
    pub to: f32,
    /// Every colour has [`MIN_CONTRAST`] with the view. False only on a mid-tone view, where no
    /// colour of the ramp can (the built-in SolidCraft 95 teal).
    pub stands_out: bool,
}

impl Scale {
    pub(crate) fn for_view(top: Color32, bottom: Color32) -> Scale {
        const N: usize = 400;
        let middle = top.lerp_to_gamma(bottom, 0.5);
        let ok = |k: f32| {
            let c = ramp(k);
            [1.0, SHADE_MIN].iter().all(|shade| [top, middle, bottom].iter().all(|bg| contrast(rgb(c, *shade), *bg) >= MIN_CONTRAST))
        };
        let (mut best, mut run): (Option<(usize, usize)>, Option<usize>) = (None, None);
        for i in 0..=N {
            if ok(i as f32 / N as f32) {
                let start = *run.get_or_insert(i);
                if best.is_none_or(|(a, b)| i - start > b - a) {
                    best = Some((start, i));
                }
            } else {
                run = None;
            }
        }
        match best {
            Some((a, b)) => Scale { from: a as f32 / N as f32, to: b as f32 / N as f32, stands_out: true },
            // Nothing stands out (a mid-grey view): the end of the ramp farther from it.
            None => {
                let far = if contrast(rgb(ramp(0.0), 1.0), middle) >= contrast(rgb(ramp(1.0), 1.0), middle) { 0.0 } else { 1.0 };
                Scale { from: far, to: far, stands_out: false }
            }
        }
    }

    /// The colour of `k` (0 low .. 1 high), lit by `shade` (SHADE_MIN .. 1).
    pub(crate) fn color(self, k: f32, shade: f32) -> Color32 {
        let k = if k.is_finite() { k.clamp(0.0, 1.0) } else { 0.0 };
        rgb(ramp(self.from + (self.to - self.from) * k), shade.clamp(SHADE_MIN, 1.0))
    }
}

/// A fixed light (from above, front left), so a face's colour is the same from every side and is
/// worked out once per result, not every frame.
const LIGHT: [f64; 3] = [-0.3, -0.5, 0.81];

/// The value at each corner, the top of the scale, and the yield strength's place on it (stress
/// against yield only).
fn values(f: &Field) -> (Vec<f32>, f32, Option<f32>) {
    match f.quantity {
        Quantity::Stress => {
            let top = match f.scale {
                fea::ScaleMode::Yield => f.yield_mpa as f32,
                fea::ScaleMode::Auto => f.max_stress as f32,
            };
            (f.stress.clone(), top, Some(f.yield_mpa as f32))
        }
        Quantity::Deflection => (f.disp.iter().map(|u| (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt() as f32).collect(), f.max_disp as f32, None),
    }
}

/// What the overlay keeps between frames: each corner's colour (per result, theme and dimming)
/// and the projected, sorted mesh (per camera too), so a still view costs nothing and an orbit
/// only projects and sorts.
#[derive(Default)]
struct Cache {
    colours_key: u64,
    colours: std::sync::Arc<Vec<Color32>>,
    mesh_key: u64,
    mesh: std::sync::Arc<egui::Mesh>,
    /// What it last fingerprinted (the body's mesh, the session's and the result's revisions),
    /// and the fingerprint.
    shape: (u64, u64),
    /// When the running estimate was first seen (app seconds), for its timer.
    running_since: Option<(String, f64)>,
}

thread_local! {
    static CACHE: std::cell::RefCell<Cache> = std::cell::RefCell::new(Cache::default());
}

fn hash_of(parts: &[u64]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

/// Is the result for a body whose shape has changed since (or that is gone)?
fn out_of_date(app: &SolveApp, f: &Field) -> bool {
    shape_changed(app, &f.body, f.fingerprint)
}

/// Has body `name` changed shape since `fingerprint` was taken (or is it gone)?
fn shape_changed(app: &SolveApp, name: &str, fingerprint: u64) -> bool {
    let Ok(st) = app.session.world_state() else { return false };
    let Some(body) = st.bodies.iter().find(|b| b.name == name) else { return true };
    let mesh = body.mesh();
    let key = hash_of(&[std::sync::Arc::as_ptr(&mesh) as usize as u64, app.session.revision, fea::revision()]);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.shape.0 != key {
            c.shape = (key, fea::fingerprint(body));
        }
        c.shape.1 != fingerprint
    })
}

/// What an agent's stress estimate would run, for its approval card: the setup in plain words
/// (checked on a copy of the design, solving nothing), or why it can't run, so a bad setup goes
/// back to the agent before anyone is asked. `None` for any other request.
pub(crate) fn approval(app: &SolveApp, method: &str, params: &serde_json::Value) -> Option<Result<String, String>> {
    if method != "engine.execute" {
        return None;
    }
    let id = params.get("command").and_then(serde_json::Value::as_str)?;
    if id != "inspect.stress_estimate" {
        return None;
    }
    let mut p = params.get("params").cloned().unwrap_or_else(|| json!({}));
    let o = p.as_object_mut()?;
    o.insert("dry_run".into(), json!(true));
    let mut copy = app.session.scratch();
    Some(match copy.execute(id, &p) {
        Ok(v) => {
            // What the run will take comes first: it is what the user weighs before allowing it.
            let lines: Vec<String> = predicted_line(&v)
                .into_iter()
                .chain(
                    v["setup"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .chain(v["warnings"].as_array().into_iter().flatten())
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string),
                )
                .collect();
            Ok(lines.join("\n"))
        }
        Err(e) => {
            // An older setup's preview would show faces this request does not use.
            fea::clear_preview(&app.session.document_id);
            Err(e.to_string())
        }
    })
}

/// `from`, or `from – to` when the third mesh may follow and it says something else.
fn span(from: String, to: Option<String>) -> String {
    match to {
        Some(to) if to != from => crate::i18n::format("ui.stress_estimate_card_range", &[("from", &from), ("to", &to)]),
        _ => from,
    }
}

/// What a run will take, from a dry run's `predicted` (the part's size, before meshing), for the
/// approval card (A13): each figure from the runs likely made to those with the mesh check's third
/// mesh (`most`), when it may follow. `None` when the dry run could not say.
pub(crate) fn predicted_line(v: &serde_json::Value) -> Option<String> {
    let p = v.get("predicted")?;
    let figures = |p: &serde_json::Value| {
        let number = |key: &str| p.get(key).and_then(serde_json::Value::as_f64);
        Some((number("nodes")?, number("memory_gb")? * 1.0e9, number("seconds")?, p.get("runs")?.as_u64()?))
    };
    let (nodes, bytes, seconds, runs) = figures(p)?;
    let most = p.get("most").and_then(figures);
    let up_to = |words: &dyn Fn((f64, f64, f64, u64)) -> String| most.map(words);
    let runs = if runs == 1 && most.is_none() {
        crate::i18n::tr("ui.stress_estimate_card_one_run").to_string()
    } else {
        crate::i18n::format("ui.stress_estimate_card_runs", &[("n", &span(runs.to_string(), up_to(&|m| m.3.to_string())))])
    };
    Some(crate::i18n::format(
        "ui.stress_estimate_card_before",
        &[
            ("nodes", &span(about_nodes(nodes), up_to(&|m| about_nodes(m.0)))),
            ("memory", &span(memory_words(bytes), up_to(&|m| memory_words(m.1)))),
            ("time", &span(time_words(seconds), up_to(&|m| time_words(m.2)))),
            ("runs", &runs),
        ],
    ))
}

/// What a run will still take once it has meshed the part and counted its nodes (A13), for the
/// card the user allowed it from.
pub(crate) fn counted_line(f: &fea::Forecast) -> String {
    let model = match f.elements {
        fea::Elements::Shell => crate::i18n::tr("ui.stress_estimate_model_shell"),
        fea::Elements::Solid => crate::i18n::tr("ui.stress_estimate_model_solid"),
    };
    crate::i18n::format(
        "ui.stress_estimate_card_counted",
        &[
            ("run", &f.run.to_string()),
            ("runs", &span(f.runs.to_string(), Some(f.runs_most.to_string()))),
            ("model", model),
            ("nodes", &crate::frame_rate::grouped(f.nodes as u64)),
            ("memory", &memory_words(f.bytes)),
            ("time", &span(time_words(f.seconds_left), Some(time_words(f.seconds_most)))),
        ],
    )
}

/// A predicted count to two significant figures, grouped: "95 000".
fn about_nodes(n: f64) -> String {
    if !(n.is_finite() && n >= 0.0) {
        return "?".into();
    }
    let digits = if n >= 100.0 { n.log10().floor() as i32 - 1 } else { 0 };
    let k = 10f64.powi(digits);
    crate::frame_rate::grouped(((n / k).round() * k) as u64)
}

/// Memory to two significant figures: "3.1 GB", "12 GB", "480 MB".
fn memory_words(bytes: f64) -> String {
    if !(bytes.is_finite() && bytes >= 0.0) {
        return "?".into();
    }
    let two = |x: f64| if x >= 9.95 { format!("{x:.0}") } else { format!("{x:.1}").trim_end_matches(".0").to_string() };
    if bytes >= 1.0e9 { format!("{} GB", two(bytes / 1.0e9)) } else { format!("{} MB", two((bytes / 1.0e6).max(1.0))) }
}

/// A predicted time, as roughly as it is known: "under a second", "8 s", "45 s", "2.5 min",
/// "25 min", "1.4 h".
fn time_words(seconds: f64) -> String {
    let f = |id: &str, n: String| crate::i18n::format(id, &[("n", &n)]);
    if !(seconds.is_finite() && seconds >= 0.0) {
        return "?".into();
    }
    if seconds < 1.0 {
        return crate::i18n::tr("ui.stress_estimate_time_under_second").to_string();
    }
    if seconds < 90.0 {
        let k = if seconds < 10.0 { 1.0 } else { 5.0 };
        return f("ui.stress_estimate_time_seconds", format!("{}", ((seconds / k).round() * k).max(1.0)));
    }
    let minutes = seconds / 60.0;
    if minutes < 90.0 {
        let m = if minutes < 10.0 { (minutes * 2.0).round() / 2.0 } else { minutes.round() };
        return f("ui.stress_estimate_time_minutes", format!("{m}"));
    }
    f("ui.stress_estimate_time_hours", format!("{}", (minutes / 6.0).round() / 10.0))
}

/// Open the chat with a request to start from: the ribbon's and the palette's Stress Estimate.
pub(crate) fn ask_in_chat(app: &mut SolveApp) {
    app.agent_chat.open = true;
    let starter = app.tr("ui.stress_estimate_starter").to_string();
    if app.agent_chat.input.trim().is_empty() {
        app.agent_chat.input = starter.clone();
    }
    // The Assistant's web page keeps its own message box: the conversation says what to ask.
    app.agent_chat.notice(starter, false);
}

/// Paint the result over the model, back to front, with what was applied on it, the status of a
/// running estimate and the legend in `free`.
pub(crate) fn paint(app: &mut SolveApp, ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj, free: Rect) {
    let doc = app.session.document_id.clone();
    let running = fea::progress().filter(|p| p.document_id == doc);
    status(app, ui, running.as_ref(), free);
    // A setup checked but not solved: its supports and loads, in place of any older result.
    if running.is_none()
        && let Some(pv) = fea::preview(&doc)
        && !shape_changed(app, &pv.body, pv.fingerprint)
    {
        setup_marks(painter, proj, &pv.loads, &pv.supports, pv.size, true);
        let t = crate::theme::Tokens::get();
        pill(painter, free.left_bottom() + vec2(t.space_12, -t.space_12), egui::Align2::LEFT_BOTTOM, app.tr("ui.stress_estimate_preview"));
        return;
    }
    let Some(f) = fea::shown(&doc) else { return };
    if f.hidden {
        return;
    }
    let t = crate::theme::Tokens::get();
    let stale = out_of_date(app, &f);
    let dim = stale || running.is_some();
    let scale = Scale::for_view(t.viewport_top, t.viewport_bottom);
    let (vals, top, _) = values(&f);
    let top = if top > 0.0 { top } else { 1.0 };
    let deform = if f.quantity == Quantity::Deflection { f.deform_scale } else { 0.0 };
    // Each corner's colour: once per result, theme and dimming.
    let colours_key = hash_of(&[fea::revision(), u64::from(dim), scale.from.to_bits().into(), scale.to.to_bits().into(), u64::from(top.to_bits())]);
    let colours = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.colours_key != colours_key || c.colours.len() != f.positions.len() {
            let light = LIGHT;
            let out: Vec<Color32> = (0..f.positions.len())
                .map(|i| {
                    let v = vals.get(i).copied().unwrap_or(0.0);
                    let n = f.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
                    let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).abs().min(1.0) as f32;
                    let c = scale.color(v / top, SHADE_MIN + (1.0 - SHADE_MIN) * lit);
                    if dim { c.gamma_multiply(0.35) } else { c }
                })
                .collect();
            c.colours = std::sync::Arc::new(out);
            c.colours_key = colours_key;
        }
        std::sync::Arc::clone(&c.colours)
    });
    // The projected mesh: once per camera.
    let vp: Vec<u64> = (0..4).flat_map(|r| (0..4).map(move |k| (r, k))).map(|(r, k)| proj.vp.0[r][k].to_bits()).collect();
    let mut parts = vec![
        colours_key,
        u64::from(proj.rect.min.x.to_bits()),
        u64::from(proj.rect.max.x.to_bits()),
        u64::from(proj.rect.min.y.to_bits()),
        u64::from(proj.rect.max.y.to_bits()),
    ];
    parts.extend(vp);
    let mesh_key = hash_of(&parts);
    let cached = CACHE.with(|c| {
        let c = c.borrow();
        (c.mesh_key == mesh_key).then(|| std::sync::Arc::clone(&c.mesh))
    });
    let mesh = match cached {
        Some(m) => m,
        None => {
            let m = std::sync::Arc::new(build_mesh(&f, &colours, proj, deform));
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                c.mesh = std::sync::Arc::clone(&m);
                c.mesh_key = mesh_key;
            });
            m
        }
    };
    painter.add(Shape::Mesh(mesh));
    if !dim {
        let away = f
            .away_stress
            .filter(|_| f.quantity == Quantity::Stress)
            .map(|(v, at)| (at, crate::i18n::format("ui.stress_estimate_away_marker", &[("value", &format!("{v:.0}"))])));
        markers(painter, proj, &f, deform, away);
    }
    legend(app, ui, &f, top, stale, running.is_some(), free);
}

/// The result's triangles facing the eye, back to front, in screen space.
fn build_mesh(f: &Field, colours: &[Color32], proj: &Proj, deform: f64) -> egui::Mesh {
    let eye = proj.cam.eye();
    let back = proj.cam.back();
    let perspective = proj.cam.fov > 0.0;
    let world: Vec<solidcore_record::Vec3> = f
        .positions
        .iter()
        .zip(f.disp.iter().chain(std::iter::repeat(&[0.0; 3])))
        .map(|(p, u)| solidcore_record::Vec3::new(p[0] + deform * u[0], p[1] + deform * u[1], p[2] + deform * u[2]))
        .collect();
    let screen: Vec<Option<Pos2>> = world.iter().map(|p| proj.to_screen(*p).filter(|s| s.x.is_finite() && s.y.is_finite())).collect();
    let mut order: Vec<(f64, usize)> = Vec::with_capacity(f.triangles.len() / 2);
    for (k, tri) in f.triangles.iter().enumerate() {
        let corner = |i: u32| world.get(i as usize).copied();
        let (Some(a), Some(b), Some(c)) = (corner(tri[0]), corner(tri[1]), corner(tri[2])) else { continue };
        if tri.iter().any(|i| screen.get(*i as usize).copied().flatten().is_none()) {
            continue;
        }
        let n = (b - a).cross(c - a);
        let centre = (a + b + c) * (1.0 / 3.0);
        let toward = if perspective { eye - centre } else { back };
        if n.dot(toward) <= 0.0 {
            continue;
        }
        order.push((centre.dot(back), k));
    }
    order.sort_unstable_by(|x, y| x.0.total_cmp(&y.0));
    // A node without a value (never expected) is drawn in the dim text colour.
    let missing = crate::theme::Tokens::get().text_dim;
    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve(order.len() * 3);
    mesh.indices.reserve(order.len() * 3);
    for (_, k) in order {
        let Some(tri) = f.triangles.get(k) else { continue };
        let base = mesh.vertices.len() as u32;
        for i in tri {
            let p = screen.get(*i as usize).copied().flatten().unwrap_or(Pos2::ZERO);
            mesh.colored_vertex(p, colours.get(*i as usize).copied().unwrap_or(missing));
        }
        mesh.add_triangle(base, base + 1, base + 2);
    }
    mesh
}

/// A label on a dark pill, so it reads on any colour of the map.
fn pill(painter: &egui::Painter, at: Pos2, anchor: egui::Align2, text: &str) {
    let t = crate::theme::Tokens::get();
    let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(t.font_small), t.text);
    let rect = anchor.anchor_size(at, galley.size()).expand(3.0);
    painter.rect_filled(rect, 3.0, t.overlay);
    painter.galley(rect.min + vec2(3.0, 3.0), galley, t.text);
}

/// The loads as arrows into their faces, the supports as held marks, the highest value, and the
/// highest stress away from supports and loads when the highest sits at one (`away`).
fn markers(painter: &egui::Painter, proj: &Proj, f: &Field, deform: f64, away: Option<([f64; 3], String)>) {
    let t = crate::theme::Tokens::get();
    let ink = t.text;
    let size = {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in &f.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt()
    };
    setup_marks(painter, proj, &f.loads, &f.supports, size, false);
    let v = |p: [f64; 3]| solidcore_record::Vec3::new(p[0], p[1], p[2]);
    let (at, text) = match f.quantity {
        Quantity::Stress => (f.max_stress_at, format!("max {:.0} MPa", f.max_stress)),
        Quantity::Deflection => (f.max_disp_at, format!("max {:.2} mm", f.max_disp)),
    };
    // The deflection's place moves with the exaggerated shape (it is already the deformed point).
    let at = if f.quantity == Quantity::Deflection && deform > 0.0 {
        let u = fea::model::sub(at, f.max_disp_at);
        fea::model::add(f.max_disp_at, fea::model::mul(u, deform))
    } else {
        at
    };
    if let Some(c) = proj.to_screen(v(at)) {
        painter.circle_stroke(c, 7.0, Stroke::new(2.0, ink));
        painter.circle_filled(c, 2.5, ink);
        pill(painter, c + vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, &text);
    }
    // The peak away from supports and loads: a diamond, its label below so the two never overlap.
    if let Some((at, text)) = away
        && let Some(c) = proj.to_screen(v(at))
    {
        let d = 7.0;
        painter.add(Shape::closed_line(vec![c + vec2(0.0, -d), c + vec2(d, 0.0), c + vec2(0.0, d), c + vec2(-d, 0.0)], Stroke::new(2.0, ink)));
        painter.circle_filled(c, 2.5, ink);
        pill(painter, c + vec2(10.0, 10.0), egui::Align2::LEFT_TOP, &text);
    }
}

/// The loads as arrows into their faces (their length from the body's `size`) and the supports as
/// held marks, on the undeformed faces; `labels` also names each support's kind (the preview).
fn setup_marks(painter: &egui::Painter, proj: &Proj, loads: &[fea::Marker], supports: &[fea::Marker], size: f64, labels: bool) {
    let t = crate::theme::Tokens::get();
    let ink = t.text;
    let v = |p: [f64; 3]| solidcore_record::Vec3::new(p[0], p[1], p[2]);
    for l in loads {
        let Some(d) = l.dir else { continue };
        let head = v(l.at);
        let tail = head - v(d) * (0.18 * size);
        let (Some(h), Some(tl)) = (proj.to_screen(head), proj.to_screen(tail)) else { continue };
        let stroke = Stroke::new(2.5, ink);
        painter.line_segment([tl, h], stroke);
        let along = (h - tl).normalized();
        let side = vec2(-along.y, along.x);
        painter.add(Shape::convex_polygon(vec![h, h - along * 11.0 + side * 5.0, h - along * 11.0 - side * 5.0], ink, Stroke::NONE));
        pill(painter, tl, egui::Align2::CENTER_BOTTOM, &l.text);
    }
    for s in supports {
        let Some(c) = proj.to_screen(v(s.at)) else { continue };
        // A held mark: a triangle standing on a hatched ground line.
        let stroke = Stroke::new(1.8, ink);
        let (a, b, top) = (c + vec2(-7.0, 10.0), c + vec2(7.0, 10.0), c);
        painter.add(Shape::convex_polygon(vec![top, b, a], t.overlay, stroke));
        painter.line_segment([c + vec2(-10.0, 10.0), c + vec2(10.0, 10.0)], stroke);
        for k in 0..4 {
            let x = -9.0 + 6.0 * k as f32;
            painter.line_segment([c + vec2(x, 10.0), c + vec2(x - 4.0, 15.0)], Stroke::new(1.2, ink));
        }
        if labels && !s.text.is_empty() {
            pill(painter, c + vec2(0.0, 18.0), egui::Align2::CENTER_TOP, &s.text);
        }
    }
}

/// A running estimate: what it does, for how long, and Stop.
fn status(app: &mut SolveApp, ui: &mut egui::Ui, running: Option<&fea::Progress>, free: Rect) {
    let now = app.now;
    let since = CACHE.with(|c| {
        let mut c = c.borrow_mut();
        match running {
            Some(p) => match &c.running_since {
                Some((doc, at)) if *doc == p.document_id => *at,
                _ => {
                    c.running_since = Some((p.document_id.clone(), now));
                    now
                }
            },
            None => {
                c.running_since = None;
                now
            }
        }
    });
    let Some(p) = running else { return };
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
    let stage = match p.stage {
        fea::Stage::Meshing => app.tr("ui.stress_estimate_stage_meshing"),
        fea::Stage::Solving => app.tr("ui.stress_estimate_stage_solving"),
    };
    let model = match p.elements {
        fea::Elements::Shell => app.tr("ui.stress_estimate_model_shell"),
        fea::Elements::Solid => app.tr("ui.stress_estimate_model_solid"),
    };
    let text = crate::i18n::format(
        "ui.stress_estimate_running",
        &[
            ("stage", stage),
            ("model", model),
            ("run", &p.run.to_string()),
            ("runs", &p.runs.to_string()),
            ("seconds", &format!("{:.0}", (now - since).max(0.0))),
        ],
    );
    let t = crate::theme::Tokens::get();
    let mut stop = false;
    let container = crate::ui_kit::Container::new(ui.ctx(), crate::ui_kit::Surface::ViewportStatus);
    container
        .area(egui::Id::new("sc_stress_estimate_status"))
        .order(egui::Order::Background)
        .movable(false)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(free.center_bottom() + vec2(0.0, -64.0))
        .show(ui.ctx(), |ui| {
            let frame = container.frame(ui.ctx(), t.overlay, crate::frame::Side::Left).inner_margin(t.space_8);
            container.show_frame(ui, frame, free, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(egui::RichText::new(text).size(t.font_label).color(t.text));
                    if crate::ui_kit::button(ui, app.tr("ui.stress_estimate_stop"), crate::ui_kit::Button::Small).clicked() {
                        stop = true;
                    }
                });
            });
        });
    if stop {
        fea::cancel();
    }
}

fn legend(app: &mut SolveApp, ui: &mut egui::Ui, f: &Field, top: f32, stale: bool, running: bool, free: Rect) {
    let t = crate::theme::Tokens::get();
    let colours = Scale::for_view(t.viewport_top, t.viewport_bottom);
    let (quantity, unit, digits) = match f.quantity {
        Quantity::Stress => (app.tr("ui.stress_estimate_von_mises"), "MPa", 0usize),
        Quantity::Deflection => (app.tr("ui.stress_estimate_deflection"), "mm", 3usize),
    };
    let quantity = quantity.to_string();
    let mut chosen: Option<serde_json::Value> = None;
    let container = crate::ui_kit::Container::new(ui.ctx(), crate::ui_kit::Surface::ViewportStatus);
    container
        .area(egui::Id::new("sc_stress_estimate_legend"))
        .order(egui::Order::Background)
        .movable(false)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(free.left_bottom() + vec2(t.space_12, -t.space_12))
        .show(ui.ctx(), |ui| {
            let frame = container.frame(ui.ctx(), t.overlay, crate::frame::Side::Left).inner_margin(t.space_8);
            container.show_frame(ui, frame, free, |ui| {
                ui.set_max_width(320.0);
                ui.label(egui::RichText::new(app.tr("ui.stress_estimate_title")).strong().size(t.font_label).color(t.text));
                ui.label(egui::RichText::new(app.tr("ui.stress_estimate_badge")).size(t.font_small).color(t.text));
                ui.label(egui::RichText::new(&f.heading).size(t.font_small).color(t.text_dim));
                if stale {
                    ui.label(egui::RichText::new(app.tr("ui.stress_estimate_out_of_date")).strong().size(t.font_small).color(t.text));
                } else if running {
                    ui.label(egui::RichText::new(app.tr("ui.stress_estimate_rerunning")).size(t.font_small).color(t.text));
                }
                for w in &f.warnings {
                    ui.label(egui::RichText::new(format!("⚠ {w}")).size(t.font_small).color(t.text));
                }
                if let Some(e) = &f.equilibrium {
                    ui.label(egui::RichText::new(e).size(t.font_small).color(t.text));
                }
                if let Some(m) = &f.mesh_check {
                    ui.label(egui::RichText::new(format!("{}: {m}", app.tr("ui.stress_estimate_mesh_check"))).size(t.font_small).color(t.text));
                }
                ui.add_space(t.space_4);
                let shape = if f.quantity == Quantity::Deflection {
                    crate::i18n::format("ui.stress_estimate_exaggerated", &[("scale", &format!("{:.0}", f.deform_scale))])
                } else {
                    String::new()
                };
                ui.label(egui::RichText::new(format!("{quantity} ({unit}) {shape}")).size(t.font_small).color(t.text));
                // The colour bar, zero to the top of the scale, with five values under it.
                let width = 300.0;
                let (bar, _) = ui.allocate_exact_size(vec2(width, 14.0), egui::Sense::hover());
                let mut m = egui::Mesh::default();
                let steps = 24;
                for s in 0..steps {
                    let (k0, k1) = (s as f32 / steps as f32, (s + 1) as f32 / steps as f32);
                    let x0 = bar.left() + bar.width() * k0;
                    let x1 = bar.left() + bar.width() * k1;
                    let r = Rect::from_min_max(egui::pos2(x0, bar.top()), egui::pos2(x1, bar.bottom()));
                    let (c0, c1) = (colours.color(k0, 1.0), colours.color(k1, 1.0));
                    let i = m.vertices.len() as u32;
                    m.colored_vertex(r.left_top(), c0);
                    m.colored_vertex(r.right_top(), c1);
                    m.colored_vertex(r.right_bottom(), c1);
                    m.colored_vertex(r.left_bottom(), c0);
                    m.add_triangle(i, i + 1, i + 2);
                    m.add_triangle(i, i + 2, i + 3);
                }
                ui.painter().add(Shape::mesh(m));
                ui.painter().rect_stroke(bar, 0.0, Stroke::new(1.0, t.text_dim), egui::StrokeKind::Inside);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let w = width / 5.0;
                    for k in 0..5 {
                        let v = top * k as f32 / 4.0;
                        let (r, _) = ui.allocate_exact_size(vec2(w, 14.0), egui::Sense::hover());
                        let align = if k == 4 {
                            egui::Align2::RIGHT_TOP
                        } else if k == 0 {
                            egui::Align2::LEFT_TOP
                        } else {
                            egui::Align2::CENTER_TOP
                        };
                        let x = if k == 4 {
                            r.right()
                        } else if k == 0 {
                            r.left()
                        } else {
                            r.center().x
                        };
                        let label = if k == 4 && f.quantity == Quantity::Stress && f.scale == fea::ScaleMode::Yield {
                            format!("{v:.digits$} ({})", app.tr("ui.stress_estimate_yield"))
                        } else {
                            format!("{v:.digits$}")
                        };
                        ui.painter().text(egui::pos2(x, r.top()), align, label, egui::FontId::proportional(t.font_small), t.text);
                    }
                });
                if f.quantity == Quantity::Stress {
                    ui.horizontal(|ui| {
                        let yield_scale = f.scale == fea::ScaleMode::Yield;
                        if crate::ui_kit::choice_chip(ui, "stress_scale_yield", app.tr("ui.stress_estimate_scale_yield"), yield_scale).clicked() {
                            chosen = Some(json!({"scale": "yield"}));
                        }
                        if crate::ui_kit::choice_chip(ui, "stress_scale_auto", app.tr("ui.stress_estimate_scale_auto"), !yield_scale).clicked() {
                            chosen = Some(json!({"scale": "auto"}));
                        }
                    });
                    if f.scale == fea::ScaleMode::Yield && f.max_stress > f.yield_mpa {
                        ui.label(egui::RichText::new(app.tr("ui.stress_estimate_above_yield")).size(t.font_small).color(t.text_dim));
                    }
                } else if f.deform_scale > 1.0 {
                    ui.label(egui::RichText::new(app.tr("ui.stress_estimate_original_shape")).size(t.font_small).color(t.text_dim));
                }
                ui.add_space(t.space_4);
                // The assumptions fold, closed at first, through the kit's one disclosure.
                let fold = egui::Id::new("sc_stress_estimate_assumptions");
                let mut open = ui.data(|d| d.get_temp::<bool>(fold)).unwrap_or(false);
                let title = app.tr("ui.stress_estimate_assumptions");
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(t.control_height, t.control_height), egui::Sense::hover());
                    if crate::ui_kit::disclosure_button_at(ui, rect, open, title).clicked() {
                        open = !open;
                    }
                    ui.label(egui::RichText::new(title).size(t.font_small).color(t.text));
                });
                ui.data_mut(|d| d.insert_temp(fold, open));
                if open {
                    for a in &f.assumptions {
                        ui.label(egui::RichText::new(format!("• {a}")).size(t.font_small).color(t.text_dim));
                    }
                }
                ui.horizontal(|ui| {
                    let stress = f.quantity == Quantity::Stress;
                    if crate::ui_kit::choice_chip(ui, "stress_show_stress", app.tr("ui.stress_estimate_show_stress"), stress).clicked() {
                        chosen = Some(json!({"show": "stress"}));
                    }
                    if crate::ui_kit::choice_chip(ui, "stress_show_deflection", app.tr("ui.stress_estimate_show_deflection"), !stress).clicked() {
                        chosen = Some(json!({"show": "deflection"}));
                    }
                    if crate::ui_kit::button(ui, app.tr("ui.stress_estimate_hide"), crate::ui_kit::Button::Small).clicked() {
                        chosen = Some(json!({"show": "hide"}));
                    }
                    if crate::ui_kit::button(ui, app.tr("ui.stress_estimate_clear"), crate::ui_kit::Button::Small).clicked() {
                        chosen = Some(json!({"show": "clear"}));
                    }
                });
            });
        });
    if let Some(p) = chosen {
        let _ = app.run("inspect.stress_estimate.show", p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ramp_rises_in_lightness_has_no_red_and_clamps() {
        assert_eq!(ramp(-1.0), RAMP[0]);
        assert_eq!(ramp(f32::NAN), RAMP[0]);
        assert_eq!(ramp(2.0), RAMP[4]);
        // Lightness of the ramp itself (before 8-bit rounding, which can tie two neighbours).
        let lin = |v: f32| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let lightness = |c: [f32; 3]| 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
        for i in 0..1000 {
            let (a, b) = (ramp(i as f32 / 1000.0), ramp((i + 1) as f32 / 1000.0));
            assert!(lightness(b) > lightness(a), "lightness must rise at {i}/1000: {a:?} then {b:?}");
        }
        let samples: Vec<Color32> = (0..=200).map(|i| rgb(ramp(i as f32 / 200.0), 1.0)).collect();
        for c in &samples {
            // No red: red never leads both green and blue by much.
            assert!(i16::from(c.r()) - i16::from(c.g()).max(i16::from(c.b())) < 24, "{c:?} reads red");
        }
    }

    /// Every colour of the scale, lit full and shaded darkest, stands out from the view's
    /// background (its top, middle and bottom) by at least 3:1 in the default Dark and Light
    /// themes and every built-in theme (Black among them), and in each colour vision mode.
    #[test]
    fn every_scale_colour_stands_out_from_each_themes_view() {
        use crate::theme::{ColorVision, Tokens};
        let mut views: Vec<(String, Color32, Color32)> = Vec::new();
        for mode in [ColorVision::Off, ColorVision::Protan, ColorVision::Deutan, ColorVision::Tritan] {
            for (name, dark) in [("Dark", true), ("Light", false)] {
                let t = Tokens::themed(dark, mode);
                views.push((format!("{name} {mode:?}"), t.viewport_top, t.viewport_bottom));
            }
        }
        for theme in crate::theme_files::builtin() {
            let t = crate::theme_files::resolve(theme);
            views.push((theme.id.clone(), t.viewport_top, t.viewport_bottom));
        }
        assert!(views.iter().any(|v| v.0 == "black"), "the Black theme is built in");
        // A mid-tone view leaves no colour of the ramp 3:1 from it. Known: SolidCraft 95's teal.
        // A new theme here needs its own answer (an outline, say), so the list is exact.
        let mid_tone: Vec<&str> = views.iter().filter(|(_, top, bottom)| !Scale::for_view(*top, *bottom).stands_out).map(|v| v.0.as_str()).collect();
        assert_eq!(mid_tone, ["solidcraft-95"], "views where no scale colour stands out");
        for (name, top, bottom) in views.iter().filter(|v| v.0 != "solidcraft-95") {
            let scale = Scale::for_view(*top, *bottom);
            let middle = top.lerp_to_gamma(*bottom, 0.5);
            eprintln!("{name}: view {top:?} .. {bottom:?}, scale {scale:?}");
            for i in 0..=100 {
                for shade in [1.0, SHADE_MIN] {
                    let c = scale.color(i as f32 / 100.0, shade);
                    for bg in [*top, middle, *bottom] {
                        let r = contrast(c, bg);
                        assert!(r >= MIN_CONTRAST, "{name}: {c:?} on {bg:?} is {r:.2}:1 at {i}% (shade {shade})");
                    }
                }
            }
            // The low and high ends still tell apart (lightness alone) in the themes people use.
            if ["Dark Off", "Light Off", "black"].contains(&name.as_str()) {
                let ends = contrast(scale.color(0.0, 1.0), scale.color(1.0, 1.0));
                assert!(ends >= 2.0, "{name}: the scale's ends are only {ends:.2}:1 apart ({scale:?})");
            }
        }
    }

    /// A13: the approval card says what the run will take. Before the user allows it, from a dry
    /// run's prediction (the part's size); once a run has meshed the part, from its count.
    /// Input: a dry run's `predicted` (73,529 nodes, 0.97 GB, 23 s, two runs; then one run), and a
    /// forecast after meshing (run 1 of 2, solids, 68,745 nodes, 0.84 GB, 95 s left). Action:
    /// `predicted_line`, `counted_line`. Pass: the English lines, word for word, with the predicted
    /// count rounded and the counted one exact; no line without a prediction.
    #[test]
    fn the_approval_card_says_what_the_run_will_take_before_and_after_meshing() {
        let dry = json!({"predicted": {"nodes": 73_529, "memory_gb": 0.97, "seconds": 23.0, "runs": 2, "line": ""}});
        assert_eq!(
            predicted_line(&dry).as_deref(),
            Some(
                "Before meshing, from the part's size: about 74\u{2009}000 nodes, about 970 MB of memory, about 25 s in all (2 runs). Small \
                 holes, fillets and bends can make the mesh several times larger."
            )
        );
        let one = json!({"predicted": {"nodes": 1_234_567, "memory_gb": 31.4, "seconds": 5400.0, "runs": 1, "line": ""}});
        let line = predicted_line(&one).unwrap_or_default();
        assert!(line.contains("about 1\u{2009}200\u{2009}000 nodes, about 31 GB of memory, about 1.5 h in all (one run)"), "{line}");
        assert_eq!(predicted_line(&json!({"setup": []})), None);
        let f = fea::Forecast {
            document_id: "d".into(),
            seq: 1,
            run: 1,
            runs: 2,
            runs_most: 2,
            elements: fea::Elements::Solid,
            nodes: 68_745,
            bytes: 0.84e9,
            seconds_left: 95.0,
            seconds_most: 95.0,
        };
        assert_eq!(counted_line(&f), "Meshed, run 1 of 2 (solid): 68\u{2009}745 nodes. The solver needs about 840 MB; about 1.5 min left.");
        // A third mesh may follow: the most it takes, beside what it surely takes.
        let third = fea::Forecast { runs_most: 3, seconds_most: 400.0, ..f.clone() };
        assert_eq!(
            counted_line(&third),
            "Meshed, run 1 of 2 – 3 (solid): 68\u{2009}745 nodes. The solver needs about 840 MB; about 1.5 min – about 6.5 min left."
        );
        let most = json!({"predicted": {"nodes": 51_942, "memory_gb": 0.91, "seconds": 17.0, "runs": 2, "line": "",
                                       "most": {"nodes": 160_000, "memory_gb": 4.2, "seconds": 90.0, "runs": 3}}});
        let line = predicted_line(&most).unwrap_or_default();
        assert!(
            line.contains(
                "about 52\u{2009}000 – 160\u{2009}000 nodes, about 910 MB – 4.2 GB of memory, about 15 s – about 1.5 min in all (2 – 3 runs)"
            ),
            "{line}"
        );
        let shells = fea::Forecast { elements: fea::Elements::Shell, run: 2, nodes: 2_839, bytes: 3.2e9, seconds_left: 0.4, seconds_most: 0.4, ..f };
        assert_eq!(counted_line(&shells), "Meshed, run 2 of 2 (shells): 2\u{2009}839 nodes. The solver needs about 3.2 GB; under a second left.");
    }

    /// A13: the card and the dry run's own sentence, which the agent reads, say the same.
    /// Input: a 100 x 20 x 20 mm bar, held at one end and pulled at the other, as solids with the
    /// mesh check on, at 1 to 8 mm in steps of 0.1. Real: the app's modelling and its dry run.
    /// Action: `predicted_line`. Pass: every memory and time the card names is in the sentence.
    #[test]
    fn the_approval_card_says_what_the_dry_runs_sentence_says() {
        let mut s = solidcraft_engine::Session::default();
        s.execute("solid.box", &json!({"length": 100, "width": 20, "height": 20, "body_name": "Bar"})).unwrap();
        let faces = s.execute("model.faces", &json!({"body": "Bar"})).unwrap();
        let end = |sign: f64| {
            let faces = faces["faces"].as_array().into_iter().flatten();
            let mut facing = faces.filter(|f| f["plane_normal"][0].as_f64().is_some_and(|n| n * sign > 0.99));
            facing.find_map(|f| f["index"].as_u64()).unwrap_or_else(|| panic!("no end face facing {sign} X"))
        };
        let (root, tip) = (end(-1.0), end(1.0));
        let mut wrong = Vec::new();
        for tenths in 10..=80 {
            let r = s
                .execute(
                    "inspect.stress_estimate",
                    &json!({"loads": [{"face": tip, "force": [800, 0, 0]}], "supports": [{"face": root}], "material": "6061-T6", "elements": "solid",
                            "mesh_size": f64::from(tenths) / 10.0, "dry_run": true}),
                )
                .unwrap();
            let sentence = r["predicted"]["line"].as_str().unwrap_or_default().to_string();
            let p = &r["predicted"];
            let card = predicted_line(&r).unwrap_or_else(|| panic!("{r}"));
            for f in [Some(p), p.get("most")].into_iter().flatten() {
                let (gb, seconds) = (f["memory_gb"].as_f64().unwrap_or(f64::NAN), f["seconds"].as_f64().unwrap_or(f64::NAN));
                for words in [memory_words(gb * 1.0e9), time_words(seconds)] {
                    assert!(card.contains(&words), "the card says {words}: {card}");
                    if !sentence.contains(&words) {
                        wrong.push(format!("{tenths} tenths: the card says {words}; {sentence}"));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "{} differ: {}", wrong.len(), wrong[0]);
    }

    #[test]
    fn the_legend_words_say_estimate_and_never_pass_or_fail() {
        let en = crate::i18n::Locale::for_language("en");
        let words: Vec<String> = [
            "ui.stress_estimate_title",
            "ui.stress_estimate_badge",
            "ui.stress_estimate_von_mises",
            "ui.stress_estimate_deflection",
            "ui.stress_estimate_assumptions",
            "ui.stress_estimate_show_stress",
            "ui.stress_estimate_show_deflection",
            "ui.stress_estimate_hide",
            "ui.stress_estimate_clear",
            "ui.stress_estimate_out_of_date",
            "ui.stress_estimate_rerunning",
            "ui.stress_estimate_running",
            "ui.stress_estimate_above_yield",
            "ui.stress_estimate_starter",
            "ui.stress_estimate_mesh_check",
            "ui.stress_estimate_away_marker",
            "ui.stress_estimate_card_before",
            "ui.stress_estimate_card_counted",
        ]
        .iter()
        .map(|id| en.format(id, &[]).to_lowercase())
        .collect();
        assert!(words.iter().any(|w| w.contains("estimate")), "{words:?}");
        for w in &words {
            assert!(!w.starts_with("ui."), "{w}: no English text");
            for verdict in ["pass", "fail", "safe", "approved", "acceptable"] {
                assert!(!w.contains(verdict), "`{verdict}` in `{w}`");
            }
        }
    }
}
