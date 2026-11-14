//! macOS system menus, owned and updated on eframe's main thread.

use std::sync::mpsc::{Receiver, sync_channel};

use muda::{
    AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use solidcraft_ui_egui::{SolveApp, toolbar::FileAction};

pub struct NativeMenu {
    menu: Menu,
    actions: Vec<(MenuItem, FileAction)>,
    help_actions: Vec<(MenuItem, &'static str)>,
    events: Receiver<MenuEvent>,
    recent: Submenu,
    recent_paths: Vec<String>,
    edit: Submenu,
    /// The text editing items, in the Edit menu only while a native text view has the keyboard.
    text_editing: Vec<PredefinedMenuItem>,
    /// Undo (false) and Redo (true) chosen while that view had the keyboard: they are its text's.
    text_edits: Vec<bool>,
}

impl NativeMenu {
    pub fn new(app: &SolveApp, ctx: &egui::Context) -> muda::Result<Self> {
        let menu = Menu::new();
        let application = Submenu::new("Intcad", true);
        let file = Submenu::new(app.tr("ui.file"), true);
        let edit = Submenu::new(app.tr_label("Edit"), true);
        let recent = Submenu::new(app.tr("ui.open_recent"), true);
        let window = Submenu::new("Window", true);
        let help = Submenu::new(app.tr("ui.help"), true);
        let mut help_actions = Vec::new();
        let mut actions = Vec::new();
        let mut add = |parent: &Submenu, label: &str, action: FileAction, shortcut: Option<Accelerator>| -> muda::Result<()> {
            let item = MenuItem::new(label, Self::action_enabled(&action, app, ctx), shortcut);
            parent.append(&item)?;
            actions.push((item, action));
            Ok(())
        };
        application.append(&PredefinedMenuItem::about(
            Some("About Intcad"),
            Some(AboutMetadata { name: Some("Intcad".into()), version: Some(env!("CARGO_PKG_VERSION").into()), ..Default::default() }),
        ))?;
        application.append(&PredefinedMenuItem::separator())?;
        add(
            &application,
            &format!("{}\u{2026}", app.tr("ui.preferences")),
            FileAction::Settings,
            Some(Accelerator::new(Modifiers::META, Code::Comma)),
        )?;
        add(&application, &format!("{}\u{2026}", app.tr("ui.keyboard_shortcuts")), FileAction::Shortcuts, None)?;
        application.append(&PredefinedMenuItem::separator())?;
        application.append(&PredefinedMenuItem::services(None))?;
        application.append(&PredefinedMenuItem::separator())?;
        application.append_items(&[
            &PredefinedMenuItem::hide(Some("Hide Intcad")),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
        ])?;
        add(&application, &format!("{} Intcad", app.tr("ui.quit")), FileAction::Quit, Some(Accelerator::new(Modifiers::META, Code::KeyQ)))?;
        add(&file, app.tr("ui.new_design"), FileAction::New, Some(Accelerator::new(Modifiers::META, Code::KeyN)))?;
        add(&file, &format!("{}\u{2026}", app.tr("ui.open")), FileAction::Open, Some(Accelerator::new(Modifiers::META, Code::KeyO)))?;
        file.append(&recent)?;
        file.append(&PredefinedMenuItem::separator())?;
        for (label, command) in
            [("Insert STEP\u{2026}", "file.insert_step"), ("Insert Mesh\u{2026}", "file.insert_mesh"), ("Insert Design", "component.insert_design")]
        {
            add(&file, app.tr_label(label), FileAction::Command(command), None)?;
        }
        file.append(&PredefinedMenuItem::separator())?;
        add(&file, app.tr("ui.save"), FileAction::Save, Some(Accelerator::new(Modifiers::META, Code::KeyS)))?;
        add(
            &file,
            &format!("{}\u{2026}", app.tr("ui.save_as")),
            FileAction::SaveAs,
            Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyS)),
        )?;
        add(&file, app.tr_label("Save As JSON\u{2026}"), FileAction::SaveJson, None)?;
        file.append(&PredefinedMenuItem::separator())?;
        for (label, action) in [
            ("Export STL\u{2026}", FileAction::ExportMesh("stl")),
            ("Export 3MF\u{2026}", FileAction::ExportMesh("3mf")),
            ("Export STEP\u{2026}", FileAction::Export("step")),
            ("Export Current View as DXF\u{2026}", FileAction::Command("file.export_view_dxf")),
            ("Export OBJ\u{2026}", FileAction::Export("obj")),
            ("Export URDF\u{2026}", FileAction::Export("urdf")),
            ("Export Design as JSON\u{2026}", FileAction::Export("json")),
        ] {
            add(&file, app.tr_label(label), action, None)?;
        }
        file.append(&PredefinedMenuItem::separator())?;
        add(&file, app.tr_label("Design Properties"), FileAction::Command("document.metadata"), None)?;
        add(&edit, app.tr("ui.undo"), FileAction::Undo, Some(Accelerator::new(Modifiers::META, Code::KeyZ)))?;
        add(&edit, app.tr("ui.redo"), FileAction::Redo, Some(Accelerator::new(Modifiers::META | Modifiers::SHIFT, Code::KeyZ)))?;
        add(&window, app.tr_label("Agent Chat"), FileAction::AgentChat, None)?;
        window.append(&PredefinedMenuItem::separator())?;
        window.append_items(&[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::maximize(None),
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ])?;
        for (action, label) in [
            ("docs", "Documentation"),
            ("updates", app.tr("update.check_for_updates")),
            ("shortcuts", "Keyboard Shortcuts"),
            ("messages", "Messages\u{2026}"),
            ("report", "Report an Issue\u{2026}"),
            ("crashes", "Crash Reports\u{2026}"),
            ("install_cli", "Install solidcraft-cli command"),
        ] {
            let item = MenuItem::new(app.tr_label(label), true, None);
            help.append(&item)?;
            help_actions.push((item, action));
        }
        menu.append_items(&[&application, &file, &edit, &window, &help])?;
        let (sender, events) = sync_channel(64);
        let ctx = ctx.clone();
        MenuEvent::set_event_handler(Some(move |event| {
            let _ = sender.try_send(event);
            ctx.request_repaint();
        }));
        menu.init_for_nsapp();
        let mut native =
            Self { menu, actions, help_actions, events, recent, recent_paths: Vec::new(), edit, text_editing: Vec::new(), text_edits: Vec::new() };
        native.update_recent(app)?;
        Ok(native)
    }

    pub fn raw_input_hook(&mut self, app: &mut SolveApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
        for event in self.events.try_iter().take(64) {
            if let Some((_, action)) = self.help_actions.iter().find(|(item, _)| item.id() == &event.id) {
                solidcraft_ui_egui::help::run(app, ctx, action);
            }
            if let Some((_, action)) = self.actions.iter().find(|(item, _)| item.id() == &event.id) {
                if matches!(action, FileAction::Undo | FileAction::Redo) && !self.text_editing.is_empty() {
                    // The Assistant's page has the keyboard: Undo and Redo are for the text being
                    // typed there, never for the design.
                    self.text_edits.push(matches!(action, FileAction::Redo));
                } else if matches!(action, FileAction::Undo | FileAction::Redo) && ctx.egui_wants_keyboard_input() {
                    raw.events.push(egui::Event::Key {
                        key: egui::Key::Z,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers { command: true, mac_cmd: true, shift: matches!(action, FileAction::Redo), ..Default::default() },
                    });
                } else {
                    action.run(app);
                }
            }
        }
    }

    /// Cut, Copy, Paste and Select All for a native text view that has the keyboard (the
    /// Assistant's web view): macOS sends those keys through menu items. They are in the Edit
    /// menu only meanwhile, so that otherwise the same keys reach the window as they always have.
    pub fn set_text_editing(&mut self, app: &SolveApp, on: bool) {
        if on == !self.text_editing.is_empty() {
            return;
        }
        if on {
            let items = [
                PredefinedMenuItem::separator(),
                PredefinedMenuItem::cut(Some(app.tr("ui.cut"))),
                PredefinedMenuItem::copy(Some(app.tr("ui.copy"))),
                PredefinedMenuItem::paste(Some(app.tr("ui.paste"))),
                PredefinedMenuItem::select_all(Some(app.tr("ui.select_all"))),
            ];
            for item in items {
                match self.edit.append(&item) {
                    Ok(()) => self.text_editing.push(item),
                    Err(error) => log::warn!("Edit menu: {error}"),
                }
            }
        } else {
            for item in self.text_editing.drain(..) {
                if let Err(error) = self.edit.remove(&item) {
                    log::warn!("Edit menu: {error}");
                }
            }
        }
    }

    /// Undo (false) and Redo (true) chosen since the last call while the native text view had
    /// the keyboard, for that view to carry out.
    pub fn take_text_edits(&mut self) -> Vec<bool> {
        std::mem::take(&mut self.text_edits)
    }

    fn action_enabled(action: &FileAction, app: &SolveApp, ctx: &egui::Context) -> bool {
        action.menu_enabled(app, ctx.egui_wants_keyboard_input())
    }

    pub fn tick(&mut self, app: &mut SolveApp, ctx: &egui::Context) {
        for (item, action) in &self.actions {
            let enabled = Self::action_enabled(action, app, ctx);
            if item.is_enabled() != enabled {
                item.set_enabled(enabled);
            }
        }
        for (item, action) in &self.help_actions {
            let enabled = solidcraft_ui_egui::help::available(app, action);
            if item.is_enabled() != enabled {
                item.set_enabled(enabled);
            }
        }
        if !self.recent_paths.iter().eq(app.home.recent.iter().take(32))
            && let Err(error) = self.update_recent(app)
        {
            app.set_status(format!("Recent designs menu: {error}"), true);
        }
    }

    fn update_recent(&mut self, app: &SolveApp) -> muda::Result<()> {
        while self.recent.remove_at(0).is_some() {}
        self.actions.retain(|(_, action)| !matches!(action, FileAction::OpenRecent(_) | FileAction::ClearRecent));
        self.recent_paths = app.home.recent.iter().take(32).cloned().collect();
        for path in &self.recent_paths {
            let action = FileAction::OpenRecent(path.clone());
            let item = MenuItem::new(path, action.enabled(app), None);
            self.recent.append(&item)?;
            self.actions.push((item, action));
        }
        self.recent.append(&PredefinedMenuItem::separator())?;
        let clear = MenuItem::new(app.tr("ui.clear_recent"), !self.recent_paths.is_empty(), None);
        self.recent.append(&clear)?;
        self.actions.push((clear, FileAction::ClearRecent));
        Ok(())
    }
}

impl Drop for NativeMenu {
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
        self.menu.remove_for_nsapp();
    }
}
