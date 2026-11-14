//! Project Settings: process, material, thickness, automatic checks, the selected DFM data and
//! the project's named overrides.
//!
//! A window, not a command dialog: programmatic `project.inspect` returns its reply without
//! opening anything, and this window is opened only from the app (`ui.start`).

use serde_json::{Value, json};

use crate::{SolveApp, ui_kit};

/// What the window shows and what is being typed.
pub struct ProjectSettings {
    pub has_project: bool,
    pub root: String,
    pub process: String,
    pub material: String,
    pub thickness: String,
    pub auto_check: bool,
    pub skip: String,
    pub data_kind: String,
    pub data_name: String,
    pub data_version: String,
    pub data_available: bool,
    pub data_unavailable: Option<String>,
    pub overrides: Vec<Row>,
    pub new_rule: String,
    pub new_value: String,
    pub new_reason: String,
    pub new_by: String,
    pub new_date: String,
    pub error: Option<String>,
    pub notice: Option<String>,
}

/// One override as the window lists it.
#[derive(Clone)]
pub struct Row {
    pub rule: String,
    pub limit: String,
    pub value: String,
    pub reason: String,
    pub by: String,
    pub date: String,
}

impl ProjectSettings {
    fn from_reply(reply: &Value) -> Self {
        let project = &reply["project"];
        let ci = &reply["ci"];
        let thickness = project["thicknessMm"].as_f64().map(|value| format!("{value}")).unwrap_or_default();
        let skip = ci["skip"].as_array().map(|rules| rules.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let overrides = ci["overrides"]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|over| Row {
                        rule: text(&over["rule"]),
                        limit: text(&over["limit"]),
                        value: over["value"]
                            .as_object()
                            .map(|values| values.iter().map(|(name, number)| format!("{name} {number}")).collect::<Vec<_>>().join(", "))
                            .unwrap_or_default(),
                        reason: text(&over["reason"]),
                        by: text(&over["by"]),
                        date: text(&over["date"]),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            has_project: reply["hasProject"].as_bool().unwrap_or(false),
            root: text(&reply["root"]),
            process: text(&project["process"]),
            material: text(&project["material"]),
            thickness,
            auto_check: ci["enabled"].as_bool().unwrap_or(true),
            skip,
            data_kind: text(&ci["dataKind"]),
            data_name: text(&ci["dataName"]),
            data_version: text(&ci["dataVersion"]),
            data_available: ci["dataAvailable"].as_bool().unwrap_or(false),
            data_unavailable: ci["dataUnavailable"].as_str().map(str::to_string),
            overrides,
            new_rule: String::new(),
            new_value: String::new(),
            new_reason: String::new(),
            new_by: String::new(),
            new_date: String::new(),
            error: None,
            notice: None,
        }
    }

    fn reload(&mut self, reply: &Value) {
        let fresh = Self::from_reply(reply);
        let (new_rule, new_value, new_reason, new_by, new_date) =
            (self.new_rule.clone(), self.new_value.clone(), self.new_reason.clone(), self.new_by.clone(), self.new_date.clone());
        *self = fresh;
        (self.new_rule, self.new_value, self.new_reason, self.new_by, self.new_date) = (new_rule, new_value, new_reason, new_by, new_date);
    }
}

fn text(value: &Value) -> String {
    value.as_str().map(str::to_string).unwrap_or_default()
}

/// Ask the engine for the open design's project and open the window.
pub fn open(app: &mut SolveApp) {
    match app.session.execute("project.inspect", &json!({})) {
        Ok(reply) => {
            let settings = ProjectSettings::from_reply(&reply);
            if !settings.has_project {
                app.set_status("This design is not in a project. Make one with New Project first.", true);
                return;
            }
            app.project_settings = Some(settings);
        }
        Err(error) => app.set_status(format!("Project settings: {error}"), true),
    }
}

/// Run a project command and show its refusal or reload the window from its reply.
fn run(app: &mut SolveApp, dialog: &mut ProjectSettings, id: &str, params: Value) -> bool {
    match app.run(id, params) {
        Ok(reply) => {
            dialog.error = None;
            if id != "project.settings_set" && id != "project.ci_set" && id != "project.inspect" {
                dialog.notice = Some("Saved.".into());
            }
            if id == "dfm.override" {
                match app.session.execute("project.inspect", &json!({})) {
                    Ok(fresh) => dialog.reload(&fresh),
                    Err(error) => dialog.error = Some(error.to_string()),
                }
            } else {
                dialog.reload(&reply);
            }
            true
        }
        Err(error) => {
            dialog.error = Some(error);
            false
        }
    }
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut dialog) = app.project_settings.take() else { return };
    let mut import = false;
    let mut generic = false;
    let mut remove: Option<Row> = None;
    let mut add = false;
    let title = crate::i18n::tr("cmd.project.inspect");
    let (closed, action) = ui_kit::Window::new(egui::Id::new("sc_project_settings"), "settings", title)
        .actions(crate::i18n::tr("ui.save"), crate::i18n::tr("ui.close"), None)
        .fields()
        .show(ctx, true, |ui| {
            ui_kit::text(ui, &dialog.root, ui_kit::Text::Note("root"));
            ui_kit::row(ui, crate::i18n::tr("project_settings.process"), |ui| {
                let _ = ui_kit::text_field(ui, &mut dialog.process, "");
            });
            ui_kit::row(ui, crate::i18n::tr_label("Material"), |ui| {
                let _ = ui_kit::text_field(ui, &mut dialog.material, "5052-H32");
            });
            ui_kit::row(ui, crate::i18n::tr_label("Thickness"), |ui| {
                let _ = ui_kit::text_field(ui, &mut dialog.thickness, "2.0");
            });
            if ui_kit::checkbox(ui, crate::i18n::tr("project_settings.auto_check"), &mut dialog.auto_check).changed() {
                let wanted = dialog.auto_check;
                if !run(app, &mut dialog, "project.ci_set", json!({"enabled": wanted})) {
                    dialog.auto_check = !wanted;
                }
            }
            ui_kit::row(ui, crate::i18n::tr("project_settings.skip"), |ui| {
                let _ = ui_kit::text_field(ui, &mut dialog.skip, "sm.hole_spacing, …");
            });

            ui_kit::divider(ui, "data");
            let data = format!("{} {} {}", dialog.data_kind, dialog.data_name, dialog.data_version);
            ui_kit::text(ui, &data, ui_kit::Text::Cell);
            if let Some(reason) = &dialog.data_unavailable {
                let why = crate::i18n::format("project_settings.not_covered", &[("reason", reason)]);
                ui_kit::text(ui, &why, ui_kit::Text::Report("data_error", true));
            }
            ui_kit::label_actions(ui, crate::i18n::tr("project_settings.data"), |ui| {
                if ui_kit::button(ui, crate::i18n::tr_label("Import"), ui_kit::Button::Normal).clicked() {
                    import = true;
                }
                if ui_kit::button(ui, crate::i18n::tr("project_settings.use_generic"), ui_kit::Button::Normal).clicked() {
                    generic = true;
                }
            });

            ui_kit::divider(ui, "overrides");
            ui_kit::text(ui, crate::i18n::tr("project_settings.overrides_note"), ui_kit::Text::Note("override_note"));
            for row in dialog.overrides.clone() {
                ui_kit::label_actions(
                    ui,
                    &format!("{} ({}): {} — {} / {} {}", row.rule, row.limit, row.value, row.reason, row.by, row.date),
                    |ui| {
                        if ui_kit::button(ui, crate::i18n::tr_label("Remove"), ui_kit::Button::Normal).clicked() {
                            remove = Some(row.clone());
                        }
                    },
                );
            }
            ui_kit::section(ui, "project_add_override", crate::i18n::tr("project_settings.add_override"), false, |ui| {
                ui_kit::row(ui, crate::i18n::tr_label("Rule"), |ui| {
                    let _ = ui_kit::text_field(ui, &mut dialog.new_rule, "sm.hole_to_edge");
                });
                ui_kit::row(ui, crate::i18n::tr_label("Value"), |ui| {
                    let _ = ui_kit::text_field(ui, &mut dialog.new_value, "2.5");
                });
                ui_kit::row(ui, crate::i18n::tr("project_settings.reason"), |ui| {
                    let _ = ui_kit::text_field(ui, &mut dialog.new_reason, "");
                });
                ui_kit::row(ui, crate::i18n::tr("project_settings.by"), |ui| {
                    let _ = ui_kit::text_field(ui, &mut dialog.new_by, "");
                });
                ui_kit::row(ui, crate::i18n::tr("project_settings.date"), |ui| {
                    let _ = ui_kit::text_field(ui, &mut dialog.new_date, "2026-11-14");
                });
                if ui_kit::button(ui, crate::i18n::tr_label("Add"), ui_kit::Button::Normal).clicked() {
                    add = true;
                }
            });
            if let Some(error) = &dialog.error {
                ui_kit::text(ui, error, ui_kit::Text::Report("error", true));
            }
            if let Some(notice) = &dialog.notice {
                ui_kit::text(ui, notice, ui_kit::Text::Note("notice"));
            }
        });

    let mut keep = !(closed || action == ui_kit::Action::Cancel);
    if action == ui_kit::Action::Ok {
        let thickness = dialog.thickness.trim().to_string();
        let mut params = json!({
            "process": dialog.process.clone(),
            "material": dialog.material.clone(),
            "autoCheck": dialog.auto_check,
        });
        params["clearThickness"] = json!(thickness.is_empty());
        if let Ok(value) = thickness.parse::<f64>() {
            params["thicknessMm"] = json!(value);
        }
        let saved = run(app, &mut dialog, "project.settings_set", params);
        if saved {
            let skip: Vec<String> = dialog.skip.split(',').map(str::trim).filter(|rule| !rule.is_empty()).map(str::to_string).collect();
            let enabled = dialog.auto_check;
            run(app, &mut dialog, "project.ci_set", json!({"enabled": enabled, "skip": skip}));
        }
        keep = !saved;
    }
    if let Some(row) = remove {
        run(app, &mut dialog, "dfm.override", json!({"rule": row.rule, "limit": row.limit, "remove": true}));
    }
    if add {
        let value = dialog.new_value.trim().parse::<f64>();
        match value {
            Ok(mm) => {
                let rule = dialog.new_rule.trim().to_string();
                let reason = dialog.new_reason.trim().to_string();
                let by = dialog.new_by.trim().to_string();
                let date = dialog.new_date.trim().to_string();
                let done = run(
                    app,
                    &mut dialog,
                    "dfm.override",
                    json!({
                        "rule": rule,
                        "limit": "min",
                        "value": {"mm": mm},
                        "reason": reason,
                        "by": by,
                        "date": date,
                    }),
                );
                if done {
                    dialog.new_rule.clear();
                    dialog.new_value.clear();
                    dialog.new_reason.clear();
                    dialog.new_by.clear();
                }
            }
            Err(_) => dialog.error = Some("The override needs a number in mm.".into()),
        }
    }
    if keep {
        app.project_settings = Some(dialog);
    }
    if import {
        app.pick_file("DFM data", &["json"], |app, path| {
            let Some(path) = path else { return };
            let params = json!({"kind": "factory", "path": path, "upgrade": true});
            match app.run("project.data_set", params) {
                Ok(_) => open(app),
                Err(error) => {
                    if let Some(dialog) = app.project_settings.as_mut() {
                        dialog.error = Some(error);
                    }
                }
            }
        });
    }
    if generic && let Err(error) = app.run("project.data_set", json!({"kind": "generic"})) {
        if let Some(dialog) = app.project_settings.as_mut() {
            dialog.error = Some(error);
        } else {
            app.set_status(error, true);
        }
    }
}
