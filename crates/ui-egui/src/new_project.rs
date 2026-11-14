//! The New Project window: a name and the folder to make the project in.
//!
//! Create runs `project.create`, which makes the project folder and saves the open design into
//! it. A refusal keeps the window open with its message and leaves nothing on disk.

use serde_json::json;

use crate::{SolveApp, ui_kit};

/// Whether the window makes a new folder or adds `.intgral/` beside an existing design.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    New,
    Beside { design: String },
}

/// What is being typed, and the last refusal from `project.create` or `project.attach`.
pub struct NewProject {
    pub name: String,
    pub folder: String,
    pub mode: Mode,
    pub error: Option<String>,
}

/// Open the window with the active design's name and the last chosen folder (else the home
/// folder).
pub fn open(app: &mut SolveApp) {
    let folder = app.last_project_folder.clone().or_else(home_folder).unwrap_or_default();
    app.new_project = Some(NewProject { name: app.session.doc.name.clone(), folder, mode: Mode::New, error: None });
    app.home.open = false;
}

/// Open the same window for a saved design that has no project beside it. The design's folder is
/// fixed; the name is the design's.
pub fn open_beside(app: &mut SolveApp, design: &str) {
    let path = std::path::Path::new(design);
    let name = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("design").to_string();
    let folder = path.parent().map(|parent| parent.display().to_string()).unwrap_or_default();
    app.new_project = Some(NewProject { name, folder, mode: Mode::Beside { design: design.to_string() }, error: None });
    app.home.open = false;
}

/// Ask the host for a folder. Cancelling leaves the window and its folder as they were.
pub(crate) fn choose_folder(app: &mut SolveApp) {
    app.pick_folder(|app, path| {
        if let Some(path) = path
            && let Some(dialog) = app.new_project.as_mut()
        {
            dialog.folder = path;
            dialog.error = None;
        }
    });
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.new_project.take() else { return };
    let mut choose = false;
    let title = crate::i18n::tr("new_project.title");
    let beside = matches!(dialog.mode, Mode::Beside { .. });
    let valid = beside || (!dialog.name.trim().is_empty() && !dialog.folder.trim().is_empty());
    let (closed, action) = ui_kit::Window::new(egui::Id::new("sc_new_project"), "new", title)
        .actions(crate::i18n::tr("ui.create"), crate::i18n::tr("ui.cancel"), None)
        .fields()
        .show(ctx, valid, |ui| {
            let name = crate::i18n::tr("new_project.name");
            ui_kit::row(ui, name, |ui| {
                let _ = ui_kit::text_field(ui, &mut dialog.name, "");
            });
            let location = crate::i18n::tr("new_project.location");
            ui_kit::row(ui, location, |ui| {
                if !beside && ui_kit::button(ui, crate::i18n::tr("new_project.choose"), ui_kit::Button::Normal).clicked() {
                    choose = true;
                }
                ui_kit::text(ui, &dialog.folder, ui_kit::Text::Note("folder"));
            });
            if let Mode::Beside { design } = &dialog.mode {
                let hint = crate::i18n::format("new_project.beside_hint", &[("design", design)]);
                ui_kit::text(ui, &hint, ui_kit::Text::Note("beside"));
            }
            if let Some(error) = &dialog.error {
                ui_kit::text(ui, error, ui_kit::Text::Report("error", true));
            }
        });
    if closed || action == ui_kit::Action::Cancel {
        return;
    }
    if action == ui_kit::Action::Ok {
        let folder = dialog.folder.trim().to_string();
        let result = match &dialog.mode {
            Mode::New => app.run("project.create", json!({ "name": dialog.name.trim(), "folder": folder })),
            Mode::Beside { design } => app.run("project.attach", json!({ "path": design })),
        };
        match result {
            Ok(_) => {
                // A new project saved the open design into it; its tab is this design.
                app.last_project_folder = Some(folder);
                return;
            }
            Err(error) => dialog.error = Some(error),
        }
    }
    app.new_project = Some(dialog);
    if choose {
        choose_folder(app);
    }
}

/// The home folder a new project starts in.
fn home_folder() -> Option<String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(|home| home.to_string_lossy().into_owned())
}
