//! Drawing the Files panel: the folder's title, the tree, the questions a file operation asks,
//! and the results it reports.

use egui::{Align, Color32, FontId, Layout, Rect, Sense, Stroke, UiBuilder, pos2, vec2};
use solidcraft_engine::io::files::{self, Conflict, Kind};

use super::{EditKind, External, Listing, Modal, Row, RowState};
use crate::i18n::tr;
use crate::ui_kit::{self, Button};
use crate::{SolveApp, i18n::format as trf, theme::Tokens};

/// One row's height.
const ROW: f32 = 22.0;
/// One level of nesting.
const INDENT: f32 = 13.0;
/// The fold arrow's box.
const FOLD: f32 = 12.0;
/// The icon's box before the name.
const ICON: f32 = 14.0;
const HEADER: f32 = 30.0;

/// The id the tree takes keyboard focus under.
fn tree_id() -> egui::Id {
    egui::Id::new("sc_file_tree")
}

/// Draw the Files panel into `area` (the panel's body, below its title).
pub(crate) fn show(app: &mut SolveApp, ui: &mut egui::Ui, area: Rect) {
    crate::file_tree::tick(app, ui.ctx(), app.now);
    let t = Tokens::get();
    let mut child = ui.new_child(UiBuilder::new().max_rect(area.shrink2(vec2(t.space_8, t.space_6))).layout(Layout::top_down(Align::Min)));
    let ui = &mut child;
    if app.file_tree.root().is_none() {
        open_folder_state(app, ui);
        modals(app, ui.ctx());
        return;
    }
    header(app, ui);
    ui.add_space(t.space_4);
    external(app, ui);
    if app.file_tree.find.is_some() {
        find_row(app, ui);
    }
    let footer = footer_height(app);
    let list_height = (ui.available_height() - footer).max(ROW * 4.0);
    rows(app, ui, list_height);
    footer_rows(app, ui);
    modals(app, ui.ctx());
}

/// Nothing is open yet: say so and offer a folder.
fn open_folder_state(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let area = ui.available_rect_before_wrap();
    let width = (area.width() - t.space_24).max(80.0);
    let mut y = (area.center().y - 60.0).max(area.top() + t.space_16);
    let icon = Rect::from_center_size(pos2(area.center().x, y + 12.0), vec2(24.0, 24.0));
    paint_folder(ui.painter(), icon, t.text_dim);
    y += 34.0;
    let title = ui.painter().layout(tr("ui.files_no_folder").to_string(), FontId::proportional(t.font_label), t.text, width);
    ui.painter().galley(pos2(area.center().x - title.size().x / 2.0, y), title.clone(), t.text);
    y += title.size().y + t.space_6;
    let note = ui.painter().layout(tr("ui.files_no_folder_note").to_string(), FontId::proportional(t.font_small), t.text_dim, width);
    ui.painter().galley(pos2(area.center().x - note.size().x / 2.0, y), note.clone(), t.text_dim);
    y += note.size().y + t.space_12;
    let button = Rect::from_center_size(pos2(area.center().x, y + t.control_height / 2.0), vec2(140.0, t.control_height));
    let response = ui_kit::button_at(ui, "files_open_folder", button, tr("ui.files_open_folder"), Button::Primary);
    crate::scenario::publish_target("files_open_folder", response.rect);
    crate::scenario::publish_target("files:empty", area);
    if cfg!(target_arch = "wasm32") {
        ui_kit::text(ui, tr("ui.files_unavailable_web"), ui_kit::Text::Note(tr("ui.files_unavailable_web")));
        return;
    }
    if response.clicked() {
        pick_folder(app);
    }
}

fn pick_folder(app: &mut SolveApp) {
    app.pick_folder(|app, path| {
        let Some(path) = path else { return };
        let path = std::path::PathBuf::from(path);
        if path.is_dir() {
            app.file_tree.set_root(Some(path));
        } else if let Some(parent) = path.parent() {
            app.file_tree.set_root(Some(parent.to_path_buf()));
        }
    });
}

/// The root's name with the menu that acts on the folder itself.
fn header(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let Some(root) = app.file_tree.root.clone() else { return };
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER), Sense::hover());
    crate::scenario::publish_target("files:root", rect);
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
    let tooltip = root.display().to_string();
    let mut right = rect.right();
    let more = Rect::from_center_size(pos2(right - t.space_8, rect.center().y), vec2(20.0, 20.0));
    let more_response = ui_kit::button_at(ui, "files_more", more, "⋯", Button::Ghost).on_hover_text(tr("ui.files_root_menu"));
    crate::scenario::publish_target("files:more", more);
    right = more.left() - t.space_2;
    let refresh = Rect::from_center_size(pos2(right - t.space_8, rect.center().y), vec2(20.0, 20.0));
    let refresh_response = ui_kit::button_at(ui, "files_refresh", refresh, "⟳", Button::Ghost).on_hover_text(tr("ui.files_refresh"));
    crate::scenario::publish_target("files:refresh", refresh);
    right = refresh.left() - t.space_2;
    let plus = Rect::from_center_size(pos2(right - t.space_8, rect.center().y), vec2(20.0, 20.0));
    let plus_response = ui_kit::button_at(ui, "files_new", plus, "+", Button::Ghost).on_hover_text(tr("ui.files_new"));
    crate::scenario::publish_target("files:new", plus);
    right = plus.left() - t.space_6;
    let icon = Rect::from_min_size(pos2(rect.left() + t.space_2, rect.center().y - ICON / 2.0), vec2(ICON, ICON));
    paint_folder(ui.painter(), icon, t.text_dim);
    let name_rect = Rect::from_min_max(pos2(icon.right() + t.space_6, rect.top()), pos2(right, rect.bottom()));
    let font = FontId::proportional(t.font_label);
    let galley = ui.painter().layout_no_wrap(name, font, t.text);
    let painter = ui.painter().with_clip_rect(name_rect);
    painter.galley(pos2(name_rect.left(), name_rect.center().y - galley.size().y / 2.0), galley, t.text);
    let _ = ui.interact(name_rect, egui::Id::new("files_root_name"), Sense::hover()).on_hover_text(tooltip);

    if refresh_response.clicked() {
        app.file_tree.refresh(&root);
        app.file_tree.notice = None;
    }
    ui_kit::Menu::context(&plus_response, |ui| {
        if ui_kit::MenuItem::new("files_new_file", tr("ui.files_new_file"), "new").show(ui).clicked() {
            let folder = app.file_tree.target_folder().unwrap_or_else(|| root.clone());
            app.file_tree.start_new(&folder, EditKind::File);
            ui.close();
        }
        if ui_kit::MenuItem::new("files_new_folder", tr("ui.files_new_folder"), "folder").show(ui).clicked() {
            let folder = app.file_tree.target_folder().unwrap_or_else(|| root.clone());
            app.file_tree.start_new(&folder, EditKind::Folder);
            ui.close();
        }
        if ui_kit::MenuItem::new("files_new_model", tr("ui.files_new_model"), "new").show(ui).clicked() {
            let folder = app.file_tree.target_folder().unwrap_or_else(|| root.clone());
            app.file_tree.start_new(&folder, EditKind::Model);
            ui.close();
        }
        let paste = app.file_tree.clip.is_some() || cfg!(not(target_arch = "wasm32"));
        let mut item = ui_kit::MenuItem::new("files_paste", tr("ui.files_paste"), "paste");
        item.enabled = paste;
        item.shortcut = shortcut("V");
        if item.show(ui).clicked() {
            let folder = app.file_tree.target_folder().unwrap_or_else(|| root.clone());
            app.file_tree.paste(&folder);
            ui.close();
        }
    });
    ui_kit::Menu::context(&more_response, |ui| {
        let mut hidden = ui_kit::MenuItem::new("files_hidden", tr("ui.files_show_hidden"), "eye");
        hidden.checked = Some(app.file_tree.show_hidden);
        if hidden.show(ui).clicked() {
            app.file_tree.show_hidden = !app.file_tree.show_hidden;
            ui.close();
        }
        let mut select = ui_kit::MenuItem::new("files_select_all", tr("ui.files_select_all"), "select");
        select.shortcut = shortcut("A");
        if select.show(ui).clicked() {
            let rows = app.file_tree.rows();
            app.file_tree.selection = rows.iter().filter(|row| row.state == RowState::Entry).map(|row| row.path.clone()).collect();
            app.file_tree.focus = rows.iter().find(|row| row.state == RowState::Entry).map(|row| row.path.clone());
            ui.close();
        }
        if ui_kit::MenuItem::new("files_collapse", tr("ui.files_collapse_all"), "revert").show(ui).clicked() {
            app.file_tree.expanded.clear();
            app.file_tree.expanded.insert(root.clone());
            ui.close();
        }
        if ui_kit::MenuItem::new("files_refresh_item", tr("ui.files_refresh"), "revert").show(ui).clicked() {
            app.file_tree.refresh(&root);
            ui.close();
        }
        ui.separator();
        let active = app.session.path.clone();
        let mut locate = ui_kit::MenuItem::new("files_locate", tr("ui.files_locate"), "inspect");
        locate.enabled = active.as_deref().is_some_and(|path| std::path::Path::new(path).starts_with(&root));
        if locate.show(ui).clicked() {
            if let Some(path) = active {
                app.file_tree.reveal_path(std::path::Path::new(&path));
            }
            ui.close();
        }
        ui.separator();
        if ui_kit::MenuItem::new("files_copy_root_path", tr("ui.files_copy_path"), "copy").show(ui).clicked() {
            app.clipboard_text = Some(root.display().to_string());
            ui.close();
        }
        if ui_kit::MenuItem::new("files_copy_root_relative", tr("ui.files_copy_relative_path"), "copy").show(ui).clicked() {
            app.clipboard_text = Some(".".into());
            ui.close();
        }
        if ui_kit::MenuItem::new("files_reveal_root", tr("ui.files_reveal"), "folder").show(ui).clicked() {
            report_error(app, files::reveal(&root), tr("ui.files_reveal"));
            ui.close();
        }
        ui.separator();
        if ui_kit::MenuItem::new("files_switch_folder", tr("ui.files_switch_folder"), "folder").show(ui).clicked() {
            pick_folder(app);
            ui.close();
        }
    });
}

/// The open designs whose files changed under them.
fn external(app: &mut SolveApp, ui: &mut egui::Ui) {
    if app.file_tree.external.is_empty() {
        return;
    }
    let t = Tokens::get();
    let shown: Vec<External> = app.file_tree.external.clone();
    for change in shown.iter().take(2) {
        let name = std::path::Path::new(&change.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| change.path.clone());
        let message =
            if change.missing { trf("ui.files_missing_on_disk", &[("name", &name)]) } else { trf("ui.files_changed_on_disk", &[("name", &name)]) };
        let mut action = None;
        egui::Frame::NONE
            .fill(t.warning.gamma_multiply(0.12))
            .corner_radius(t.radius_sm)
            .inner_margin(egui::Margin::symmetric(t.space_6 as i8, t.space_4 as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui_kit::text(ui, &message, ui_kit::Text::Note(&message));
                if change.dirty {
                    ui_kit::text(ui, tr("ui.files_changed_dirty"), ui_kit::Text::Note(tr("ui.files_changed_dirty")));
                }
                ui.horizontal(|ui| {
                    if !change.missing && ui_kit::button(ui, tr("ui.files_reload"), Button::Small).clicked() {
                        action = Some(0);
                    }
                    if ui_kit::button(ui, tr("ui.files_save_as"), Button::Small).clicked() {
                        action = Some(1);
                    }
                    if ui_kit::button(ui, tr("ui.files_keep"), Button::Ghost).clicked() {
                        action = Some(2);
                    }
                });
            });
        match action {
            Some(0) => crate::file_tree::reload(app, &change.path, change.dirty),
            Some(1) => save_as_open(app, &change.path),
            Some(2) => app.file_tree.dismiss_external(&change.path),
            _ => {}
        }
        ui.add_space(t.space_4);
    }
    if app.file_tree.external.len() > 2 {
        let rest = trf("ui.files_more_changed", &[("n", &(app.file_tree.external.len() - 2).to_string())]);
        ui_kit::text(ui, &rest, ui_kit::Text::Note(&rest));
    }
    crate::scenario::publish_target("files:external", ui.min_rect());
}

/// Show a changed design's own tab, then ask Save As for it: the person chooses where the work
/// goes, and the design follows that file.
fn save_as_open(app: &mut SolveApp, path: &str) {
    if let Some(index) = crate::documents::index_at(app, path) {
        crate::documents::switch(app, index);
    }
    crate::toolbar::save_as(app);
}

/// The quick-locate field (Ctrl+F).
fn find_row(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    let mut text = app.file_tree.find.clone().unwrap_or_default();
    let before = text.clone();
    let response = ui_kit::cell_editor(ui, egui::Id::new("files_find"), &mut text, tr("ui.files_find_hint"), false);
    if response.changed() {
        app.file_tree.find = Some(text.clone());
    }
    let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    if escape {
        app.file_tree.find = None;
        return;
    }
    if before != text {
        locate(app, &text);
    }
    ui.add_space(t.space_4);
}

/// Select the first visible row whose name contains `text`.
fn locate(app: &mut SolveApp, text: &str) {
    let needle = text.to_lowercase();
    if needle.is_empty() {
        return;
    }
    let rows = app.file_tree.rows();
    if let Some(row) = rows.iter().find(|r| r.name.to_lowercase().contains(&needle)) {
        app.file_tree.select_only(&row.path);
        app.file_tree.scroll_to = Some(row.path.clone());
    }
}

/// The tree itself, in a scroll area that takes `height`.
fn rows(app: &mut SolveApp, ui: &mut egui::Ui, height: f32) {
    let t = Tokens::get();
    let visible: Vec<std::path::PathBuf> = app.file_tree.rows().iter().filter(|r| r.state == RowState::Entry).map(|r| r.path.clone()).collect();
    let open: std::collections::BTreeMap<String, bool> =
        crate::documents::open_sessions(app).into_iter().filter_map(|s| s.path.clone().map(|p| (p, s.is_dirty()))).collect();
    let mut edit = app.file_tree.editing.take();
    if let Some(draft) = edit.as_mut() {
        let depth = edit_depth(draft.parent.as_path(), &visible);
        match editing_row(ui, draft, depth) {
            Edited::Commit => {
                app.file_tree.editing = edit.take();
                let open = app.file_tree.commit_edit();
                if let Some(path) = open {
                    crate::file_tree::open_path(app, &path);
                }
            }
            Edited::Cancel => {
                app.file_tree.cancel_edit();
                edit = None;
            }
            Edited::Keep => {}
        }
    }
    let rows = app.file_tree.rows();
    if rows.is_empty() {
        ui.add_space(t.space_12);
        let note = if app.file_tree.root.as_deref().is_some_and(|root| app.file_tree.dirs.get(root).is_some_and(|d| d.state == Listing::Loading)) {
            tr("ui.files_loading")
        } else {
            tr("ui.files_folder_empty")
        };
        ui_kit::text(ui, note, ui_kit::Text::Note(note));
        ui.add_space(t.space_6);
        ui.horizontal(|ui| {
            if ui_kit::button(ui, tr("ui.files_new_file"), Button::Small).clicked()
                && let Some(folder) = app.file_tree.root.clone()
            {
                app.file_tree.start_new(&folder, EditKind::File);
            }
            if ui_kit::button(ui, tr("ui.files_new_folder"), Button::Small).clicked()
                && let Some(folder) = app.file_tree.root.clone()
            {
                app.file_tree.start_new(&folder, EditKind::Folder);
            }
        });
        crate::scenario::publish_target("files:empty_folder", ui.min_rect());
        if let Some(draft) = edit {
            app.file_tree.editing = Some(draft);
        }
        keys(app, ui, &visible);
        return;
    }
    let root = app.file_tree.root.clone().map(|root| root.to_string_lossy().into_owned()).unwrap_or_default();
    egui::ScrollArea::vertical().id_salt(("sc_files_rows", root)).auto_shrink([false, false]).max_height(height).show_rows(
        ui,
        ROW,
        rows.len(),
        |ui, range| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for index in range {
                let Some(row) = rows.get(index) else { continue };
                let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                row_ui(app, ui, rect, row, &visible, &open);
            }
            crate::scenario::publish_target("files:list", ui.min_rect());
        },
    );
    if let Some(draft) = edit {
        app.file_tree.editing = Some(draft);
    }
    if app.file_tree.drag.is_some() && ui.input(|i| i.pointer.any_released()) {
        if ui.rect_contains_pointer(ui.max_rect()) {
            let target = app.file_tree.drop_target.clone().or_else(|| app.file_tree.target_folder());
            let sources = app.file_tree.drag.take().unwrap_or_default();
            let copy = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
            app.file_tree.drop_target = None;
            if let Some(target) = target.filter(|_| !sources.is_empty()) {
                app.file_tree.place(sources, &target, !copy, None);
            }
        } else {
            app.file_tree.drag = None;
            app.file_tree.drop_target = None;
        }
    }
    keys(app, ui, &visible);
}

enum Edited {
    Keep,
    Commit,
    Cancel,
}

/// The inline name field, under the folder it belongs to.
fn editing_row(ui: &mut egui::Ui, draft: &mut super::Edit, depth: usize) -> Edited {
    let t = Tokens::get();
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW + 2.0), Sense::hover());
    ui.painter().rect_filled(rect, t.radius_sm, t.control_fill);
    let left = rect.left() + (depth as f32) * INDENT + FOLD + t.space_6;
    let field = Rect::from_min_max(pos2(left, rect.top() + 1.0), pos2(rect.right() - t.space_4, rect.bottom() - 1.0));
    let mut name = draft.name.clone();
    let response = ui
        .scope_builder(UiBuilder::new().max_rect(field).layout(Layout::left_to_right(Align::Center)), |ui| {
            ui_kit::cell_editor(ui, egui::Id::new(("files_edit", &draft.parent)), &mut name, "", draft.error.is_some())
        })
        .inner;
    if draft.focus {
        response.request_focus();
    }
    draft.focus = false;
    let entered = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) || ui.input(|i| i.key_pressed(egui::Key::Enter));
    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
    if name != draft.name {
        draft.name = name;
        draft.error = None;
    }
    crate::scenario::publish_target("files:edit", field);
    if escape {
        return Edited::Cancel;
    }
    if entered {
        return Edited::Commit;
    }
    Edited::Keep
}

/// One tree row: fold arrow, icon, name, and what the pointer and keyboard do on it.
fn row_ui(
    app: &mut SolveApp,
    ui: &mut egui::Ui,
    rect: Rect,
    row: &Row,
    visible: &[std::path::PathBuf],
    open: &std::collections::BTreeMap<String, bool>,
) {
    let t = Tokens::get();
    let is_dir = matches!(row.kind, Kind::Dir);
    let selected = app.file_tree.selection.contains(&row.path) || app.file_tree.focus.as_deref() == Some(row.path.as_path());
    let dropping = app.file_tree.drag.is_some() && app.file_tree.drop_target.as_deref() == Some(row.path.as_path());
    let response = ui.interact(rect, egui::Id::new(("files_row", &row.path)), Sense::click_and_drag());
    let fill = if dropping {
        t.accent_soft
    } else if selected {
        t.selected_bg
    } else if response.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    if fill != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, t.radius_sm, fill);
    }
    let ink = if selected { t.selected_ink } else { t.text };
    let dim = if selected { t.selected_ink } else { t.text_dim };
    let left = rect.left() + (row.depth as f32) * INDENT + t.space_4;
    let fold = Rect::from_center_size(pos2(left + FOLD / 2.0, rect.center().y), vec2(FOLD, FOLD));
    if is_dir {
        paint_fold(ui.painter(), fold, row.expanded, dim);
        let fold_response = ui.interact(fold, egui::Id::new(("files_fold", &row.path)), Sense::click());
        if fold_response.clicked() {
            app.file_tree.focus = Some(row.path.clone());
            ui.memory_mut(|m| m.request_focus(tree_id()));
            app.file_tree.toggle_dir(&row.path);
            return;
        }
    }
    let icon = Rect::from_center_size(pos2(fold.right() + t.space_4 + ICON / 2.0, rect.center().y), vec2(ICON, ICON));
    match row.kind {
        Kind::Dir => paint_folder(ui.painter(), icon, dim),
        Kind::Link { to_dir } => {
            if to_dir {
                paint_folder(ui.painter(), icon, dim);
            } else {
                paint_file(ui.painter(), icon, dim, false);
            }
            ui.painter().circle_filled(icon.right_top() + vec2(-1.0, 1.0), 2.0, dim);
        }
        Kind::File => paint_file(ui.painter(), icon, dim, row.name.to_ascii_lowercase().ends_with(".sc")),
    }
    let name_left = icon.right() + t.space_6;
    let name_rect = Rect::from_min_max(pos2(name_left, rect.top()), pos2(rect.right() - t.space_4, rect.bottom()));
    let mut color = ink;
    let mut suffix = String::new();
    if let Some(dirty) = open.get(&row.path.to_string_lossy().into_owned()) {
        suffix = if *dirty { " •".into() } else { " ○".into() };
        if !selected {
            color = t.text_dim;
        }
    }
    let text = format!("{}{suffix}", row.name);
    let galley = ui.painter().layout_no_wrap(text, FontId::proportional(t.font_small), color);
    ui.painter().with_clip_rect(name_rect).galley(pos2(name_rect.left(), name_rect.center().y - galley.size().y / 2.0), galley, color);
    response.clone().on_hover_text(row.path.display().to_string());
    crate::scenario::publish_target(&format!("files:row:{}", row.name), rect);
    match &row.state {
        RowState::Loading => {
            let label = tr("ui.files_loading");
            ui.painter().text(name_rect.center(), egui::Align2::CENTER_CENTER, label, FontId::proportional(t.font_small), t.text_dim);
        }
        RowState::Failed(error) => {
            let galley = ui.painter().layout_no_wrap(error.clone(), FontId::proportional(t.font_small), t.form_error);
            ui.painter().with_clip_rect(name_rect).galley(pos2(name_rect.left(), name_rect.center().y - galley.size().y / 2.0), galley, t.form_error);
            let retry = Rect::from_center_size(pos2(rect.right() - t.space_16, rect.center().y), vec2(44.0, ROW - 4.0));
            if ui_kit::button_at(ui, "files_retry", retry, tr("ui.files_retry"), Button::Small).clicked() {
                let dir = row.path.clone();
                app.file_tree.refresh(&dir);
            }
        }
        RowState::Entry => {}
    }

    let modifiers = ui.input(|i| i.modifiers);
    if response.clicked() {
        ui.memory_mut(|m| m.request_focus(tree_id()));
        app.file_tree.press = Some(row.path.clone());
        app.file_tree.click(&row.path, modifiers, visible);
    }
    if response.double_clicked()
        && let Some(path) = app.file_tree.activate(&row.path)
    {
        crate::file_tree::open_path(app, &path);
    }
    if response.drag_started() {
        if !app.file_tree.selection.contains(&row.path) {
            app.file_tree.select_only(&row.path);
        }
        app.file_tree.drag = Some(app.file_tree.selected(Some(&row.path)));
    }
    if app.file_tree.drag.is_some() && is_dir && response.hovered() {
        app.file_tree.drop_target = Some(row.path.clone());
    }
    if app.file_tree.scroll_to.as_deref() == Some(row.path.as_path()) {
        ui.scroll_to_rect(rect, Some(Align::Center));
        app.file_tree.scroll_to = None;
    }
    if response.secondary_clicked() || (response.clicked() && modifiers.command && cfg!(target_os = "macos")) {
        if !app.file_tree.selection.contains(&row.path) {
            app.file_tree.select_only(&row.path);
        }
        ui.memory_mut(|m| m.request_focus(tree_id()));
        context_menu(app, &response, row);
    }
}

/// The row's context menu: open, the clipboard, rename, delete, and where the file is.
fn context_menu(app: &mut SolveApp, response: &egui::Response, row: &Row) {
    let root = app.file_tree.root.clone();
    ui_kit::Menu::context(response, |ui| {
        let is_dir = row.kind.is_dir();
        let mut open = ui_kit::MenuItem::new("files_open", tr("ui.files_open"), if is_dir { "folder" } else { "open" });
        open.shortcut = "Enter";
        if open.show(ui).clicked() {
            if let Some(path) = app.file_tree.activate(&row.path) {
                crate::file_tree::open_path(app, &path);
            }
            ui.close();
        }
        if is_dir {
            let mut paste = ui_kit::MenuItem::new("files_paste_here", tr("ui.files_paste_into"), "paste");
            paste.shortcut = shortcut("V");
            if paste.show(ui).clicked() {
                app.file_tree.paste(&row.path);
                ui.close();
            }
        }
        ui.separator();
        let mut copy = ui_kit::MenuItem::new("files_copy", tr("ui.files_copy"), "copy");
        copy.shortcut = shortcut("C");
        if copy.show(ui).clicked() {
            app.file_tree.copy_selection(false);
            ui.close();
        }
        let mut cut = ui_kit::MenuItem::new("files_cut", tr("ui.files_cut"), "cut");
        cut.shortcut = shortcut("X");
        if cut.show(ui).clicked() {
            app.file_tree.copy_selection(true);
            ui.close();
        }
        ui.separator();
        let mut rename = ui_kit::MenuItem::new("files_rename", tr("ui.files_rename"), "rename");
        rename.shortcut = "F2";
        if rename.show(ui).clicked() {
            app.file_tree.start_rename(&row.path);
            ui.close();
        }
        let mut delete = ui_kit::MenuItem::new("files_delete", tr("ui.files_delete"), "delete");
        delete.danger = true;
        delete.shortcut = "Del";
        if delete.show(ui).clicked() {
            request_delete(app);
            ui.close();
        }
        ui.separator();
        if ui_kit::MenuItem::new("files_copy_path", tr("ui.files_copy_path"), "copy").show(ui).clicked() {
            app.clipboard_text = Some(row.path.display().to_string());
            ui.close();
        }
        if ui_kit::MenuItem::new("files_copy_relative", tr("ui.files_copy_relative_path"), "copy").show(ui).clicked() {
            let relative = root.as_deref().and_then(|root| row.path.strip_prefix(root).ok()).map(|p| p.display().to_string()).unwrap_or_default();
            app.clipboard_text = Some(relative);
            ui.close();
        }
        if ui_kit::MenuItem::new("files_reveal_row", tr("ui.files_reveal"), "folder").show(ui).clicked() {
            report_error(app, files::reveal(&row.path), tr("ui.files_reveal"));
            ui.close();
        }
    });
}

/// Ask about deleting the selection. The caller gathered the open designs' unsaved state.
pub(crate) fn request_delete(app: &mut SolveApp) {
    let open: Vec<(String, bool)> =
        crate::documents::open_sessions(app).into_iter().filter_map(|session| session.path.clone().map(|path| (path, session.is_dirty()))).collect();
    app.file_tree.request_delete(&open);
}

/// The keyboard: navigation, open, rename, delete, the clipboard and Select All. Shortcuts act
/// only while the tree has focus.
fn keys(app: &mut SolveApp, ui: &mut egui::Ui, visible: &[std::path::PathBuf]) {
    if app.file_tree.editing.is_some() {
        return;
    }
    let focused = ui.memory(|m| m.has_focus(tree_id()));
    let command = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
    if !focused {
        return;
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
        app.file_tree.find = Some(String::new());
        return;
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.file_tree.find = None;
        return;
    }
    let shift = ui.input(|i| i.modifiers.shift);
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
        move_focus(app, visible, 1, shift);
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)) {
        move_focus(app, visible, -1, shift);
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Home)) {
        jump_focus(app, visible, 0, shift);
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::End)) {
        jump_focus(app, visible, visible.len().saturating_sub(1), shift);
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
        if let Some(path) = app.file_tree.focus.clone()
            && matches!(app.file_tree.kind_at(&path), Some(Kind::Dir))
            && !app.file_tree.expanded.contains(&path)
        {
            app.file_tree.toggle_dir(&path);
        }
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
        if let Some(path) = app.file_tree.focus.clone() {
            if app.file_tree.expanded.contains(&path) {
                app.file_tree.toggle_dir(&path);
            } else if let Some(parent) = path.parent()
                && visible.iter().any(|p| p == parent)
            {
                app.file_tree.select_only(parent);
            }
        }
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        if let Some(path) = app.file_tree.focus.clone()
            && let Some(path) = app.file_tree.activate(&path)
        {
            crate::file_tree::open_path(app, &path);
        }
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
        if let Some(path) = app.file_tree.focus.clone() {
            app.file_tree.start_rename(&path);
        }
    } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        request_delete(app);
    } else if command && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::C)) {
        app.file_tree.copy_selection(false);
    } else if command && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::X)) {
        app.file_tree.copy_selection(true);
    } else if command && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::V)) {
        if let Some(folder) = app.file_tree.target_folder() {
            app.file_tree.paste(&folder);
        }
    } else if command && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        app.file_tree.selection = visible.iter().cloned().collect();
        app.file_tree.focus = visible.first().cloned();
        app.file_tree.anchor = visible.first().cloned();
    }
}

fn move_focus(app: &mut SolveApp, visible: &[std::path::PathBuf], delta: isize, extend: bool) {
    let current = app.file_tree.focus.as_ref().and_then(|focus| visible.iter().position(|p| p == focus));
    let next = match current {
        Some(index) => (index as isize + delta).clamp(0, visible.len().saturating_sub(1) as isize) as usize,
        None if delta >= 0 => 0,
        None => visible.len().saturating_sub(1),
    };
    select_at(app, visible, next, extend);
}

fn jump_focus(app: &mut SolveApp, visible: &[std::path::PathBuf], index: usize, extend: bool) {
    if !visible.is_empty() {
        select_at(app, visible, index.min(visible.len() - 1), extend);
    }
}

fn select_at(app: &mut SolveApp, visible: &[std::path::PathBuf], index: usize, extend: bool) {
    let Some(path) = visible.get(index) else { return };
    if extend {
        let modifiers = egui::Modifiers { shift: true, ..Default::default() };
        app.file_tree.click(path, modifiers, visible);
    } else {
        app.file_tree.select_only(path);
    }
    app.file_tree.scroll_to = Some(path.clone());
}

/// How many lines the notice and the batch report need.
fn footer_height(app: &SolveApp) -> f32 {
    let mut height = 0.0;
    if app.file_tree.busy {
        height += ROW + 4.0;
    }
    if app.file_tree.notice.is_some() {
        height += ROW;
    }
    if app.file_tree.report.is_some() {
        height += ROW * 2.0;
    }
    height.min(ROW * 5.0)
}

fn footer_rows(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    if app.file_tree.busy {
        ui.horizontal(|ui| {
            ui.spinner();
            ui_kit::text(ui, tr("ui.files_working"), ui_kit::Text::Note(tr("ui.files_working")));
            if ui_kit::button(ui, tr("ui.files_cancel"), Button::Small).clicked() {
                app.file_tree.cancel_operations();
            }
        });
    }
    if let Some((notice, error)) = app.file_tree.notice.clone() {
        ui.horizontal(|ui| {
            let style = if error { ui_kit::Text::Error(&notice) } else { ui_kit::Text::Note(&notice) };
            ui_kit::text(ui, &notice, style);
            if let Some(path) = app.file_tree.notice_reveal.clone()
                && ui_kit::button(ui, tr("ui.files_reveal"), Button::Small).clicked()
            {
                report_error(app, files::reveal(&path), tr("ui.files_reveal"));
                app.file_tree.notice_reveal = None;
            }
            if ui_kit::button(ui, "×", Button::Ghost).clicked() {
                app.file_tree.notice = None;
                app.file_tree.notice_reveal = None;
            }
        });
        crate::scenario::publish_target("files:notice", ui.min_rect());
    }
    let Some(report) = app.file_tree.report.as_ref() else { return };
    let (succeeded, failed, skipped, cancelled) = (report.succeeded(), report.failed(), report.skipped(), report.cancelled);
    let mut retry = report.retry.len();
    let details: Vec<String> = report
        .results
        .iter()
        .filter_map(|result| match &result.outcome {
            solidcraft_engine::io::files::Outcome::Failed(e) => Some(format!("{}: {e}", result.source.display())),
            _ => None,
        })
        .take(3)
        .collect();
    let summary = trf("ui.files_report", &[("done", &succeeded.to_string()), ("failed", &failed.to_string()), ("skipped", &skipped.to_string())]);
    let ink = if failed > 0 { t.form_error } else { t.text_dim };
    let mut dismiss = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(summary).font(FontId::proportional(t.font_small)).color(ink));
        if cancelled {
            ui_kit::text(ui, tr("ui.files_cancelled"), ui_kit::Text::Note(tr("ui.files_cancelled")));
        }
        if ui_kit::button(ui, "×", Button::Ghost).clicked() {
            dismiss = true;
        }
        if retry > 0 && ui_kit::button(ui, tr("ui.files_retry_failed"), Button::Small).clicked() {
            retry = usize::MAX;
        }
    });
    if !details.is_empty() {
        let text = details.join("\n");
        ui_kit::text(ui, &text, ui_kit::Text::Note(&text));
    }
    if retry == usize::MAX {
        app.file_tree.retry_failed();
    } else if dismiss {
        app.file_tree.report = None;
    }
    crate::scenario::publish_target("files:report", ui.min_rect());
}

/// The questions a file operation asks.
fn modals(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(modal) = app.file_tree.modal.take() else { return };
    match modal {
        Modal::Delete { paths, dirty } => delete_modal(app, ctx, paths, dirty),
        Modal::Permanent { paths, reason } => permanent_modal(app, ctx, paths, reason),
        Modal::Conflict { sources, destination, cut, names, files_only } => conflict_modal(app, ctx, sources, destination, cut, names, files_only),
        Modal::DiscardReload { path } => discard_reload_modal(app, ctx, path),
        Modal::ReplaceOpen { sources, destination, cut, dirty } => replace_open_modal(app, ctx, sources, destination, cut, dirty),
    }
}

fn names_summary(names: &[String], total: usize) -> String {
    let shown: Vec<&str> = names.iter().take(4).map(String::as_str).collect();
    let mut text = shown.join(", ");
    if total > shown.len() {
        text.push_str(&format!(", +{}", total - shown.len()));
    }
    text
}

fn delete_modal(app: &mut SolveApp, ctx: &egui::Context, paths: Vec<std::path::PathBuf>, dirty: Vec<String>) {
    let names: Vec<String> = paths.iter().map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()).collect();
    let title = if paths.len() == 1 { tr("ui.files_delete_title") } else { tr("ui.files_delete_many_title") };
    let summary = trf("ui.files_delete_count", &[("n", &paths.len().to_string()), ("names", &names_summary(&names, paths.len()))]);
    let dirty_note = (!dirty.is_empty()).then(|| trf("ui.files_delete_dirty", &[("n", &dirty.len().to_string())]));
    let ok = if dirty.is_empty() { tr("ui.files_move_to_trash") } else { tr("ui.files_discard_and_delete") };
    let left = (!dirty.is_empty()).then_some(tr("ui.files_save_copies_delete"));
    let (closed, action) = ui_kit::Confirm::new(egui::Id::new("sc_files_delete"), "delete", title)
        .tone(ui_kit::Tone::Danger)
        .actions(ok, tr("ui.files_cancel"), left)
        .show(ctx, true, |ui| {
            ui_kit::text(ui, &summary, ui_kit::Text::Note(&summary));
            if let Some(note) = &dirty_note {
                ui_kit::text(ui, note, ui_kit::Text::Error(note));
            }
        });
    match action {
        ui_kit::Action::Ok => app.file_tree.start_trash(paths),
        ui_kit::Action::Apply => match crate::file_tree::save_copies(app, &paths) {
            Ok(_) => app.file_tree.start_trash(paths),
            Err(e) => {
                app.file_tree.notice = Some((trf("ui.files_save_copies_failed", &[("reason", &e)]), true));
                app.file_tree.modal = Some(Modal::Delete { paths, dirty });
            }
        },
        _ if !closed => app.file_tree.modal = Some(Modal::Delete { paths, dirty }),
        _ => {}
    }
}

fn permanent_modal(app: &mut SolveApp, ctx: &egui::Context, paths: Vec<std::path::PathBuf>, reason: String) {
    let names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    let title = tr("ui.files_delete_permanent_title");
    let summary = trf("ui.files_permanent_count", &[("n", &paths.len().to_string()), ("names", &names_summary(&names, paths.len()))]);
    let (closed, action) = ui_kit::Confirm::new(egui::Id::new("sc_files_permanent"), "delete", title)
        .tone(ui_kit::Tone::Danger)
        .actions(tr("ui.files_delete_permanently"), tr("ui.files_cancel"), None)
        .show(ctx, true, |ui| {
            ui_kit::text(ui, &reason, ui_kit::Text::Error(&reason));
            ui_kit::text(ui, &summary, ui_kit::Text::Note(&summary));
        });
    match action {
        ui_kit::Action::Ok => app.file_tree.delete_permanently(paths),
        _ if !closed => app.file_tree.modal = Some(Modal::Permanent { paths, reason }),
        _ => {}
    }
}

fn conflict_modal(
    app: &mut SolveApp,
    ctx: &egui::Context,
    sources: Vec<std::path::PathBuf>,
    destination: std::path::PathBuf,
    cut: bool,
    names: Vec<String>,
    files_only: bool,
) {
    let title = tr("ui.files_conflict_title");
    let summary = trf("ui.files_conflict_count", &[("n", &names.len().to_string()), ("names", &names_summary(&names, names.len()))]);
    let note = if files_only { tr("ui.files_conflict_files") } else { tr("ui.files_conflict_folders") };
    let mut skip = false;
    let left = files_only.then_some(tr("ui.files_replace"));
    let (closed, action) = ui_kit::Confirm::new(egui::Id::new("sc_files_conflict"), "warning", title)
        .tone(ui_kit::Tone::Warning)
        .actions(tr("ui.files_keep_both"), tr("ui.files_cancel"), left)
        .show(ctx, true, |ui| {
            ui_kit::text(ui, &summary, ui_kit::Text::Note(&summary));
            ui_kit::text(ui, note, ui_kit::Text::Note(note));
            if ui_kit::button(ui, tr("ui.files_skip"), Button::Small).clicked() {
                skip = true;
            }
        });
    let restore = |app: &mut SolveApp| {
        app.file_tree.modal =
            Some(Modal::Conflict { sources: sources.clone(), destination: destination.clone(), cut, names: names.clone(), files_only });
    };
    if skip {
        app.file_tree.resolve_conflict(sources, destination, cut, Conflict::Skip);
        return;
    }
    match action {
        ui_kit::Action::Apply if files_only => app.file_tree.resolve_conflict(sources, destination, cut, Conflict::Replace),
        ui_kit::Action::Ok => app.file_tree.resolve_conflict(sources, destination, cut, Conflict::KeepBoth),
        _ if !closed => restore(app),
        _ => {}
    }
}

fn replace_open_modal(
    app: &mut SolveApp,
    ctx: &egui::Context,
    sources: Vec<std::path::PathBuf>,
    destination: std::path::PathBuf,
    cut: bool,
    dirty: Vec<String>,
) {
    let count = dirty.len();
    let message = trf("ui.files_replace_dirty", &[("n", &count.to_string())]);
    let (closed, action) = ui_kit::Confirm::new(egui::Id::new("sc_files_replace_open"), "warning", tr("ui.files_replace"))
        .tone(ui_kit::Tone::Danger)
        .actions(tr("ui.files_discard_and_replace"), tr("ui.files_cancel"), Some(tr("ui.files_save_copies_replace")))
        .show(ctx, true, |ui| {
            ui_kit::text(ui, &message, ui_kit::Text::Error(&message));
        });
    let restore = |app: &mut SolveApp| {
        app.file_tree.modal = Some(Modal::ReplaceOpen { sources: sources.clone(), destination: destination.clone(), cut, dirty: dirty.clone() });
    };
    match action {
        ui_kit::Action::Ok => app.file_tree.resolve_conflict(sources, destination, cut, Conflict::Replace),
        ui_kit::Action::Apply => match crate::file_tree::save_copies(app, &dirty.iter().map(std::path::PathBuf::from).collect::<Vec<_>>()) {
            Ok(_) => app.file_tree.resolve_conflict(sources, destination, cut, Conflict::Replace),
            Err(e) => {
                app.file_tree.notice = Some((trf("ui.files_save_copies_failed", &[("reason", &e)]), true));
                restore(app);
            }
        },
        _ if !closed => restore(app),
        _ => {}
    }
}

fn discard_reload_modal(app: &mut SolveApp, ctx: &egui::Context, path: String) {
    let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone());
    let message = trf("ui.files_reload_dirty", &[("name", &name)]);
    let (closed, action) = ui_kit::Confirm::new(egui::Id::new("sc_files_reload"), "revert", tr("ui.files_reload"))
        .tone(ui_kit::Tone::Warning)
        .actions(tr("ui.files_discard_and_reload"), tr("ui.files_cancel"), None)
        .show(ctx, true, |ui| {
            ui_kit::text(ui, &message, ui_kit::Text::Note(&message));
        });
    match action {
        ui_kit::Action::Ok => crate::file_tree::reload_now(app, &path),
        _ if !closed => app.file_tree.modal = Some(Modal::DiscardReload { path }),
        _ => {}
    }
}

/// The depth a new or renamed entry is drawn at: inside its folder's row.
fn edit_depth(parent: &std::path::Path, visible: &[std::path::PathBuf]) -> usize {
    // A folder's children stand one level in. The rows are walked depth first, so the parent's
    // nesting is counted from the visible folders that hold it.
    visible.iter().filter(|p| parent.starts_with(p)).count()
}

fn paint_fold(painter: &egui::Painter, rect: Rect, open: bool, ink: Color32) {
    let c = rect.center();
    let points = if open {
        vec![pos2(c.x - 3.5, c.y - 2.0), pos2(c.x + 3.5, c.y - 2.0), pos2(c.x, c.y + 2.5)]
    } else {
        vec![pos2(c.x - 2.0, c.y - 3.5), pos2(c.x + 2.5, c.y), pos2(c.x - 2.0, c.y + 3.5)]
    };
    painter.add(egui::Shape::convex_polygon(points, ink, Stroke::NONE));
}

fn paint_folder(painter: &egui::Painter, rect: Rect, ink: Color32) {
    let stroke = Stroke::new(1.2, ink);
    let tab = Rect::from_min_max(rect.min, pos2(rect.center().x - 1.0, rect.top() + 3.0));
    painter.rect_filled(tab, 1.0, ink);
    painter.rect_stroke(Rect::from_min_max(pos2(rect.left(), rect.top() + 3.0), rect.max), 1.5, stroke, egui::StrokeKind::Middle);
}

fn paint_file(painter: &egui::Painter, rect: Rect, ink: Color32, design: bool) {
    let stroke = Stroke::new(1.2, ink);
    let page = Rect::from_min_max(rect.min + vec2(2.0, 0.5), rect.max - vec2(2.0, 0.5));
    painter.rect_stroke(page, 1.5, stroke, egui::StrokeKind::Middle);
    if design {
        painter.rect_filled(Rect::from_center_size(page.center() + vec2(0.0, 1.0), vec2(4.0, 4.0)), 0.5, ink);
    } else {
        let y = page.top() + 4.0;
        painter.line_segment([pos2(page.left() + 2.0, y), pos2(page.right() - 2.0, y)], stroke);
        painter.line_segment([pos2(page.left() + 2.0, y + 3.0), pos2(page.right() - 3.0, y + 3.0)], stroke);
    }
}

fn shortcut(key: &str) -> &'static str {
    match (cfg!(target_vendor = "apple"), key) {
        (true, "C") => "⌘C",
        (true, "X") => "⌘X",
        (true, "V") => "⌘V",
        (true, "A") => "⌘A",
        (false, "C") => "Ctrl+C",
        (false, "X") => "Ctrl+X",
        (false, "V") => "Ctrl+V",
        (false, "A") => "Ctrl+A",
        _ => "",
    }
}

fn report_error(app: &mut SolveApp, result: Result<(), String>, what: &str) {
    if let Err(error) = result {
        app.set_status(trf("ui.files_action_failed", &[("action", what), ("reason", &error)]), true);
    }
}
