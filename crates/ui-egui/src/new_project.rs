//! The New Project window: a name and the folder to make the project in.
//!
//! Create runs `project.create`, which makes the project folder and saves the open design into
//! it. A refusal keeps the window open with its message and leaves nothing on disk.

use serde_json::json;

use crate::{SolveApp, ui_kit};

/// What is being typed, and the last refusal from `project.create`.
pub struct NewProject {
    pub name: String,
    pub folder: String,
    pub error: Option<String>,
}

/// Open the window with the active design's name and the last chosen folder (else the home
/// folder).
pub fn open(app: &mut SolveApp) {
    let folder = app.last_project_folder.clone().or_else(home_folder).unwrap_or_default();
    app.new_project = Some(NewProject { name: app.session.doc.name.clone(), folder, error: None });
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
    let valid = !dialog.name.trim().is_empty() && !dialog.folder.trim().is_empty();
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
                if ui_kit::button(ui, crate::i18n::tr("new_project.choose"), ui_kit::Button::Normal).clicked() {
                    choose = true;
                }
                ui_kit::text(ui, &dialog.folder, ui_kit::Text::Note("folder"));
            });
            if let Some(error) = &dialog.error {
                ui_kit::text(ui, error, ui_kit::Text::Report("error", true));
            }
        });
    if closed || action == ui_kit::Action::Cancel {
        return;
    }
    if action == ui_kit::Action::Ok {
        let folder = dialog.folder.trim().to_string();
        match app.run("project.create", json!({ "name": dialog.name.trim(), "folder": folder })) {
            Ok(_) => {
                // The command saved the open design into the project, so its tab is this design.
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
