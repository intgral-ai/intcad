//! The desktop Assistant panel has one conversation: the host's page. Without a host it says why
//! and offers Retry or the settings; it never draws a second chat interface.

#[path = "support/raster.rs"]
mod raster;

use std::collections::HashMap;

use egui::{ColorImage, TextureId};
use serde_json::json;
use solidcraft_ui_egui::{
    agent_chat::{Item, web},
    scenario::{self, Harness},
};

#[test]
fn the_panel_never_draws_a_second_conversation() {
    let mut h = Harness::new();
    h.call("ui.resize", json!({"width":1100,"height":1500}));
    h.call("ui.agent_chat", json!({"features":true,"open":true,"picker":false,"width":440.0}));
    h.app.agent_chat.items = vec![Item::Agent { thought: false, text: "A transcript the page would draw.".into() }];
    // The browser build and a desktop host without a view are the same path here: the page is the
    // only conversation.
    web::set_host(&mut h.app, false);
    // A fresh context transfers all textures to the software renderer, including fonts.
    h.ctx = egui::Context::default();
    h.ctx.enable_accesskit();
    solidcraft_ui_egui::fonts::setup_fonts(&h.ctx);
    solidcraft_ui_egui::theme::apply(&h.ctx);
    let mut textures: HashMap<TextureId, ColorImage> = HashMap::new();
    for _ in 0..8 {
        raster::paint(&mut h, &mut textures, vec![]);
    }
    let output = raster::paint(&mut h, &mut textures, vec![]);
    h.output = output.platform_output.clone();
    let screenshot = raster::raster(&mut h, &textures, output);
    let panel = scenario::rect_of("agent_chat:panel").expect("Assistant panel");
    assert!(panel.width() > 0.0);
    // The panel draws its diagnostic and actions, so its area is not left as the window's own
    // background. (A colour outside the panel is the untouched background.)
    let background = screenshot.get_pixel(1, 1).0;
    let painted = (panel.top() as u32..panel.bottom() as u32)
        .step_by(4)
        .any(|y| (panel.left() as u32..panel.right() as u32).step_by(4).any(|x| screenshot.get_pixel(x, y).0 != background));
    assert!(painted, "the Assistant panel draws its diagnostic and actions");
    assert!(h.widget("A transcript the page would draw.").is_none(), "the egui panel must not draw a second conversation");
    assert!(
        scenario::handle_at("agent_chat:web_retry").is_some() || scenario::handle_at("agent_chat:web_settings").is_some(),
        "the panel offers Retry or its settings instead"
    );
}
