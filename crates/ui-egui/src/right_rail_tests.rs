use super::*;
use crate::scenario::{Harness, rect_of};
use crate::shell::LayoutMode;
use serde_json::json;

fn window(mode: LayoutMode, width: u32) -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    // The harness starts on the window without the rail; these tests are about the rail.
    h.call("ui.prefs", json!({"right_rail": true}));
    h.call("ui.resize", json!({"width": width, "height": 900}));
    h.call("ui.set", json!({"layout": mode}));
    h.app.run("solid.box", json!({"length": 40, "width": 30, "height": 20})).unwrap();
    h.settle();
    h
}

fn item(item: Item) -> Rect {
    rect_of(&format!("rail:item:{}", item.id())).unwrap()
}

fn click(h: &mut Harness, item: Item) {
    h.step(&json!({"click": {"handle": format!("rail:item:{}", item.id())}})).unwrap();
    h.settle();
}

/// The rail is on in a fresh application: the preference's default, and what a reset of the
/// Layout page restores. Only the test harness starts without it.
#[test]
fn the_rail_is_on_by_default() {
    assert!(crate::prefs::Prefs::default().right_rail);
    let app = crate::SolveApp::new(solidcraft_engine::Session::default(), crate::Services::default());
    assert!(app.preferences.right_rail && open(&app).is_none() && panel_width(&app) == DEFAULT_PANEL);
    assert!(!Harness::new().app.preferences.right_rail, "the suite's own window has no rail");
}

/// In both layouts the rail stands at the window's right edge, from under the application bar to
/// the window's bottom, with its four items in order and Files set apart from the three under
/// it. No panel is open at first, and the view ends where the rail begins.
#[test]
fn the_rail_stands_at_the_windows_right_edge_in_both_layouts() {
    for mode in [LayoutMode::Docked, LayoutMode::Floating] {
        let h = window(mode, 1400);
        let rail = rect_of("rail:strip").unwrap();
        assert_eq!((rail.right(), rail.width()), (1400.0, width(&h.ctx)), "{mode:?}: {rail:?}");
        assert!((MIN_WIDTH..=MAX_WIDTH).contains(&rail.width()), "{mode:?}: {rail:?}");
        assert_eq!((rail.top(), rail.bottom()), (crate::toolbar::APP_BAR_HEIGHT, 900.0), "{mode:?}: {rail:?}");
        let items = Item::ALL.map(item);
        assert!(items.iter().all(|r| rail.contains_rect(*r)), "{mode:?}: {items:?} in {rail:?}");
        for pair in items.windows(2) {
            assert!(pair[0].bottom() <= pair[1].top(), "{mode:?}: the items stand in order: {items:?}");
        }
        // Files and Tickets stand together; a rule sets them apart from the ways of working.
        let (in_project, before_modes) = (items[1].top() - items[0].bottom(), items[2].top() - items[1].bottom());
        assert!(before_modes > in_project + 4.0, "{mode:?}: a rule sets Files and Tickets apart: {before_modes} against {in_project}");
        assert!(rect_of("rail:panel").is_none() && open(&h.app).is_none(), "{mode:?}: no panel is open at first");
        assert_eq!(h.app.shell.right_rail, Some(rail), "{mode:?}: the shell knows where the rail stands");
        let view = h.app.viewport.rect.unwrap();
        assert!(view.right() <= rail.left() + 0.5 && view.width() > 600.0, "{mode:?}: the view ends at the rail: {view:?} {rail:?}");
    }
}

/// The Assistant's entry is the rail's first row, over a rule of its own, and beside a rail the
/// application bar has no Assistant button. A press opens the Assistant and the same press
/// closes it; an item's panel opens beside it and neither closes the other. Where no rail is
/// drawn and the Assistant can show, the bar keeps its button as before the rail: with the rail
/// preference off, and in the Render workspace. Home draws neither.
#[test]
fn the_assistant_is_the_rails_first_row_and_leaves_the_application_bar() {
    let press = |h: &mut Harness| {
        h.step(&json!({"click": {"handle": "rail:assistant"}})).unwrap();
        h.settle();
    };
    for mode in [LayoutMode::Docked, LayoutMode::Floating] {
        let mut h = window(mode, 1400);
        let (rail, entry, files) = (rect_of("rail:strip").unwrap(), rect_of("rail:assistant").unwrap(), item(Item::Files));
        assert!(rail.contains_rect(entry) && entry.size() == files.size(), "{mode:?}: {entry:?} in {rail:?}");
        let (after_entry, between_modes) = (files.top() - entry.bottom(), item(Item::Simulation).top() - item(Item::Review).bottom());
        assert!(after_entry > between_modes + 4.0, "{mode:?}: a rule sets the Assistant apart: {after_entry} against {between_modes}");
        assert!(rect_of("titlebar:assistant").is_none() && rect_of("titlebar:preferences").is_some(), "{mode:?}: the bar has no Assistant button");
        let name = crate::agent_chat::assistant_tip(&h.app, crate::agent_chat::assistant_mark(&h.app));
        assert!(h.widget(&name).is_some(), "{mode:?}: the row is named with the Assistant's shortcut and state: {name}");

        press(&mut h);
        assert!(h.app.agent_chat.open && rect_of("agent_chat:panel").is_some(), "{mode:?}: a press opens the Assistant");
        click(&mut h, Item::Files);
        assert!(h.app.agent_chat.open && rect_of("agent_chat:panel").is_some(), "{mode:?}: an item leaves the Assistant open");
        assert!(open(&h.app) == Some(Item::Files) && rect_of("rail:panel").is_some(), "{mode:?}: its panel opens beside the Assistant");
        press(&mut h);
        assert!(!h.app.agent_chat.open && rect_of("agent_chat:panel").is_none(), "{mode:?}: the same press closes the Assistant");
        assert_eq!(open(&h.app), Some(Item::Files), "{mode:?}: the item's panel stays open");

        h.call("ui.prefs", json!({"right_rail": false}));
        h.settle();
        assert!(rect_of("rail:assistant").is_none() && rect_of("titlebar:assistant").is_some(), "{mode:?}: without the rail the bar has its button");
        h.step(&json!({"click": {"handle": "titlebar:assistant"}})).unwrap();
        assert!(h.app.agent_chat.open, "{mode:?}: and it opens the Assistant as before");
    }
    let mut h = window(LayoutMode::Floating, 1400);
    h.call("ui.workspace", json!({"workspace": "render"}));
    h.settle();
    assert!(rect_of("rail:assistant").is_none() && rect_of("titlebar:assistant").is_some(), "the Render workspace keeps the bar's button");
    let mut h = window(LayoutMode::Floating, 1400);
    h.app.home.open = true;
    h.settle();
    assert!(rect_of("rail:assistant").is_none() && rect_of("titlebar:assistant").is_none(), "Home draws neither");
    // With the AI features off the rail starts at Files, and nothing of the Assistant is drawn.
    let mut h = window(LayoutMode::Floating, 1400);
    let first = rect_of("rail:assistant").unwrap().top();
    h.app.preferences.ai_features = false;
    h.settle();
    assert!(rect_of("rail:assistant").is_none() && rect_of("titlebar:assistant").is_none() && item(Item::Files).top() == first);
}

/// An item opens its panel beside the rail, another item replaces it, and the open item closes
/// it: one panel at a time. The view gives the panel its width and takes it back.
#[test]
fn an_item_opens_its_panel_and_the_same_item_closes_it() {
    for mode in [LayoutMode::Docked, LayoutMode::Floating] {
        let mut h = window(mode, 1400);
        let rail = rect_of("rail:strip").unwrap();
        let view_closed = h.app.viewport.rect.unwrap();

        click(&mut h, Item::Review);
        assert_eq!(open(&h.app), Some(Item::Review), "{mode:?}");
        let panel = rect_of("rail:panel").unwrap();
        assert!((panel.right() - rail.left()).abs() <= 1.0, "{mode:?}: the panel stands beside the rail: {panel:?} {rail:?}");
        assert!((panel.width() - DEFAULT_PANEL).abs() <= 1.0, "{mode:?}: {panel:?}");
        assert!(rect_of("review:run").is_some(), "{mode:?}: the Review panel offers its check");
        assert!(h.widget(Item::Files.empty()).is_none(), "{mode:?}: one panel at a time");
        let view = h.app.viewport.rect.unwrap();
        assert!(view.right() <= panel.left() + 1.0, "{mode:?}: the view ends at the panel: {view:?} {panel:?}");
        assert_eq!(h.app.shell.right_rail, Some(rail.union(panel)), "{mode:?}");
        let edge = egui::pos2(panel.left() - 3.0, panel.center().y);
        assert!(
            crate::shell::covered(&h.app, edge) && !crate::shell::covered(&h.app, edge - egui::vec2(20.0, 0.0)),
            "{mode:?}: the panel's edge is not the view's to drag"
        );

        click(&mut h, Item::Autocomplete);
        assert_eq!(open(&h.app), Some(Item::Autocomplete), "{mode:?}: another item replaces the panel");
        assert!(h.widget(Item::Autocomplete.empty()).is_some() && rect_of("review:run").is_none(), "{mode:?}");

        click(&mut h, Item::Autocomplete);
        assert_eq!(open(&h.app), None, "{mode:?}: the open item closes its panel");
        assert!(rect_of("rail:panel").is_none(), "{mode:?}");
        assert_eq!(h.app.viewport.rect.unwrap(), view_closed, "{mode:?}: the view has its width back");
    }
}

/// The open item and the panel's width are remembered as preferences, and both are taken as
/// hostile input: a name that is none of the four means no panel, and a width out of range or
/// not a number is brought into range. A width set from outside is taken up at once.
#[test]
fn the_open_item_and_the_width_are_remembered_and_checked() {
    let mut h = window(LayoutMode::Floating, 1400);
    h.call("ui.prefs", json!({"right_rail_open": "simulation"}));
    h.settle();
    assert_eq!(open(&h.app), Some(Item::Simulation));
    assert!((rect_of("rail:panel").unwrap().width() - DEFAULT_PANEL).abs() <= 1.0);

    h.call("ui.prefs", json!({"right_rail_width": 420.0}));
    h.settle();
    assert!((rect_of("rail:panel").unwrap().width() - 420.0).abs() <= 1.0, "a width set from outside is taken up");
    assert!((h.app.preferences.right_rail_width - 420.0).abs() <= 1.0);

    for (stored, shown) in [(40.0, MIN_PANEL), (9000.0, MAX_PANEL), (f32::NAN, DEFAULT_PANEL), (f32::INFINITY, DEFAULT_PANEL)] {
        h.app.preferences.right_rail_width = stored;
        assert_eq!(panel_width(&h.app), shown, "{stored}");
        h.settle();
        let width = rect_of("rail:panel").unwrap().width();
        assert!((MIN_PANEL..=MAX_PANEL).contains(&width), "{stored}: drawn {width} wide");
    }

    for name in ["", "assistant", "Files", "review ", "check", "lessons", "intent", "\u{0}"] {
        h.app.preferences.right_rail_open = name.to_string();
        h.settle();
        assert_eq!(open(&h.app), None, "{name:?}");
        assert!(rect_of("rail:panel").is_none(), "{name:?}");
    }
    for known in Item::ALL {
        assert_eq!(Item::parse(known.id()), Some(known));
    }
}

/// The preference takes the rail away and the window is as it was before the rail existed: the
/// view runs to the window's right edge again. Home, which has no design to look at, the Render
/// workspace, which has its own panels there, and an embed, which has no chrome, never show it.
#[test]
fn the_preference_takes_the_rail_away_and_home_render_and_an_embed_never_show_it() {
    for mode in [LayoutMode::Docked, LayoutMode::Floating] {
        let mut h = window(mode, 1400);
        click(&mut h, Item::Files);
        let rail = rect_of("rail:strip").unwrap();
        h.call("ui.prefs", json!({"right_rail": false}));
        h.settle();
        assert!(!shown(&h.app) && rect_of("rail:strip").is_none() && rect_of("rail:panel").is_none(), "{mode:?}");
        assert!(h.app.shell.right_rail.is_none() && !crate::shell::covered(&h.app, rail.center()), "{mode:?}");
        assert!(Item::ALL.iter().all(|item| rect_of(&format!("rail:item:{}", item.id())).is_none()), "{mode:?}");
        let view = h.app.viewport.rect.unwrap();
        assert!((view.right() - rail.right()).abs() <= 0.5, "{mode:?}: the view runs to the window's edge again: {view:?}");
        assert_eq!(open(&h.app), Some(Item::Files), "{mode:?}: the open item is kept for when the rail returns");
    }
    let mut h = window(LayoutMode::Floating, 1400);
    h.app.home.open = true;
    h.settle();
    assert!(h.app.home.open && !shown(&h.app) && rect_of("rail:strip").is_none(), "Home has no rail");
    let mut h = window(LayoutMode::Floating, 1400);
    h.call("ui.workspace", json!({"workspace": "render"}));
    h.settle();
    assert!(crate::render_workspace::active(&h.app) && !shown(&h.app) && rect_of("rail:strip").is_none(), "the Render workspace has no rail");
    let mut h = Harness::new();
    h.call("ui.prefs", json!({"right_rail": true}));
    h.app.embedded = true;
    h.settle();
    assert!(h.app.preferences.right_rail && !shown(&h.app) && rect_of("rail:strip").is_none());
}

/// The Assistant docks beside the rail, in both layouts and with a panel open: dragged to the
/// right half of the window, its landing place is shown where it then lands, left of the rail.
#[test]
fn the_assistant_docks_beside_the_rail_and_its_landing_place_says_so() {
    for mode in [LayoutMode::Docked, LayoutMode::Floating] {
        for open_panel in [false, true] {
            let mut h = window(mode, 1600);
            if open_panel {
                click(&mut h, Item::Files);
            }
            h.call("ui.agent_chat", json!({"open": true, "side": "left", "width": 440, "picker": false}));
            h.frames(6);
            let side = h.app.shell.right_rail.unwrap().left();
            let grip = crate::scenario::handle_at("agent_chat:grip").expect("the Assistant's drag grip");
            let target = egui::pos2(side - 100.0, grip.y);
            let press = |pressed: bool, pos: egui::Pos2| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            h.app.synthetic.push(egui::Event::PointerMoved(grip));
            h.frame();
            h.app.synthetic.push(press(true, grip));
            h.frame();
            h.app.synthetic.push(egui::Event::PointerMoved(target));
            h.frames(2);
            let hint = rect_of("agent_chat:dock_hint").expect("the landing place is shown");
            h.app.synthetic.push(press(false, target));
            h.frames(6);
            let card = h.app.agent_chat.panel_rect.expect("the docked Assistant");
            assert_eq!(h.app.preferences.agent_chat_side, "right", "{mode:?}");
            assert!(
                (hint.min - card.min).length() < 1.0 && (hint.max - card.max).length() < 1.0,
                "{mode:?}, panel {open_panel}: shown {hint:?}, landed {card:?}"
            );
            assert!(card.right() <= side + 1.0 && hint.right() <= side + 1.0, "{mode:?}, panel {open_panel}: beside the rail at {side}: {card:?}");
        }
    }
}

/// In a window too narrow for the panel and a usable view, the panel is not drawn: the view
/// keeps its least width, the rail still marks the open item, and the chosen width is kept for
/// a wider window.
#[test]
fn a_narrow_window_keeps_the_view_and_leaves_the_panel_out() {
    let mut h = window(LayoutMode::Floating, 1400);
    h.call("ui.prefs", json!({"right_rail_open": "review", "right_rail_width": 400.0}));
    h.settle();
    assert!(rect_of("rail:panel").is_some());

    // Room for a narrower panel than the one chosen: it gives way, the view keeps its least width.
    h.call("ui.resize", json!({"width": width(&h.ctx) + MIN_VIEW + 300.0, "height": 900}));
    h.settle();
    let panel = rect_of("rail:panel").unwrap();
    assert!(panel.width() <= 301.0 && panel.width() >= MIN_PANEL, "{panel:?}");
    let view = h.app.viewport.rect.unwrap();
    assert!(view.width() >= MIN_VIEW - 1.0, "{view:?}");
    assert!((h.app.preferences.right_rail_width - 400.0).abs() <= 1.0, "the chosen width is kept for a wider window");

    // No room for both: the rail alone, with its item still marked open.
    h.call("ui.resize", json!({"width": 500, "height": 900}));
    h.settle();
    assert!(rect_of("rail:panel").is_none() && rect_of("rail:strip").is_some());
    assert_eq!(open(&h.app), Some(Item::Review));
    let view = h.app.viewport.rect.unwrap();
    assert!(view.width() >= MIN_VIEW, "{view:?}");

    h.call("ui.resize", json!({"width": 1400, "height": 900}));
    h.settle();
    assert!((rect_of("rail:panel").unwrap().width() - 400.0).abs() <= 1.0);
}

/// The names follow the application's language, and each item carries its panel's title as its
/// accessible name.
#[test]
fn the_items_are_named_in_the_apps_language() {
    let mut h = window(LayoutMode::Floating, 1400);
    assert_eq!(Item::ALL.map(Item::name), ["Files", "Tickets", "Review", "Simulation", "Autocomplete"]);
    for item in Item::ALL {
        assert!(h.widget(item.title()).is_some(), "{item:?} is a named widget");
    }
    h.call("ui.prefs", json!({"language": "zh"}));
    h.settle();
    let _language = crate::i18n::scope("zh");
    assert_eq!(Item::ALL.map(Item::name), ["文件", "部件票", "审查", "仿真", "补全"]);
    assert_eq!(Item::ALL.map(Item::title), ["项目文件", "部件票", "审查", "仿真", "自动补全"]);
    for item in Item::ALL {
        assert!(h.widget(item.title()).is_some(), "{item:?} is named in Chinese");
    }
}

/// The rail works without a mouse: Tab walks its four items in the order they stand, and Enter
/// or Space on the focused item opens its panel and closes it again.
#[test]
fn the_items_are_reached_and_opened_with_the_keyboard() {
    let mut h = window(LayoutMode::Floating, 1400);
    let focused = |h: &Harness| h.ctx.memory(|memory| memory.focused()).and_then(|id| Item::ALL.into_iter().find(|item| item_id(*item) == id));
    h.ctx.memory_mut(|memory| memory.request_focus(item_id(Item::Files)));
    h.frame();
    let mut walked = vec![focused(&h)];
    for _ in 1..Item::ALL.len() {
        h.step(&json!({"key": "Tab"})).unwrap();
        walked.push(focused(&h));
    }
    assert_eq!(walked, Item::ALL.map(Some), "Tab walks the items in order");

    for key in ["Enter", "Space"] {
        h.ctx.memory_mut(|memory| memory.request_focus(item_id(Item::Simulation)));
        h.frame();
        h.step(&json!({"key": key})).unwrap();
        h.settle();
        assert_eq!(open(&h.app), Some(Item::Simulation), "{key} opens the focused item's panel");
        assert_eq!(focused(&h), Some(Item::Simulation), "{key}: the item keeps the focus");
        h.step(&json!({"key": key})).unwrap();
        h.settle();
        assert_eq!(open(&h.app), None, "{key} closes it again");
    }
}

/// A drag on the panel's left edge makes it wider, between its narrowest and widest, and the
/// width is remembered: it is there again after the panel was closed and opened.
#[test]
fn the_panel_is_dragged_wider_and_keeps_its_width() {
    let mut h = window(LayoutMode::Floating, 1400);
    click(&mut h, Item::Review);
    let panel = rect_of("rail:panel").unwrap();
    // A hand takes the edge a little to its view side, where the view would otherwise answer too.
    let grip = [f64::from(panel.left()) - 2.0, f64::from(panel.center().y)];
    h.step(&json!({"drag": [grip, [grip[0] - 100.0, grip[1]]]})).unwrap();
    h.settle();
    let wider = rect_of("rail:panel").unwrap();
    assert!((wider.width() - (DEFAULT_PANEL + 100.0)).abs() <= 3.0, "{panel:?} to {wider:?}");
    assert!((h.app.preferences.right_rail_width - wider.width()).abs() <= 0.5, "the width is remembered");
    assert_eq!(wider.right(), panel.right(), "the panel grows away from the rail");

    click(&mut h, Item::Review);
    click(&mut h, Item::Files);
    assert!((rect_of("rail:panel").unwrap().width() - wider.width()).abs() <= 0.5, "another item's panel has the same width");

    let grip = [f64::from(rect_of("rail:panel").unwrap().left()), grip[1]];
    h.step(&json!({"drag": [grip, [grip[0] + 900.0, grip[1]]]})).unwrap();
    h.settle();
    assert!((rect_of("rail:panel").unwrap().width() - MIN_PANEL).abs() <= 0.5, "never narrower than its least width");
    h.step(&json!({"drag": [[f64::from(rect_of("rail:panel").unwrap().left()), grip[1]], [20.0, grip[1]]]})).unwrap();
    h.settle();
    assert!((rect_of("rail:panel").unwrap().width() - MAX_PANEL).abs() <= 0.5, "never wider than its greatest width");
}

/// The Tickets panel opens at its own first width while no width was chosen, and opening it
/// stores nothing, so the other panels keep theirs. A width chosen before wins, and a drag on the
/// Tickets panel is that width for every panel.
#[test]
fn the_tickets_panel_opens_wider_until_a_width_is_chosen() {
    let mut h = window(LayoutMode::Floating, 1400);
    click(&mut h, Item::Tickets);
    let panel = rect_of("rail:panel").unwrap();
    assert!((panel.width() - TICKETS_PANEL).abs() <= 1.0, "{panel:?}");
    assert_eq!(h.app.preferences.right_rail_width, DEFAULT_PANEL, "opening it stores nothing");
    for other in [Item::Files, Item::Review] {
        click(&mut h, other);
        assert!((rect_of("rail:panel").unwrap().width() - DEFAULT_PANEL).abs() <= 1.0, "{other:?}");
    }

    h.call("ui.prefs", json!({"right_rail_open": "tickets", "right_rail_width": 360.0}));
    h.settle();
    assert!((rect_of("rail:panel").unwrap().width() - 360.0).abs() <= 1.0, "a width chosen before wins");

    h.call("ui.prefs", json!({"right_rail_width": DEFAULT_PANEL}));
    h.settle();
    let panel = rect_of("rail:panel").unwrap();
    let grip = [f64::from(panel.left()) - 2.0, f64::from(panel.center().y)];
    h.step(&json!({"drag": [grip, [grip[0] - 60.0, grip[1]]]})).unwrap();
    h.settle();
    let wider = rect_of("rail:panel").unwrap();
    assert!((wider.width() - (TICKETS_PANEL + 60.0)).abs() <= 3.0, "{panel:?} to {wider:?}");
    assert!((h.app.preferences.right_rail_width - wider.width()).abs() <= 0.5, "the drag is remembered");
    click(&mut h, Item::Files);
    assert!((rect_of("rail:panel").unwrap().width() - wider.width()).abs() <= 0.5, "and is every panel's width");
}

/// The rail and its panel read in Dark, Light and Black: a quiet name and the open item's name
/// on the rail, a title and the empty block's sentence in the panel.
#[test]
fn the_rail_and_its_panel_read_in_dark_light_and_black() {
    let mut h = window(LayoutMode::Floating, 1400);
    for theme in ["dark", "light", "black"] {
        h.call("ui.prefs", json!({"theme": theme}));
        h.frames(40);
        assert_eq!(h.app.preferences.theme, theme);
        let t = Tokens::get();
        let pairs = [
            (t.text_dim, t.panel, "a quiet name on the rail, and the empty block's sentence"),
            (t.text, t.hover, "the open item's name on its fill"),
            (t.text, t.panel, "a panel's title"),
        ];
        for (ink, surface, what) in pairs {
            let ratio = crate::theme::contrast_ratio(ink, surface);
            assert!(ratio >= 4.5, "{theme}: {what}: contrast {ratio:.2}");
        }
    }
}

/// The rail is as wide as the longest name needs, in every language of the catalogs, and no
/// name is cut: none needs more than the rail's greatest width. In the right-to-left languages
/// the empty blocks' sentences are broken by hand, and every such line fits the narrowest panel,
/// because a line the panel wrapped would be read in the wrong order.
#[test]
fn the_names_fit_the_rail_in_every_language_and_right_to_left_lines_fit_the_panel() {
    let mut h = window(LayoutMode::Floating, 1400);
    let mut widths = Vec::new();
    for language in ["en", "zh", "ja", "de", "ko", "ar", "he", "vi"] {
        h.call("ui.prefs", json!({"language": language}));
        h.settle();
        let _language = crate::i18n::scope(language);
        let t = Tokens::get();
        let measure = |text: &str, size: f32| {
            h.ctx.fonts_mut(|fonts| fonts.layout_no_wrap(text.to_string(), FontId::proportional(size), egui::Color32::WHITE).size().x)
        };
        let rail = rect_of("rail:strip").unwrap().width();
        assert_eq!(rail, width(&h.ctx), "{language}");
        let assistant = measure(tr("ui.assistant"), t.font_micro);
        assert!(
            assistant <= rail - t.space_12 * 2.0 + 1.0 && rail <= MAX_WIDTH,
            "{language}: the Assistant's name is {assistant} wide in a rail of {rail}"
        );
        for item in Item::ALL {
            let name = measure(item.name(), t.font_micro);
            assert!(name + t.space_12 * 2.0 <= MAX_WIDTH, "{language}: {:?} is {name} wide", item.name());
            assert!(name <= rail - t.space_12 * 2.0 + 1.0, "{language}: {:?} is {name} wide in a rail of {rail}", item.name());
            // Review's empty block is not drawn: its panel shows the check's entry and findings.
            if matches!(language, "ar" | "he") && item != Item::Review {
                let room = MIN_PANEL - t.space_16 * 2.0;
                for (text, size) in [(item.empty(), t.font_label), (item.note(), t.font_small)] {
                    for line in text.lines() {
                        let line_width = measure(line, size);
                        assert!(line_width <= room, "{language}: {line:?} is {line_width} wide, the narrowest panel has {room}");
                    }
                }
            }
        }
        widths.push(rail);
    }
    assert!(widths.contains(&MIN_WIDTH) && widths.iter().any(|w| *w > MIN_WIDTH), "short names keep the rail narrow, long ones widen it: {widths:?}");
}

/// A panel with nothing to list shows one block in its middle: what is missing and, under it,
/// the sentence on what will be listed. Both are labels a screen reader finds.
#[test]
fn an_empty_panel_shows_one_centred_block() {
    let mut h = window(LayoutMode::Floating, 1400);
    // Review lists the DFM check's findings instead of an empty block: its own empty state is
    // the check's entry, tested with the panel in `dfm_review_tests`.
    for item in Item::ALL.into_iter().filter(|item| !matches!(item, Item::Review | Item::Files | Item::Tickets)) {
        h.call("ui.prefs", json!({"right_rail_open": item.id()}));
        h.settle();
        let panel = rect_of("rail:panel").unwrap();
        let [x, y] = h.widget(item.empty()).unwrap_or_else(|| panic!("{item:?}: what is missing"));
        let [note_x, note_y] = h.widget(item.note()).unwrap_or_else(|| panic!("{item:?}: the sentence"));
        let centre = f64::from(panel.center().x);
        assert!((x - centre).abs() <= 1.0 && (note_x - centre).abs() <= 1.0, "{item:?}: centred across the panel: {x} {note_x} {centre}");
        assert!(note_y > y && (y - f64::from(panel.center().y)).abs() < 60.0, "{item:?}: in the panel's middle: {y} {note_y} {panel:?}");
        for other in Item::ALL.into_iter().filter(|other| *other != item && !matches!(other, Item::Review | Item::Files | Item::Tickets)) {
            assert!(h.widget(other.empty()).is_none() && h.widget(other.note()).is_none(), "{item:?}: nothing of {other:?}");
        }
    }
    // Files has a state of its own: with no folder chosen it offers one.
    h.call("ui.prefs", json!({"right_rail_open": "files"}));
    h.settle();
    assert!(rect_of("files:empty").is_some() && rect_of("files_open_folder").is_some(), "Files offers a folder");
    assert!(h.widget(Item::Files.empty()).is_none(), "the old empty block is not drawn over it");
}

/// The Tickets panel gives its whole place to the tickets view's web view, and draws its empty
/// block only where the host could make no web view.
#[test]
fn the_tickets_panel_gives_its_place_to_the_web_view() {
    let mut h = window(LayoutMode::Floating, 1400);
    click(&mut h, Item::Tickets);
    assert_eq!(open(&h.app), Some(Item::Tickets));
    let panel = rect_of("rail:panel").unwrap();
    let (place, _) = h.app.tickets.web_place.expect("the panel gives its place to the web view");
    assert!(panel.contains_rect(place) && place.width() >= panel.width() - 16.0 && place.top() <= panel.top() + 8.0, "the whole panel, its title row included: {place:?} in {panel:?}");
    assert!(h.widget(Item::Tickets.empty()).is_none(), "nothing drawn under the web view");
    crate::tickets::set_host(&mut h.app, false);
    h.settle();
    assert!(h.widget(Item::Tickets.empty()).is_some() && h.widget(Item::Tickets.note()).is_some(), "without a web view the panel says so");
}
