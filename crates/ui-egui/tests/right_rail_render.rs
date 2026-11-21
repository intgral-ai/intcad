//! Pictures of the right rail in the states a reviewer would look at, rendered from the actual
//! egui meshes without a GPU into `target/right-rail-review/`. The 3D view stays empty in them:
//! only the interface is drawn.
// Only the sized paint is used here; the helper's default-size one is for other tests.
#[allow(dead_code)]
#[path = "support/raster.rs"]
mod raster;

use egui::{ColorImage, Rect, TextureId};
use serde_json::{Value, json};
use solidcraft_ui_egui::{
    scenario::{self, Harness},
    theme,
};
use std::collections::HashMap;

/// One window: the sample design, `prefs` and `layout` applied, then whatever `more` does.
fn window(size: (f32, f32), layout: &str, prefs: Value, more: impl FnOnce(&mut Harness)) -> (Harness, image::RgbaImage) {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.app.session.run_script(&solidcraft_engine::sample::script()).unwrap();
    h.call("ui.resize", json!({"width": size.0, "height": size.1}));
    h.call("ui.set", json!({"layout": layout}));
    // The harness starts on the window without the rail.
    h.call("ui.prefs", json!({"right_rail": true}));
    let language = prefs.get("language").and_then(Value::as_str).unwrap_or("en").to_string();
    h.call("ui.prefs", prefs);
    more(&mut h);
    h.frames(6);
    // A fresh context hands every texture to the software renderer, fonts included.
    h.ctx = egui::Context::default();
    solidcraft_ui_egui::fonts::setup_fonts_for(&h.ctx, &language, "");
    theme::apply(&h.ctx);
    let size = egui::vec2(size.0, size.1);
    let mut textures: HashMap<TextureId, ColorImage> = HashMap::new();
    for _ in 0..8 {
        raster::paint_at(&mut h, &mut textures, vec![], size);
    }
    let output = raster::paint_at(&mut h, &mut textures, vec![], size);
    let image = raster::raster(&mut h, &textures, output);
    (h, image)
}

fn inside(image: &image::RgbaImage, rect: Rect) -> bool {
    rect.is_positive() && rect.left() >= 0.0 && rect.top() >= 0.0 && rect.right() <= image.width() as f32 && rect.bottom() <= image.height() as f32
}

/// One picture: its file name, the window, the layout, the preferences, the item whose panel
/// must be drawn, and what else is set up.
type Shot = (String, (f32, f32), &'static str, Value, Option<&'static str>, Box<dyn FnOnce(&mut Harness)>);

#[test]
fn the_right_rail_and_its_panels_render_in_both_layouts_and_three_themes() {
    let previous = theme::is_dark();
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/right-rail-review");
    std::fs::create_dir_all(&dir).unwrap();
    let wide = (1600.0, 1000.0);
    let assistant = |side: &'static str| -> Box<dyn FnOnce(&mut Harness)> {
        Box::new(move |h: &mut Harness| drop(h.call("ui.agent_chat", json!({"open": true, "side": side, "width": 440, "picker": false}))))
    };
    let nothing = || -> Box<dyn FnOnce(&mut Harness)> { Box::new(|_: &mut Harness| {}) };
    let sketch = || -> Box<dyn FnOnce(&mut Harness)> { Box::new(|h: &mut Harness| drop(h.app.run("sketch.create", json!({"plane": "XY"})))) };
    let open = |item: &'static str, theme: &str| (json!({"theme": theme, "right_rail_open": item}), Some(item));

    // A folder with a design, a document and a subfolder, so the Files panel renders a real tree.
    let files_dir = std::env::temp_dir().join(format!("intcad-right-rail-files-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&files_dir);
    std::fs::create_dir_all(files_dir.join("parts")).unwrap();
    std::fs::write(files_dir.join("bracket.sc"), "").unwrap();
    std::fs::write(files_dir.join("README.md"), "").unwrap();
    std::fs::write(files_dir.join("parts/pin.sc"), "").unwrap();
    let files_root = || -> Box<dyn FnOnce(&mut Harness)> {
        let dir = files_dir.to_string_lossy().into_owned();
        Box::new(move |h: &mut Harness| {
            h.call("ui.files", json!({"root": dir}));
            h.frames(20);
        })
    };
    let mut shots: Vec<Shot> = Vec::new();
    for layout in ["floating", "docked"] {
        let name = |what: &str| format!("{layout}-{what}");
        shots.push((name("closed"), wide, layout, json!({"theme": "dark"}), None, nothing()));
        let (prefs, item) = open("review", "dark");
        shots.push((name("review"), wide, layout, prefs, item, nothing()));
        let (prefs, item) = open("files", "dark");
        shots.push((name("files-assistant-right"), wide, layout, prefs.clone(), item, files_root()));
        shots.push((name("files"), wide, layout, prefs, item, files_root()));
        let (prefs, item) = open("autocomplete", "dark");
        shots.push((name("autocomplete-assistant-left"), wide, layout, prefs, item, assistant("left")));
        let prefs = json!({"theme": "dark", "right_rail_open": "simulation", "status_bar": true});
        shots.push((name("simulation-sketch-status-bar"), wide, layout, prefs, Some("simulation"), sketch()));
    }
    let (prefs, item) = open("simulation", "light");
    shots.push(("light-simulation".into(), wide, "floating", prefs, item, nothing()));
    let (prefs, item) = open("review", "black");
    shots.push(("black-review".into(), wide, "floating", prefs, item, nothing()));
    shots.push(("black-closed".into(), wide, "floating", json!({"theme": "black"}), None, nothing()));
    let prefs = json!({"theme": "dark", "language": "zh", "right_rail_open": "files"});
    shots.push(("chinese-files".into(), wide, "floating", prefs, Some("files"), nothing()));
    let prefs = json!({"theme": "dark", "language": "ar", "right_rail_open": "autocomplete"});
    shots.push(("arabic-autocomplete".into(), wide, "floating", prefs, Some("autocomplete"), nothing()));
    let prefs = json!({"theme": "dark", "right_rail_open": "review", "right_rail_width": 420});
    shots.push(("narrow-panel-squeezed".into(), (640.0, 800.0), "floating", prefs, Some("review"), nothing()));
    let (prefs, _) = open("review", "dark");
    shots.push(("narrow-panel-left-out".into(), (520.0, 800.0), "floating", prefs, None, nothing()));

    for (name, size, layout, prefs, open, more) in shots {
        let (h, image) = window(size, layout, prefs, more);
        let rail = scenario::rect_of("rail:strip").unwrap_or_else(|| panic!("{name}: the rail is drawn"));
        assert!(inside(&image, rail) && rail.right() == size.0, "{name}: {rail:?}");
        let panel = scenario::rect_of("rail:panel");
        assert_eq!(panel.is_some(), open.is_some(), "{name}: {panel:?}");
        if let Some(panel) = panel {
            assert!(inside(&image, panel) && panel.right() <= rail.left() + 1.0, "{name}: {panel:?} beside {rail:?}");
        }
        for item in ["files", "review", "simulation", "autocomplete"] {
            assert!(scenario::rect_of(&format!("rail:item:{item}")).is_some_and(|r| rail.contains_rect(r)), "{name}: {item}");
        }
        if name.ends_with("files") && name.contains("floating") {
            for row in ["files:row:parts", "files:row:bracket.sc", "files:row:README.md"] {
                assert!(scenario::rect_of(row).is_some_and(|r| panel.is_some_and(|p| p.contains_rect(r))), "{name}: {row}");
            }
        }
        if let Some(sheet) = h.app.viewport.rect {
            let side = panel.map_or(rail.left(), |panel| panel.left());
            assert!(sheet.right() <= side + 1.0, "{name}: the view ends where the rail's side begins: {sheet:?}");
        }
        image.save(dir.join(format!("{name}.png"))).unwrap();
    }
    theme::set_dark(previous);
    let _ = std::fs::remove_dir_all(&files_dir);
}
