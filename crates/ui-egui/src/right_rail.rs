//! The right rail (Preferences › Interface › Right rail; on by default): a narrow strip along the
//! window's right edge with four items. Files stands first, for what is in the design's project;
//! under a rule come Review, Simulation and Autocomplete. An item opens its panel beside the
//! rail, one panel at a time; the same item closes it again. The panel can be dragged wider, and
//! its width and the open item are remembered.
//!
//! The rail is window chrome like the application bar and the status bar: it is allocated before
//! the workspace in either layout, so the view, the ribbon and every panel and overlay anchored to
//! the view's right side (the Assistant, the axis view, tool sheets) stand to its left without
//! knowing of it. Its panel is a workspace panel hosted on the same edge.
//!
//! It is drawn the way the Assistant's own sidebar is: an item is a small line drawing with its
//! name under it, quiet until it is pointed at, and the open one stands on a neutral fill, not
//! the selection colour. A panel is a title over a rule and, while it has nothing to list, one
//! centred block: the item's drawing, what is missing, and a sentence on what will be listed.
//!
//! Nothing fills the panels yet. They hold no sample data.

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, pos2, vec2};

use crate::SolveApp;
use crate::i18n::tr;
use crate::theme::Tokens;
use crate::ui_kit::{self, BackingTooltipExt as _, Container, Edge, Surface};

/// The rail's narrowest and widest: it is as wide as the longest name of its items needs.
pub(crate) const MIN_WIDTH: f32 = 56.0;
pub(crate) const MAX_WIDTH: f32 = 88.0;
/// An item's height: its drawing over its name.
const ITEM_HEIGHT: f32 = 48.0;
/// The side of an item's drawing on the rail, and in a panel's empty block.
const ICON: f32 = 16.0;
const EMPTY_ICON: f32 = 20.0;
/// The height of a panel's title strip.
const HEADER: f32 = 44.0;
/// The widest an empty block's text runs before it wraps.
const EMPTY_TEXT: f32 = 320.0;
/// The open panel's narrowest, widest and first width.
pub(crate) const MIN_PANEL: f32 = 280.0;
pub(crate) const MAX_PANEL: f32 = 560.0;
pub(crate) const DEFAULT_PANEL: f32 = 320.0;
/// The width the view keeps beside the panel. In a window too narrow for both, the panel is not
/// drawn and the rail still marks its item, so the view never shrinks to nothing.
pub(crate) const MIN_VIEW: f32 = 240.0;

/// The rail's items, in the order they stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    Files,
    Review,
    Simulation,
    Autocomplete,
}

impl Item {
    pub(crate) const ALL: [Item; 4] = [Item::Files, Item::Review, Item::Simulation, Item::Autocomplete];

    /// The name the open item is stored by.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Item::Files => "files",
            Item::Review => "review",
            Item::Simulation => "simulation",
            Item::Autocomplete => "autocomplete",
        }
    }

    /// The item a stored name means; anything else (an empty name, a name from another version, a
    /// hand-edited file) means no panel is open.
    pub(crate) fn parse(id: &str) -> Option<Item> {
        Item::ALL.into_iter().find(|item| item.id() == id)
    }

    /// The short name on the rail.
    pub(crate) fn name(self) -> &'static str {
        tr(match self {
            Item::Files => "rail.files",
            Item::Review => "rail.review",
            Item::Simulation => "rail.simulation",
            Item::Autocomplete => "rail.autocomplete",
        })
    }

    /// The panel's title, which is also the item's tooltip and accessible name.
    pub(crate) fn title(self) -> &'static str {
        tr(match self {
            Item::Files => "rail.files_title",
            Item::Review => "rail.review_title",
            Item::Simulation => "rail.simulation_title",
            Item::Autocomplete => "rail.autocomplete_title",
        })
    }

    /// What the panel is missing while it has nothing to list.
    pub(crate) fn empty(self) -> &'static str {
        tr(match self {
            Item::Files => "rail.files_empty",
            Item::Review => "rail.review_empty",
            Item::Simulation => "rail.simulation_empty",
            Item::Autocomplete => "rail.autocomplete_empty",
        })
    }

    /// The sentence under it: why there is nothing yet, and what will be listed.
    pub(crate) fn note(self) -> &'static str {
        tr(match self {
            Item::Files => "rail.files_note",
            Item::Review => "rail.review_note",
            Item::Simulation => "rail.simulation_note",
            Item::Autocomplete => "rail.autocomplete_note",
        })
    }
}

/// The rail is drawn: the preference is on, beside a design in the Design workspace of the
/// application's own window. Home has no design to look at, the Render workspace has its own
/// panels on that edge, and an embed has no chrome.
pub(crate) fn shown(app: &SolveApp) -> bool {
    app.preferences.right_rail && !app.embedded && !app.embed_editor && !app.home.open && !crate::render_workspace::active(app)
}

/// The item whose panel is open.
pub(crate) fn open(app: &SolveApp) -> Option<Item> {
    Item::parse(&app.preferences.right_rail_open)
}

/// Open `item`'s panel, or close it when it is the open one.
pub(crate) fn toggle(app: &mut SolveApp, item: Item) {
    app.preferences.right_rail_open = if open(app) == Some(item) { String::new() } else { item.id().to_string() };
}

/// The stored panel width, in range: a preferences file can hold anything.
pub(crate) fn panel_width(app: &SolveApp) -> f32 {
    let width = app.preferences.right_rail_width;
    if width.is_finite() { width.clamp(MIN_PANEL, MAX_PANEL) } else { DEFAULT_PANEL }
}

/// The rail's width in the application's language: room for the longest of its items' names.
pub(crate) fn width(ctx: &egui::Context) -> f32 {
    let t = Tokens::get();
    let font = name_font(&t);
    let longest = ctx.fonts_mut(|fonts| {
        Item::ALL.map(|item| fonts.layout_no_wrap(item.name().to_string(), font.clone(), t.text).size().x).into_iter().fold(0.0, f32::max)
    });
    (longest + t.space_12 * 2.0).ceil().clamp(MIN_WIDTH, MAX_WIDTH)
}

fn name_font(t: &Tokens) -> FontId {
    FontId::proportional(t.font_micro)
}

fn panel_id() -> egui::Id {
    egui::Id::new("sc_right_rail_panel")
}

/// The id an item takes keyboard focus under.
fn item_id(item: Item) -> egui::Id {
    egui::Id::new(("sc_right_rail_item", item.id()))
}

/// Draw the rail and the open panel along the window's right edge. Call it after the application
/// bar and the status bar and before the workspace takes the rest of the window.
pub(crate) fn show(app: &mut SolveApp, ui: &mut egui::Ui) {
    app.shell.right_rail = None;
    app.shell.right_rail_panel = None;
    if !shown(app) {
        return;
    }
    let t = Tokens::get();
    let bar = Container::new(ui.ctx(), Surface::ApplicationBar);
    let strip = bar.edge("sc_right_rail", Edge::Right).exact_size(width(ui.ctx())).resizable(false).show_separator_line(false);
    let rail = bar.show_edge(ui, strip, bar.edge_frame(t.panel), |ui| items(app, ui)).response.rect;
    crate::scenario::publish_target("rail:strip", rail);
    app.shell.right_rail = Some(rail);

    let Some(item) = open(app) else { return };
    let room = ui.available_rect_before_wrap().width() - MIN_VIEW;
    if room.is_nan() || room < MIN_PANEL {
        return;
    }
    let widest = MAX_PANEL.min(room);
    let want = panel_width(app).min(widest);
    // The preference is the one memory of the width. The panel is laid out from it on every
    // frame, so a width set from outside (the control channel, a restored preferences file)
    // shows at once, and a panel squeezed by a narrow window has its chosen width back in a
    // wider one. The stock panel's own memory of its last size would win over both.
    let ctx = ui.ctx().clone();
    ctx.data_mut(|data| data.remove::<egui::containers::panel::PanelState>(panel_id()));
    let host = Container::new(&ctx, Surface::Panel);
    let pane = host.edge(panel_id(), Edge::Right).default_size(want).size_range(MIN_PANEL..=widest).resizable(true);
    let drawn = host.show_edge(ui, pane, host.edge_frame(t.panel), |ui| ui_kit::scope(ui, |ui| panel(ui, item))).response.rect;
    crate::scenario::publish_target("rail:panel", drawn);
    app.shell.right_rail = Some(rail.union(drawn));
    let grab = ctx.global_style().interaction.resize_grab_radius_side;
    app.shell.right_rail_panel = Some(drawn.with_min_x(drawn.left() - grab));
    // Only a drag on the panel's edge makes it another width than the one asked for.
    let width = drawn.width();
    if width.is_finite() && (width - want).abs() > 0.5 {
        app.preferences.right_rail_width = width.clamp(MIN_PANEL, MAX_PANEL);
    }
}

/// The items, from the top: Files, a rule, then the three ways of working on the design.
fn items(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let r = ui.max_rect();
    let painter = ui.painter().clone();
    painter.vline(r.left() + 0.5, r.y_range(), t.primary_border(t.border));
    let current = open(app);
    let font = name_font(&t);
    let mut top = r.top() + t.space_8;
    for item in Item::ALL {
        if item == Item::Review {
            painter.hline((r.left() + t.space_12)..=(r.right() - t.space_12), top + t.space_6, t.primary_border(t.border));
            top += t.space_12 + 1.0;
        }
        let rect = Rect::from_min_size(pos2(r.left() + t.space_6, top), vec2((r.width() - t.space_12).max(0.0), ITEM_HEIGHT));
        top = rect.bottom() + t.space_4;
        if rect.bottom() > r.bottom() {
            break;
        }
        let response = ui.interact(rect, item_id(item), Sense::click());
        let on = current == Some(item);
        let lit = on || response.hovered() || response.has_focus();
        // A row is quiet or current, never tinted: the open item stands on the neutral fill.
        if on {
            painter.rect_filled(rect, t.radius_lg, t.hover);
        } else if lit {
            painter.rect_filled(rect, t.radius_lg, t.hover.gamma_multiply(0.5));
        }
        if response.has_focus() {
            painter.rect_stroke(rect, t.radius_lg, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let ink = if lit { t.text } else { t.text_dim };
        drawing(&painter, item, pos2(rect.center().x, rect.top() + t.space_8 + ICON / 2.0), ICON, ink);
        let galley = painter.layout_no_wrap(item.name().to_string(), font.clone(), ink);
        painter.galley(pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - t.space_6 - galley.size().y), galley, ink);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, on, item.title()));
        crate::scenario::publish_target(&format!("rail:item:{}", item.id()), rect);
        if response.backing_on_hover_text(item.title()).clicked() {
            toggle(app, item);
        }
    }
}

/// An item's drawing: a few strokes in `ink`, `side` wide and high around `centre`, laid out on
/// a 16-unit square. Strokes end and meet round, as the Assistant's own icons do.
fn drawing(painter: &egui::Painter, item: Item, centre: Pos2, side: f32, ink: Color32) {
    let unit = side / 16.0;
    let at = |x: f32, y: f32| centre + vec2(x - 8.0, y - 8.0) * unit;
    let stroke = Stroke::new(1.5 * unit.max(1.0), ink);
    // `ring`: the last point is joined to the first.
    let line = |points: &[(f32, f32)], ring: bool| {
        let points: Vec<Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
        for (i, from) in points.iter().enumerate() {
            painter.circle_filled(*from, stroke.width / 2.0, ink);
            match points.get(i + 1).or(points.first().filter(|_| ring)) {
                Some(to) => painter.line_segment([*from, *to], stroke),
                None => continue,
            };
        }
    };
    match item {
        // A folder: its tab and its body.
        Item::Files => line(&[(1.5, 3.5), (6.0, 3.5), (7.75, 5.5), (14.5, 5.5), (14.5, 12.5), (1.5, 12.5)], true),
        // A sheet on a clipboard with a tick: something looked over.
        Item::Review => {
            painter.rect_stroke(Rect::from_min_max(at(3.0, 3.5), at(13.0, 14.0)), 1.5 * unit, stroke, egui::StrokeKind::Middle);
            painter.rect_filled(Rect::from_min_max(at(5.75, 2.0), at(10.25, 4.75)), unit, ink);
            line(&[(5.6, 9.3), (7.3, 11.0), (10.5, 7.4)], false);
        }
        // A response over time: a level line with one swing.
        Item::Simulation => line(&[(1.5, 8.5), (4.5, 8.5), (6.5, 3.0), (9.5, 13.5), (11.5, 8.5), (14.5, 8.5)], false),
        // A spark and a small plus: something offered before it is asked for.
        Item::Autocomplete => {
            for diamond in [[(6.5, 3.5), (7.9, 9.0), (6.5, 14.5), (5.1, 9.0)], [(1.0, 9.0), (6.5, 7.6), (12.0, 9.0), (6.5, 10.4)]] {
                painter.add(egui::Shape::convex_polygon(diamond.map(|(x, y)| at(x, y)).to_vec(), ink, Stroke::NONE));
            }
            line(&[(10.6, 3.6), (14.2, 3.6)], false);
            line(&[(12.4, 1.8), (12.4, 5.4)], false);
        }
    }
}

/// One panel: its title over a rule and, while there is nothing to list, the empty block.
fn panel(ui: &mut egui::Ui, item: Item) {
    let t = Tokens::get();
    let full = ui.max_rect();
    let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER.min(full.height())));
    let title = egui::UiBuilder::new().max_rect(header.shrink2(vec2(t.space_16, 0.0))).layout(egui::Layout::left_to_right(egui::Align::Center));
    ui.scope_builder(title, |ui| ui_kit::text(ui, item.title(), ui_kit::Text::Heading(item.id())));
    ui.painter().hline(full.x_range(), header.bottom() - 0.5, t.primary_border(t.border));
    empty(ui, Rect::from_min_max(pos2(full.left(), header.bottom()), full.max), item);
    ui.allocate_rect(full, Sense::hover());
}

/// The empty block, centred in `area`: the item's drawing, what is missing, and the sentence
/// under it. Each of the two texts is a label of its own for a screen reader.
fn empty(ui: &mut egui::Ui, area: Rect, item: Item) {
    let t = Tokens::get();
    let wrap = (area.width() - t.space_16 * 2.0).clamp(0.0, EMPTY_TEXT);
    let centred = |text: &str, size: f32, ink: Color32| {
        let mut job = egui::text::LayoutJob::simple(text.to_string(), FontId::proportional(size), ink, wrap);
        job.halign = egui::Align::Center;
        ui.painter().layout_job(job)
    };
    let (title, note) = (centred(item.empty(), t.font_label, t.text), centred(item.note(), t.font_small, t.text_dim));
    let height = EMPTY_ICON + t.space_12 + title.size().y + t.space_6 + note.size().y;
    let x = area.center().x;
    let mut y = (area.center().y - height / 2.0).max(area.top() + t.space_24);
    let painter = ui.painter().with_clip_rect(area);
    drawing(&painter, item, pos2(x, y + EMPTY_ICON / 2.0), EMPTY_ICON, t.text_dim);
    y += EMPTY_ICON + t.space_12;
    for (galley, text, gap, ink, what) in [(title, item.empty(), t.space_6, t.text, "empty"), (note, item.note(), 0.0, t.text_dim, "note")] {
        let rect = Rect::from_center_size(pos2(x, y + galley.size().y / 2.0), galley.size()).intersect(area);
        let advance = galley.size().y + gap;
        painter.galley(pos2(x, y), galley, ink);
        let response = ui.interact(rect, egui::Id::new(("sc_right_rail_empty", item.id(), what)), Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, text));
        y += advance;
    }
}

#[cfg(test)]
#[path = "right_rail_tests.rs"]
mod tests;
