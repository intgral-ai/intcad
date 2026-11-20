use super::*;
use crate::documents;
use solidcraft_engine::Session;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

static COUNT: AtomicU32 = AtomicU32::new(0);

/// A real sketch selection and CAD clipboard compete with Files. Older tests only injected
/// synthetic keys with an empty design, hiding the native event's second owner.
fn clipboard_window(root: &std::path::Path) -> crate::scenario::Harness {
    use serde_json::json;
    let mut h = crate::scenario::Harness::new();
    h.app.home.open = false;
    h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    let curves = h.app.run("sketch.rectangle.two_point", json!({"p0": [0, 0], "p1": [20, 10]})).unwrap()["curves"].clone();
    let items: Vec<_> = curves.as_array().unwrap().iter().map(|id| json!({"type": "sketch_curve", "id": id})).collect();
    h.app.run("select.set", json!({"items": items})).unwrap();
    assert!(crate::edit_clipboard::copy(&mut h.app));
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.app.file_tree = listing(root);
    h.frames(5);
    h
}

fn cad_mark(h: &crate::scenario::Harness) -> (String, String, usize, u64) {
    (
        serde_json::to_string(&h.app.session.doc).unwrap(),
        serde_json::to_string(&h.app.session.selection).unwrap(),
        h.app.session.undo.len(),
        h.app.clipboard_stamp,
    )
}

fn pointer_click(pos: egui::Pos2, modifiers: Modifiers) -> Vec<egui::Event> {
    vec![
        egui::Event::ModifiersChanged(modifiers),
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers },
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers },
    ]
}

/// Characterize the actual egui 0.36 text focus boundary without any application hooks,
/// Files state, CAD clipboard, custom field wrappers or clipboard finalizer.
#[test]
fn plain_egui_text_paste_after_click_compares_one_batch_with_two_frames() {
    fn observe(separate_frames: bool) -> String {
        let ctx = egui::Context::default();
        let mut text = String::new();
        let mut rect = egui::Rect::NOTHING;
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 1.0 / 60.0;
            let _ = ctx.run_ui(
                egui::RawInput {
                    focused: true,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 200.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    rect = ui.add(egui::TextEdit::singleline(&mut text).id(egui::Id::new("plain_text"))).rect;
                },
            );
            rect
        };
        let field = frame(Vec::new());
        let mut events = pointer_click(field.center(), Modifiers::NONE);
        if separate_frames {
            frame(events);
            frame(vec![egui::Event::Paste("needle".into())]);
        } else {
            events.push(egui::Event::Paste("needle".into()));
            frame(events);
        }
        text
    }
    let same_batch = observe(false);
    let separate_frames = observe(true);
    eprintln!("plain egui 0.36.2 same-batch click+Paste field text = {same_batch:?}");
    eprintln!("plain egui 0.36.2 click frame then Paste frame field text = {separate_frames:?}");
    assert_eq!(same_batch, "", "egui 0.36 acquires click focus after reading keyboard events");
    assert_eq!(separate_frames, "needle", "the plain already-focused widget receives one insertion");
}

#[test]
fn native_clipboard_events_in_files_do_not_touch_the_competing_cad_clipboard() {
    for cut in [false, true] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let mut h = clipboard_window(scratch.path());
        h.app.file_tree.select_only(&source);
        h.app.file_tree.focused = true;
        let before = cad_mark(&h);
        h.frame_with(vec![if cut { egui::Event::Cut } else { egui::Event::Copy }]);
        let clip = h.app.file_tree.clip.as_ref().expect("native Files copy/cut");
        assert_eq!(clip.paths, [source]);
        assert_eq!(clip.cut, cut);
        assert_eq!(cad_mark(&h), before, "Files must be the only owner");
    }
}

#[test]
fn same_frame_row_click_then_native_paste_uses_the_accepted_row_not_cad() {
    for text in ["", "arbitrary text is not a path"] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let into = scratch.dir("into");
        let mut h = clipboard_window(scratch.path());
        h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
        h.app.file_tree.focused = false;
        let row = crate::scenario::rect_of("files:row:into").unwrap();
        let before = cad_mark(&h);
        let mut events = pointer_click(row.center(), Modifiers::NONE);
        events.push(egui::Event::Paste(text.into()));
        h.frame_with(events);
        assert!(h.app.file_tree.busy, "the accepted row click must own Paste");
        h.frames(30);
        assert_eq!(fs::read(into.join("source.txt")).unwrap(), b"source");
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(cad_mark(&h), before);
        assert!(!into.join("source (copy).txt").exists(), "no replayed second paste");
    }
}

#[test]
fn find_text_click_then_paste_in_the_next_frame_belongs_only_to_the_real_text_widget() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.find = Some(String::new());
    h.frames(3);
    h.app.file_tree.select_only(&source);
    h.app.file_tree.focused = true;
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: true });
    let field = h.ctx.read_response(egui::Id::new("files_find")).unwrap().rect;
    let before = cad_mark(&h);
    // Plain egui's characterization above establishes that click focus must precede the
    // keyboard batch. Do not reconstruct or replay its pointer prefix in application code.
    h.frame_with(pointer_click(field.center(), Modifiers::NONE));
    assert!(h.ctx.text_edit_focused());
    let ctx = h.ctx.clone();
    let mut raw = window_input(&ctx, true, vec![egui::Event::Paste("needle".into())]);
    h.app.raw_input_hook(&mut raw);
    assert!(matches!(raw.events.as_slice(), [egui::Event::Paste(text)] if text == "needle"));
    let _ = ctx.run_ui(raw, |ui| {
        h.app.logic(ui.ctx());
        assert!(h.app.clipboard_input.entry_keyboard_owned, "exercise the broad early toolbar guard");
        assert!(
            ui.input(|i| matches!(i.events.as_slice(), [egui::Event::Paste(text)] if text == "needle")),
            "early logic/toolbar must leave Paste for the focused widget"
        );
        eprintln!("focused text Paste trace: raw hook retained Paste; early guard retained Paste");
        h.app.ui(ui);
    });
    h.frames(3);
    assert_eq!(h.app.file_tree.find.as_deref(), Some("needle"), "real text edit receives Paste once");
    assert_eq!(cad_mark(&h), before, "text Paste must not also paste the CAD clipboard");
    assert!(!h.app.file_tree.busy);
    assert!(h.app.file_tree.modal.is_none());
    assert!(h.app.file_tree.clip.as_ref().unwrap().cut, "the file clipboard was not copied or pasted");
    assert_eq!(fs::read(source).unwrap(), b"source");
}

/// The plain-egui focus boundary permits no insertion in this batch, not permission for
/// Files or CAD to take the event. Exercise the actual widgets with competing clipboards.
#[test]
fn same_batch_find_or_rename_click_paste_has_no_background_owner_or_replay() {
    for rename in [false, true] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let into = scratch.dir("into");
        let kept = into.join("keep.txt");
        fs::write(&kept, "kept").unwrap();
        let mut h = clipboard_window(scratch.path());
        if rename {
            h.app.file_tree.start_rename(&source);
        } else {
            h.app.file_tree.find = Some(String::new());
        }
        h.frames(3);
        h.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        let field =
            if rename { crate::scenario::rect_of("files:edit").unwrap() } else { h.ctx.read_response(egui::Id::new("files_find")).unwrap().rect };
        h.app.file_tree.select_only(&into);
        h.app.file_tree.focused = true;
        h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: true });
        let before = cad_mark(&h);
        let selection = h.app.file_tree.selection.clone();
        let focus = h.app.file_tree.focus.clone();
        let notice = h.app.file_tree.notice.clone();
        let mut events = pointer_click(field.center(), Modifiers::NONE);
        events.push(egui::Event::Paste("needle".into()));
        h.frame_with(events);
        // Do not require same-batch insertion that plain egui itself cannot provide.
        assert!(h.ctx.text_edit_focused(), "the actual text widget acquired click focus");
        h.frames(5);
        assert_eq!(cad_mark(&h), before, "rename={rename}: no CAD mutation or replay");
        assert_eq!(h.app.file_tree.selection, selection);
        assert_eq!(h.app.file_tree.focus, focus);
        let clip = h.app.file_tree.clip.as_ref().unwrap();
        assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&source));
        assert!(clip.cut);
        assert_eq!(h.app.file_tree.notice, notice);
        assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none());
        assert!(h.app.file_tree.report.is_none() && h.app.file_tree.take_answers().is_empty(), "no queued file job");
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read(&kept).unwrap(), b"kept");
        assert!(!into.join("source.txt").exists());
        let names: Vec<_> = fs::read_dir(&into).unwrap().map(|entry| entry.unwrap().file_name()).collect();
        assert_eq!(names, [std::ffi::OsString::from("keep.txt")]);
    }
}

/// Custom prompts keep ownership even in a layout pass with neither their text widget nor
/// an egui modal layer. Open real prompts first; exercise the dispatcher with only Files
/// rendered to isolate these existing owner signals rather than relying on generic focus.
fn custom_prompt_blocks_clipboard(owner: &str) {
    use serde_json::json;
    for files_focused in [true, false] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let into = scratch.dir("into");
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.run("solid.box", json!({"body_name":"PromptFixture","length":40,"width":30,"height":20})).unwrap();
        h.app.run("select.set", json!({"items":[{"type":"body","name":"PromptFixture"}]})).unwrap();
        assert!(crate::edit_clipboard::copy(&mut h.app));
        h.call("ui.prefs", json!({"right_rail":true,"right_rail_open":"files"}));
        h.app.file_tree = listing(scratch.path());
        match owner {
            "hole" => {
                h.app.session.selection = vec![solidcraft_engine::Sel::Face {
                    body: "PromptFixture".into(),
                    index: 5,
                    point: solidcore_record::Vec3::new(10.0, 10.0, 20.0),
                }];
                h.app.start("solid.hole");
                h.frames(4);
                h.step(&json!({"click":{"handle":"dialog:section:hole_presets.title"}})).unwrap();
                h.step(&json!({"widget":"Save new"})).unwrap();
                assert!(crate::hole_presets::name_prompt_open(&h.ctx, h.app.dialog.as_ref()));
            }
            "sheet" => {
                h.app.start("sheet.manage_rules");
                h.frames(4);
                h.step(&json!({"click":{"handle":"dialog:section:sheet.personal_rules.title"}})).unwrap();
                h.step(&json!({"widget":"Save new"})).unwrap();
                assert!(crate::personal_sheet_rules::name_prompt_open(&h.ctx, h.app.dialog.as_ref()));
            }
            "slicer" => {
                h.app.services.pick_program = Some(Box::new(|| Box::pin(std::future::pending())));
                h.app.start("file.send_to_slicer");
                h.frames(4);
                h.step(&json!({"click":{"handle":"dialog:Slicer"}})).unwrap();
                assert!(crate::dialogs_save_mesh::popup_open(&h.ctx));
            }
            _ => panic!("unknown custom prompt: {owner}"),
        }
        egui::Popup::close_all(&h.ctx);
        if owner != "slicer" {
            // Let modal-layer history expire without deleting the actual prompt state.
            for _ in 0..2 {
                let _ = h.ctx.run_ui(window_input(&h.ctx, true, Vec::new()), |_| {});
            }
        }
        h.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        h.app.file_tree.select_only(&into);
        h.app.file_tree.focused = files_focused;
        h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: true });
        let before = cad_mark(&h);
        let ctx = h.ctx.clone();
        let mut raw = window_input(&ctx, true, vec![egui::Event::Paste("native prompt paste".into())]);
        h.app.raw_input_hook(&mut raw);
        let _ = ctx.run_ui(raw, |ui| {
            assert!(!ui.ctx().text_edit_focused() && ui.ctx().memory(|m| m.top_modal_layer().is_none()));
            assert!(!egui::Popup::is_any_open(ui.ctx()), "isolate the custom owner, not a generic egui popup");
            let owns = match owner {
                "hole" => crate::hole_presets::name_prompt_open(ui.ctx(), h.app.dialog.as_ref()),
                "sheet" => crate::personal_sheet_rules::name_prompt_open(ui.ctx(), h.app.dialog.as_ref()),
                _ => crate::dialogs_save_mesh::popup_open(ui.ctx()),
            };
            assert!(owns, "real custom prompt owner remains active");
            crate::edit_clipboard::begin_input(&mut h.app, ui.ctx());
            crate::toolbar::shortcuts(&mut h.app, ui.ctx());
            let area = ui.available_rect_before_wrap();
            panel(&mut h.app, ui, area);
            crate::edit_clipboard::finish_input(&mut h.app, ui.ctx());
        });
        assert_eq!(cad_mark(&h), before, "{owner}: no CAD action behind a custom prompt");
        let clip = h.app.file_tree.clip.as_ref().expect("a custom prompt must not paste the file cut clipboard");
        assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&source));
        assert!(clip.cut);
        assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none(), "{owner}: no file job");
        assert!(!into.join("source.txt").exists());
        assert_eq!(fs::read(source).unwrap(), b"source");
    }
}

#[test]
fn custom_hole_name_prompt_blocks_files_and_cad_without_text_focus() {
    custom_prompt_blocks_clipboard("hole");
}

#[test]
fn custom_sheet_name_prompt_blocks_files_and_cad_without_text_focus() {
    custom_prompt_blocks_clipboard("sheet");
}

#[test]
fn custom_slicer_popup_blocks_files_and_cad_without_text_focus() {
    custom_prompt_blocks_clipboard("slicer");
}

#[test]
fn a_file_question_keeps_escape_ahead_of_an_open_parameters_dialog() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    crate::params_dialog::open(&mut h.app);
    h.frames(3);
    h.app.file_tree.select_only(&source);
    h.app.file_tree.request_delete(&[]);
    h.frames(2);
    let before = cad_mark(&h);
    h.frame_with(vec![key_event(egui::Key::Escape, true, Modifiers::NONE), key_event(egui::Key::Escape, false, Modifiers::NONE)]);
    assert!(h.app.file_tree.modal.is_none());
    assert!(h.app.params.is_some(), "the file question, not Parameters underneath it, owns Escape");
    assert_eq!(cad_mark(&h), before);
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[test]
fn file_question_takes_escape_before_custom_file_menu() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    h.app.native_file_menu = false;
    let menu = egui::Id::new("sc_file_menu");
    h.ctx.data_mut(|data| data.insert_temp(menu, true));
    h.app.file_tree.select_only(&source);
    h.app.file_tree.request_delete(&[]);
    assert!(h.app.file_tree.modal.is_some());
    let before = cad_mark(&h);
    h.frame_with(vec![key_event(egui::Key::Escape, true, Modifiers::NONE), key_event(egui::Key::Escape, false, Modifiers::NONE)]);
    assert!(h.app.file_tree.modal.is_none(), "the question receives the first Escape");
    h.frames(3);
    assert_eq!(cad_mark(&h), before, "no CAD action or replay");
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[test]
fn nonmodal_preferences_allows_focused_files_paste() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let into = scratch.dir("into");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
    h.call("ui.prefs", serde_json::json!({"open":"General"}));
    h.frames(3);
    let row = crate::scenario::rect_of("files:row:into").unwrap();
    h.frame_with(pointer_click(row.center(), Modifiers::NONE));
    assert!(h.app.prefs_window.open);
    assert!(h.app.file_tree.focused, "the Files row accepted the click");
    assert!(!h.ctx.text_edit_focused());
    let before = cad_mark(&h);
    h.frame_with(vec![egui::Event::Paste("ordinary".into())]);
    h.frames(30);
    assert_eq!(fs::read(into.join("source.txt")).unwrap(), b"source");
    assert_eq!(fs::read(source).unwrap(), b"source");
    assert_eq!(cad_mark(&h), before, "CAD must not also paste");
    assert!(!into.join("source (copy).txt").exists(), "no replay");
}

#[test]
fn nonmodal_preferences_lets_the_cad_viewport_use_its_clipboard() {
    use serde_json::json;
    for page in ["General", "Keyboard Shortcuts"] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let into = scratch.dir("into");
        let mut h = clipboard_window(scratch.path());
        h.call("ui.prefs", json!({"open": page}));
        h.frames(3);
        assert!(h.app.prefs_window.open, "{page}");
        h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: true });
        h.app.file_tree.select_only(&into);
        h.app.file_tree.focused = true;
        let sketch = h.app.session.active_sketch.expect("sketch");
        let before_curves = h.app.session.doc.sketch(sketch).unwrap().curves.len();
        let before = cad_mark(&h);
        let viewport = h.app.viewport.rect.expect("viewport");
        let at = viewport.left_top() + egui::vec2(24.0, 24.0);
        h.frame_with(pointer_click(at, Modifiers::NONE));
        assert!(!h.ctx.text_edit_focused(), "{page}: the viewport click clears text focus");
        assert!(!h.app.file_tree.focused, "{page}: the viewport click clears Files ownership");
        h.frame_with(vec![egui::Event::Paste("cad clipboard".into())]);
        h.frames(30);
        assert!(h.app.prefs_window.open, "{page}: Preferences stays nonmodal");
        assert_eq!(h.app.session.doc.sketch(sketch).unwrap().curves.len(), before_curves + 4, "{page}: one CAD paste");
        assert_eq!(h.app.session.undo.len(), before.2 + 1, "{page}: one CAD undo step");
        let clip = h.app.file_tree.clip.as_ref().expect("Files clipboard");
        assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&source), "{page}");
        assert!(clip.cut, "{page}: no Files copy or paste");
        assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none(), "{page}: no file job");
        assert!(!into.join("source.txt").exists(), "{page}: no Files paste");
        assert_eq!(fs::read(source).unwrap(), b"source");
    }
}

#[test]
fn nonmodal_shortcuts_page_allows_focused_files_copy_cut_and_paste() {
    use serde_json::json;
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let into = scratch.dir("into");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
    h.call("ui.prefs", json!({"open": "Keyboard Shortcuts"}));
    h.frames(3);
    assert!(h.app.prefs_window.open);
    let before = cad_mark(&h);
    let row = crate::scenario::rect_of("files:row:into").unwrap();
    h.frame_with(pointer_click(row.center(), Modifiers::NONE));
    assert!(h.app.file_tree.focused, "the Files row accepted the click");
    assert!(!h.ctx.text_edit_focused());
    h.frame_with(vec![egui::Event::Paste("ordinary".into())]);
    h.frames(30);
    assert_eq!(fs::read(into.join("source.txt")).unwrap(), b"source");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(cad_mark(&h), before, "CAD must not also paste");
    assert!(!into.join("source (copy).txt").exists(), "no replay");
    let source_row = crate::scenario::rect_of("files:row:source.txt").unwrap();
    h.frame_with(pointer_click(source_row.center(), Modifiers::NONE));
    assert!(h.app.file_tree.focused);
    h.frame_with(vec![egui::Event::Copy]);
    let clip = h.app.file_tree.clip.as_ref().expect("Files copy");
    assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&source));
    assert!(!clip.cut, "Copy is not Cut");
    h.frame_with(vec![egui::Event::Cut]);
    assert!(h.app.file_tree.clip.as_ref().unwrap().cut, "Cut follows the focused row");
    assert!(h.app.prefs_window.open, "Preferences stays nonmodal");
    assert_eq!(cad_mark(&h), before, "Files copy and cut never touch CAD");
}

#[test]
fn theme_paste_on_the_interface_page_precedes_competing_cad_and_files_clipboards() {
    use serde_json::json;
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let into = scratch.dir("into");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: true });
    h.app.file_tree.select_only(&into);
    h.app.file_tree.focused = true;
    h.call("ui.prefs", json!({"open": "Interface"}));
    h.frames(3);
    let before = cad_mark(&h);
    let theme = r##"{"format":"solidcraft-theme/1","name":"Competing","base":"dark","colors":{"panel":"#102030"}}"##;
    h.frame_with(vec![egui::Event::Paste(theme.into())]);
    assert!(
        h.app.preferences.custom_themes.iter().any(|t| t.id == "competing"),
        "{:?}",
        h.app.preferences.custom_themes.iter().map(|t| &t.id).collect::<Vec<_>>()
    );
    assert_eq!(h.app.preferences.theme, "competing");
    assert_eq!(cad_mark(&h), before, "the theme owns the paste, not CAD");
    let clip = h.app.file_tree.clip.as_ref().expect("Files clipboard");
    assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&source));
    assert!(clip.cut);
    assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none());
    assert!(!into.join("source.txt").exists(), "the theme is not a Files paste");
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[test]
fn shortcut_recorder_keeps_escape_while_the_custom_file_menu_is_open() {
    use serde_json::json;
    let scratch = Scratch::new();
    let mut h = clipboard_window(scratch.path());
    h.app.native_file_menu = false;
    h.call("ui.prefs", json!({"open": "Keyboard Shortcuts"}));
    let (handle, _) = crate::scenario::handles("keymap:capture:").into_iter().next().expect("visible recording control");
    h.step(&json!({"click": {"handle": handle}})).unwrap();
    assert!(crate::keymap::capturing(&h.app));
    let menu = egui::Id::new("sc_file_menu");
    h.ctx.data_mut(|data| data.insert_temp(menu, true));
    h.frame_with(vec![key_event(egui::Key::Escape, true, Modifiers::NONE), key_event(egui::Key::Escape, false, Modifiers::NONE)]);
    assert!(!crate::keymap::capturing(&h.app), "the recorder first-refuses Escape, not the File menu");
    assert!(h.ctx.data(|d| d.get_temp::<bool>(menu).unwrap_or(false)), "the File menu did not steal the key");
}

fn window_input(ctx: &egui::Context, focused: bool, events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
        focused,
        events,
        time: Some(ctx.input(|i| i.time) + 0.01),
        ..Default::default()
    }
}

fn window_frame(h: &mut crate::scenario::Harness, focused: bool, events: Vec<egui::Event>, draw: bool) {
    let ctx = h.ctx.clone();
    let mut raw = window_input(&ctx, focused, events);
    h.app.raw_input_hook(&mut raw);
    let _ = ctx.run_ui(raw, |ui| {
        h.app.logic(ui.ctx());
        if draw {
            h.app.ui(ui);
        }
    });
}

#[test]
fn unfocused_window_and_logic_only_input_do_not_replay_files_or_cad_paste() {
    for focused in [false, true] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let into = scratch.dir("into");
        let mut h = clipboard_window(scratch.path());
        let row = crate::scenario::rect_of("files:row:into").unwrap();
        h.frame_with(pointer_click(row.center(), Modifiers::NONE));
        h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
        let before = cad_mark(&h);
        let events = if focused {
            vec![egui::Event::Paste("logic-only".into())]
        } else {
            vec![egui::Event::WindowFocused(false), egui::Event::Paste("unfocused".into())]
        };
        window_frame(&mut h, focused, events, !focused);
        assert!(!h.app.file_tree.busy, "focused={focused}");
        assert!(!into.join("source.txt").exists());
        assert_eq!(cad_mark(&h), before);
        h.frames(3);
        assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none(), "no hidden-batch replay");
        assert!(!into.join("source.txt").exists());
        assert_eq!(cad_mark(&h), before);
        // A later accepted list interaction and native command still work.
        h.frame_with(pointer_click(row.center(), Modifiers::NONE));
        h.frame_with(vec![egui::Event::Paste("later legitimate input".into())]);
        h.frames(30);
        assert_eq!(fs::read(into.join("source.txt")).unwrap(), b"source");
        assert_eq!(cad_mark(&h), before);
    }
}

#[test]
fn popup_chat_keymap_report_and_picker_exclude_files_and_cad_clipboard() {
    use serde_json::json;
    for owner in ["popup", "chat", "keymap", "report", "picker"] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let into = scratch.dir("into");
        let mut h = clipboard_window(scratch.path());
        h.app.file_tree.select_only(&into);
        h.app.file_tree.focused = true;
        h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: true });
        match owner {
            "popup" => {
                h.step(&json!({"click": {"handle": "files:new"}})).unwrap();
                assert!(egui::Popup::is_any_open(&h.ctx), "the actual menu owns input");
            }
            "chat" => {
                h.app.agent_chat.open = true;
                h.app.preferences.ai_features = true;
                h.app.preferences.agent_chat_minimized = false;
                h.frames(4);
                let field = h.ctx.read_response(egui::Id::new("sc_agent_chat_input")).expect("actual Assistant input").rect;
                h.frame_with(pointer_click(field.center(), Modifiers::NONE));
                assert!(h.ctx.text_edit_focused());
            }
            "keymap" => {
                h.call("ui.prefs", json!({"open": "Keyboard Shortcuts"}));
                let (handle, _) = crate::scenario::handles("keymap:capture:").into_iter().next().expect("visible recording control");
                h.step(&json!({"click": {"handle": handle}})).unwrap();
                assert!(crate::keymap::capturing(&h.app));
            }
            "report" => {
                h.app.issue_report.draft = Some(crate::issue_report::draft_for(&h.app, None, None));
                h.frames(4);
                assert!(h.ctx.text_edit_focused());
            }
            "picker" => {
                // Only the OS picker service waits; the application's real picker barrier runs.
                h.app.services.pick_open = Some(Box::new(|_| Box::pin(std::future::pending())));
                h.app.pick_open(|_, _| {});
                h.frames(2);
                assert!(h.app.file_picker.is_some());
            }
            _ => panic!("unknown input owner: {owner}"),
        }
        // Challenge stale Files ownership as well as the already-primed real CAD clipboard.
        h.app.file_tree.focused = true;
        let before = cad_mark(&h);
        for event in [egui::Event::Copy, egui::Event::Cut, egui::Event::Paste("needle".into()), egui::Event::Paste(String::new())] {
            h.frame_with(vec![event]);
            assert_eq!(cad_mark(&h), before, "{owner}: no CAD side effect");
            assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none(), "{owner}: no file operation");
            assert!(h.app.file_tree.clip.as_ref().unwrap().cut, "{owner}: Files must not overwrite its clipboard");
            assert!(!into.join("source.txt").exists(), "{owner}");
        }
        assert_eq!(fs::read(source).unwrap(), b"source");
        if owner == "chat" {
            assert_eq!(h.app.agent_chat.input, "needle", "text owner receives nonempty Paste once");
        }
    }
}

#[test]
fn report_capture_holds_native_paste_and_releases_it_once_without_background_actions() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let into = scratch.dir("into");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&into);
    h.app.file_tree.focused = true;
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
    let before = cad_mark(&h);
    crate::issue_report::begin(&mut h.app, &h.ctx, None, None);
    assert!(crate::issue_report::pending(&h.app));
    h.frame_with(vec![egui::Event::Paste("held needle".into())]);
    assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none());
    assert_eq!(cad_mark(&h), before);
    assert!(!into.join("source.txt").exists());
    let token = h.app.shot_token;
    crate::issue_report::receive(&mut h.app, token, std::sync::Arc::new(egui::ColorImage::new([2, 2], vec![egui::Color32::TRANSPARENT; 4])));
    assert!(!crate::issue_report::pending(&h.app));
    let mut resumed = window_input(&h.ctx, true, Vec::new());
    h.app.raw_input_hook(&mut resumed);
    assert!(
        matches!(resumed.events.as_slice(), [egui::Event::Paste(text)] if text == "held needle"),
        "capture retains the native event, not a reconstructed command"
    );
    let mut next = window_input(&h.ctx, true, Vec::new());
    h.app.raw_input_hook(&mut next);
    assert!(next.events.is_empty(), "held input releases only once");
    assert_eq!(cad_mark(&h), before);
    assert_eq!(fs::read(source).unwrap(), b"source");
}

#[test]
fn a_dirty_open_document_follows_a_case_only_file_tree_rename() {
    use serde_json::json;
    let scratch = Scratch::new();
    let old = scratch.path().join("bracket.sc");
    let new = scratch.path().join("Bracket.sc");
    let mut session = Session::default();
    session.execute("file.save_as", &json!({"path": old.to_string_lossy(), "cache": false})).unwrap();
    session.execute("solid.box", &json!({"length": 3, "width": 3, "height": 3})).unwrap();
    let mut app = crate::SolveApp::new(session, Default::default());
    app.file_tree = listing(scratch.path());
    let undo = app.session.undo.len();
    let features = serde_json::to_value(&app.session.doc.features).unwrap();
    app.file_tree.start_rename(&old);
    app.file_tree.editing.as_mut().unwrap().name = "Bracket.sc".into();
    assert!(app.file_tree.commit_edit().is_none(), "rename must not open another document");
    tick(&mut app, &egui::Context::default(), 1.0);
    assert_eq!(app.session.path.as_deref(), new.to_str());
    assert_eq!(app.session.doc.name, "Bracket");
    assert!(app.session.is_dirty(), "unsaved geometry follows the renamed file");
    assert_eq!(app.session.undo.len(), undo);
    assert_eq!(serde_json::to_value(&app.session.doc.features).unwrap(), features);
    assert_eq!(app.docs.count(), 1);
    let stored: Vec<_> = fs::read_dir(scratch.path()).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert!(stored.contains(&std::ffi::OsString::from("Bracket.sc")));
    assert!(!stored.contains(&std::ffi::OsString::from("bracket.sc")), "check stored spelling, not case-insensitive exists()");
}

#[test]
fn a_clipboard_command_before_a_later_click_is_not_retargeted() {
    let scratch = Scratch::new();
    scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    let row = crate::scenario::rect_of("files:row:source.txt").unwrap();
    let before = cad_mark(&h);
    let mut events = vec![egui::Event::Copy];
    events.extend(pointer_click(row.center(), Modifiers::NONE));
    h.frame_with(events);
    assert!(h.app.file_tree.clip.is_none(), "a later row is not the command's selection");
    assert_eq!(cad_mark(&h), before, "ambiguous application actions are suppressed, not rerouted");
}

/// A native Cut and an Arrow Down in one batch: the tree consumes the Down after the command
/// and moves its row focus, so the Cut must be retired rather than applied to the later row.
/// The arrow still moves the row; only the ambiguous clipboard command is lost.
#[test]
fn same_batch_native_cut_then_down_does_not_cut_the_later_row() {
    let scratch = Scratch::new();
    let first = scratch.file("first.txt", "1");
    let later = scratch.file("later.txt", "2");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&first);
    h.app.file_tree.focused = true;
    let before = cad_mark(&h);
    h.frame_with(vec![
        egui::Event::Cut,
        key_event(egui::Key::ArrowDown, true, Modifiers::NONE),
        key_event(egui::Key::ArrowDown, false, Modifiers::NONE),
    ]);
    assert_eq!(h.app.file_tree.focus, Some(later), "the arrow still moves the row");
    assert!(h.app.file_tree.clip.is_none(), "the cut is retired, not applied to the row the arrow selected");
    assert_eq!(cad_mark(&h), before, "the ambiguous command reaches neither Files nor CAD");
}

/// A native Paste and an Arrow Down in one batch: the Down moves the row focus to a later
/// folder, so the Paste must not be sent to the later folder. It is retired, not guessed to
/// the folder that was selected when the batch began.
#[test]
fn same_batch_native_paste_then_down_does_not_paste_into_the_later_folder() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let first = scratch.dir("first");
    let later = scratch.dir("later");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
    h.app.file_tree.select_only(&first);
    h.app.file_tree.focused = true;
    let before = cad_mark(&h);
    h.frame_with(vec![
        egui::Event::Paste("native".into()),
        key_event(egui::Key::ArrowDown, true, Modifiers::NONE),
        key_event(egui::Key::ArrowDown, false, Modifiers::NONE),
    ]);
    assert_eq!(h.app.file_tree.focus, Some(later.clone()), "the arrow still moves the row");
    assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none(), "the paste is retired, not queued");
    h.frames(30);
    assert!(!later.join("source.txt").exists(), "the later folder is not the paste target");
    assert!(!first.join("source.txt").exists(), "the earlier folder is not guessed either");
    assert!(!h.app.file_tree.busy, "no file job ran");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(cad_mark(&h), before, "the ambiguous command reaches neither Files nor CAD");
    let clip = h.app.file_tree.clip.as_ref().expect("the file clipboard is kept");
    assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&source));
}

/// The entry frame need not have the Files keys yet: an accepted row click in this batch gives
/// them to the tree, and the later Down moves the row. A native Cut earlier in the batch must be
/// retired, not applied to the row the click and arrow ended on.
#[test]
fn same_batch_row_click_then_native_cut_then_down_does_not_cut_the_later_row() {
    let scratch = Scratch::new();
    let first = scratch.file("first.txt", "1");
    let later = scratch.file("later.txt", "2");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&first);
    assert!(!h.app.file_tree.focused, "the batch starts without the Files keys");
    let row = crate::scenario::rect_of("files:row:first.txt").unwrap();
    let before = cad_mark(&h);
    let mut events = pointer_click(row.center(), Modifiers::NONE);
    events.push(egui::Event::Cut);
    events.push(key_event(egui::Key::ArrowDown, true, Modifiers::NONE));
    events.push(key_event(egui::Key::ArrowDown, false, Modifiers::NONE));
    h.frame_with(events);
    assert!(h.app.file_tree.focused, "the accepted row click gives the tree the keys");
    assert_eq!(h.app.file_tree.focus, Some(later), "the Down moves the row");
    assert!(h.app.file_tree.clip.is_none(), "the Cut is retired, not applied to the row the click and Down ended on");
    assert_eq!(cad_mark(&h), before, "the ambiguous command reaches neither Files nor CAD");
}

/// The same transition for Paste: an accepted row click starts the Files keys, then the native
/// Paste, then Down moves to a later folder. The Paste is retired rather than sent to the later
/// folder or guessed back to the earlier one.
#[test]
fn same_batch_row_click_then_native_paste_then_down_does_not_paste_into_the_later_folder() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let first = scratch.dir("first");
    let later = scratch.dir("later");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
    assert!(!h.app.file_tree.focused, "the batch starts without the Files keys");
    let row = crate::scenario::rect_of("files:row:first").unwrap();
    let before = cad_mark(&h);
    let mut events = pointer_click(row.center(), Modifiers::NONE);
    events.push(egui::Event::Paste("native".into()));
    events.push(key_event(egui::Key::ArrowDown, true, Modifiers::NONE));
    events.push(key_event(egui::Key::ArrowDown, false, Modifiers::NONE));
    h.frame_with(events);
    assert!(h.app.file_tree.focused, "the accepted row click gives the tree the keys");
    assert_eq!(h.app.file_tree.focus, Some(later.clone()), "the Down moves to the later folder");
    assert!(!h.app.file_tree.busy && h.app.file_tree.modal.is_none(), "the paste is retired, not queued");
    h.frames(30);
    assert!(!later.join("source.txt").exists(), "the later folder is not the paste target");
    assert!(!first.join("source.txt").exists(), "the earlier folder is not guessed either");
    assert!(!h.app.file_tree.busy, "no file job ran");
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(cad_mark(&h), before, "the ambiguous command reaches neither Files nor CAD");
    let clip = h.app.file_tree.clip.as_ref().expect("the file clipboard is kept");
    assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&source));
}

/// A later tree key that cannot move the row is not a retarget: at the last row a Down is a
/// no-op, so the native Cut must still act on the row settled when it was pressed, not be
/// retired by mere key presence.
#[test]
fn same_batch_native_cut_then_noop_down_at_the_last_row_cuts_the_settled_row() {
    let scratch = Scratch::new();
    scratch.file("first.txt", "1");
    let later = scratch.file("later.txt", "2");
    let mut h = clipboard_window(scratch.path());
    focus_file_row(&mut h, "later.txt");
    let before = cad_mark(&h);
    h.frame_with(vec![
        egui::Event::Cut,
        key_event(egui::Key::ArrowDown, true, Modifiers::NONE),
        key_event(egui::Key::ArrowDown, false, Modifiers::NONE),
    ]);
    assert_eq!(h.app.file_tree.focus, Some(later.clone()), "the boundary Down leaves the row");
    let clip = h.app.file_tree.clip.as_ref().expect("the Cut applies to the row settled when pressed");
    assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&later));
    assert!(clip.cut);
    assert_eq!(cad_mark(&h), before, "the command reaches Files, not CAD");
}

/// A move before the command settles the target; a later no-op Down must not make that move
/// count as a post-command retarget. The command acts on the row it had when pressed.
#[test]
fn same_batch_early_down_then_native_cut_then_later_noop_down_cuts_the_settled_row() {
    let scratch = Scratch::new();
    scratch.file("first.txt", "1");
    let later = scratch.file("later.txt", "2");
    let mut h = clipboard_window(scratch.path());
    focus_file_row(&mut h, "first.txt");
    let before = cad_mark(&h);
    h.frame_with(vec![
        key_event(egui::Key::ArrowDown, true, Modifiers::NONE),
        key_event(egui::Key::ArrowDown, false, Modifiers::NONE),
        egui::Event::Cut,
        key_event(egui::Key::ArrowDown, true, Modifiers::NONE),
        key_event(egui::Key::ArrowDown, false, Modifiers::NONE),
    ]);
    assert_eq!(h.app.file_tree.focus, Some(later.clone()), "the first Down moved to the settled row");
    let clip = h.app.file_tree.clip.as_ref().expect("the Cut applies to the settled row");
    assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&later));
    assert!(clip.cut);
    assert_eq!(cad_mark(&h), before, "the command reaches Files, not CAD");
}

/// egui's logical key matching ignores the extra Alt, so the tree consumes Alt+Down and moves
/// its row. A Cut pressed before it must be retired, not applied to the row the accepted
/// navigation reached.
#[test]
fn same_batch_native_cut_then_alt_down_retires_the_command() {
    let scratch = Scratch::new();
    scratch.file("first.txt", "1");
    let later = scratch.file("later.txt", "2");
    let mut h = clipboard_window(scratch.path());
    focus_file_row(&mut h, "first.txt");
    let before = cad_mark(&h);
    let alt = Modifiers { alt: true, ..Default::default() };
    h.frame_with(vec![egui::Event::Cut, key_event(egui::Key::ArrowDown, true, alt), key_event(egui::Key::ArrowDown, false, alt)]);
    assert_eq!(h.app.file_tree.focus, Some(later), "the tree accepts Alt+Down as navigation");
    assert!(h.app.file_tree.clip.is_none(), "the Cut is retired, not applied to the row Alt+Down reached");
    assert_eq!(cad_mark(&h), before, "the ambiguous command reaches neither Files nor CAD");
}

/// egui's logical matching also ignores the extra Shift, so Cmd/Ctrl+Shift+A selects all rows.
/// A Cut pressed before it must be retired rather than applied to the later selection.
#[test]
fn same_batch_native_cut_then_command_shift_a_retires_the_command() {
    let scratch = Scratch::new();
    let first = scratch.file("first.txt", "1");
    let later = scratch.file("later.txt", "2");
    let mut h = clipboard_window(scratch.path());
    // The assistant's default Ctrl+Shift+A toggle consumes this chord before any panel. The
    // panel's own egui logical matching is what this test exercises.
    crate::keymap::bind(&mut h.app, "ui.assistant", "", false).unwrap();
    focus_file_row(&mut h, "first.txt");
    let before = cad_mark(&h);
    let chord = Modifiers::COMMAND | Modifiers::SHIFT;
    h.frame_with(vec![egui::Event::Cut, key_event(egui::Key::A, true, chord), key_event(egui::Key::A, false, chord)]);
    assert!(
        h.app.file_tree.selection.contains(&first) && h.app.file_tree.selection.contains(&later),
        "the tree accepts Cmd/Ctrl+Shift+A: {:?}",
        h.app.file_tree.selection
    );
    assert!(h.app.file_tree.clip.is_none(), "the Cut is retired, not applied to the later selection");
    assert_eq!(cad_mark(&h), before, "the ambiguous command reaches neither Files nor CAD");
}

/// The converse transition: Files has the keys when the batch starts, but a click in the
/// retained-open viewport ends that before the native CAD command. The tree never moves its
/// row in this batch, so the CAD Cut or Paste must run rather than be suppressed by the entry
/// frame's Files focus.
#[test]
fn same_batch_click_away_then_native_cad_clipboard_then_down_keeps_the_cad_command() {
    for cut in [true, false] {
        let scratch = Scratch::new();
        scratch.file("notes.txt", "n");
        let (mut h, body) = cad_body_and_files(&scratch);
        if !cut {
            assert!(crate::edit_clipboard::copy(&mut h.app));
        }
        focus_file_row(&mut h, "notes.txt");
        let body_count = h.app.session.model.state().bodies.len();
        let viewport = h.app.viewport.rect.expect("viewport");
        let at = viewport.left_top() + egui::vec2(24.0, 24.0);
        let mut events = pointer_click(at, Modifiers::NONE);
        events.push(if cut { egui::Event::Cut } else { egui::Event::Paste("native".into()) });
        events.push(key_event(egui::Key::ArrowDown, true, Modifiers::NONE));
        events.push(key_event(egui::Key::ArrowDown, false, Modifiers::NONE));
        h.frame_with(events);
        h.frames(2);
        assert!(crate::right_rail::shown(&h.app), "cut={cut}: the viewport click keeps the rail shown");
        assert_eq!(crate::right_rail::open(&h.app), Some(crate::right_rail::Item::Files), "cut={cut}: the Files panel stays open");
        assert!(h.app.shell.right_rail_panel.is_some(), "cut={cut}: the retained-open panel is drawn");
        assert!(!h.app.file_tree.focused, "cut={cut}: the viewport click takes the Files keys");
        assert!(h.app.file_tree.clip.is_none(), "cut={cut}: Files did not act on its old row");
        if cut {
            assert_eq!(h.app.cut.bodies.as_slice(), std::slice::from_ref(&body), "the CAD Cut still marks the body");
        } else {
            assert_eq!(h.app.session.model.state().bodies.len(), body_count + 1, "the CAD Paste still copies the body");
        }
    }
}

/// The reverse order is unaffected: the arrow settles the row before the native Cut, so the
/// command acts on the row the user can see when it is applied.
#[test]
fn same_batch_down_then_native_cut_cuts_the_row_it_moved_to() {
    let scratch = Scratch::new();
    let first = scratch.file("first.txt", "1");
    let later = scratch.file("later.txt", "2");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&first);
    h.app.file_tree.focused = true;
    h.frame_with(vec![
        key_event(egui::Key::ArrowDown, true, Modifiers::NONE),
        key_event(egui::Key::ArrowDown, false, Modifiers::NONE),
        egui::Event::Cut,
    ]);
    assert_eq!(h.app.file_tree.focus, Some(later.clone()), "the arrow moves the row");
    let clip = h.app.file_tree.clip.as_ref().expect("the settled row is cut");
    assert_eq!(clip.paths.as_slice(), std::slice::from_ref(&later));
    assert!(clip.cut);
}

/// The reverse order for Paste: the arrow settles the destination folder before the native
/// Paste, so the paste goes to the row the user can see when it is applied.
#[test]
fn same_batch_down_then_native_paste_pastes_into_the_folder_it_moved_to() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let first = scratch.dir("first");
    let later = scratch.dir("later");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
    h.app.file_tree.select_only(&first);
    h.app.file_tree.focused = true;
    h.frame_with(vec![
        key_event(egui::Key::ArrowDown, true, Modifiers::NONE),
        key_event(egui::Key::ArrowDown, false, Modifiers::NONE),
        egui::Event::Paste("native".into()),
    ]);
    assert_eq!(h.app.file_tree.focus, Some(later.clone()), "the arrow settles the destination");
    h.frames(30);
    assert_eq!(fs::read(later.join("source.txt")).unwrap(), b"source", "the settled folder is the paste target");
    assert!(!first.join("source.txt").exists(), "the folder before the arrow is not used");
    assert_eq!(fs::read(&source).unwrap(), b"source");
}

#[test]
fn native_paste_is_retired_across_layout_repasses_but_not_later_invocations() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let into = scratch.dir("into");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.clip = Some(Clip { paths: vec![source], cut: false });
    h.app.file_tree.select_only(&into);
    h.app.file_tree.focused = true;
    let before = cad_mark(&h);
    let ctx = h.ctx.clone();
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))),
        focused: true,
        events: vec![egui::Event::Paste("native".into())],
        time: Some(ctx.input(|i| i.time) + 0.01),
        ..Default::default()
    };
    h.app.raw_input_hook(&mut raw);
    let _ = ctx.run_ui(raw, |ui| {
        h.app.logic(ui.ctx());
        h.app.ui(ui);
        if ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("test clipboard once-only delivery");
        }
    });
    h.frames(30);
    assert_eq!(fs::read(into.join("source.txt")).unwrap(), b"source");
    assert!(!into.join("source (copy).txt").exists());
    assert!(h.app.file_tree.modal.is_none(), "replayed Paste must not ask a second question");
    assert_eq!(cad_mark(&h), before);
    h.frame_with(vec![egui::Event::Paste("later native invocation".into())]);
    assert!(matches!(h.app.file_tree.modal, Some(Modal::Conflict { .. })), "a later legitimate invocation is not lost");
}

#[test]
fn empty_paste_has_no_owner_in_home_hidden_collapsed_unfocused_or_unrendered_files() {
    use serde_json::json;
    for state in ["home", "hidden", "collapsed", "other_panel", "elsewhere", "too_narrow"] {
        let scratch = Scratch::new();
        let source = scratch.file("source.txt", "source");
        let mut h = clipboard_window(scratch.path());
        h.app.file_tree.select_only(&source);
        h.app.file_tree.focused = true;
        h.app.file_tree.clip = Some(Clip { paths: vec![source], cut: false });
        match state {
            "home" => h.app.home.open = true,
            "hidden" => h.app.preferences.right_rail = false,
            "collapsed" => h.app.preferences.right_rail_open.clear(),
            "other_panel" => h.app.preferences.right_rail_open = "review".into(),
            "elsewhere" => h.app.file_tree.focused = false,
            "too_narrow" => {
                h.call("ui.resize", json!({"width": 400, "height": 480}));
            }
            _ => panic!("unknown visibility case: {state}"),
        }
        let before = cad_mark(&h);
        h.frame_with(vec![egui::Event::Paste(String::new())]);
        assert_eq!(cad_mark(&h), before, "{state}: empty fallback is not CAD Paste");
        assert!(h.app.file_tree.modal.is_none(), "{state}: hidden Files must not paste into itself");
        assert!(!h.app.file_tree.busy, "{state}");
    }
}

#[test]
fn quick_synthetic_clipboard_chords_use_event_modifiers_after_command_release() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&source);
    h.app.file_tree.focused = true;
    let before = cad_mark(&h);
    h.frame_with(vec![
        egui::Event::ModifiersChanged(Modifiers::COMMAND | Modifiers::CTRL),
        key_event(egui::Key::C, true, Modifiers::COMMAND | Modifiers::CTRL),
        key_event(egui::Key::C, false, Modifiers::NONE),
        egui::Event::ModifiersChanged(Modifiers::NONE),
    ]);
    assert_eq!(h.app.file_tree.clip.as_ref().unwrap().paths, [source]);
    assert_eq!(cad_mark(&h), before);
}

#[test]
fn closing_an_inline_editor_does_not_expose_its_clipboard_event_to_cad_or_files() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&source);
    h.app.file_tree.focused = true;
    h.app.file_tree.start_rename(&source);
    h.frames(3);
    assert!(h.ctx.text_edit_focused());
    let before = cad_mark(&h);
    h.frame_with(vec![key_event(egui::Key::Escape, true, Modifiers::NONE), egui::Event::Copy]);
    assert!(h.app.file_tree.editing.is_none());
    assert!(h.app.file_tree.clip.is_none(), "a closing text editor retains first refusal");
    assert_eq!(cad_mark(&h), before);
}

#[test]
fn an_empty_paste_does_not_delete_selected_rename_text_or_act_on_files() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&source);
    h.app.file_tree.focused = true;
    h.app.file_tree.clip = Some(Clip { paths: vec![source.clone()], cut: false });
    h.app.file_tree.start_rename(&source);
    h.frames(3);
    h.frame_with(vec![key_event(egui::Key::A, true, Modifiers::COMMAND)]);
    let before = cad_mark(&h);
    h.frame_with(vec![egui::Event::Paste(String::new())]);
    assert_eq!(h.app.file_tree.editing.as_ref().unwrap().name, "source.txt");
    assert!(h.app.file_tree.modal.is_none());
    assert!(!h.app.file_tree.busy);
    assert_eq!(cad_mark(&h), before);
}

#[test]
fn changing_root_invalidates_the_old_lists_keyboard_ownership() {
    let first = Scratch::new();
    let second = Scratch::new();
    let source = first.file("source.txt", "source");
    let mut tree = listing(first.path());
    tree.select_only(&source);
    tree.focused = true;
    tree.set_root(Some(second.path().to_path_buf()));
    assert!(!tree.focused, "a different root has not accepted a list interaction");
}

#[test]
fn mac_control_primary_is_a_context_click_and_cmd_primary_only_toggles() {
    let scratch = Scratch::new();
    let a = scratch.file("a.txt", "a");
    let b = scratch.file("b.txt", "b");
    let mut h = clipboard_window(scratch.path());
    h.ctx.set_os(egui::os::OperatingSystem::Mac);
    h.app.file_tree.select_only(&a);
    let row = crate::scenario::rect_of("files:row:b.txt").unwrap();
    let cmd = Modifiers::MAC_CMD | Modifiers::COMMAND;
    h.frame_with(pointer_click(row.center(), cmd));
    assert!(h.app.file_tree.selection.contains(&a) && h.app.file_tree.selection.contains(&b));
    assert!(h.app.file_tree.context.is_none(), "Cmd click is selection, never a menu");
    // Separate taps, rather than a double-click which activates a file.
    h.frames(40);
    h.frame_with(pointer_click(row.center(), cmd));
    assert!(!h.app.file_tree.selection.contains(&b), "Cmd click toggles the selected row off");
    assert!(h.app.file_tree.selection.contains(&a) && h.app.file_tree.context.is_none());
    h.frames(40);
    h.frame_with(pointer_click(row.center(), cmd));
    h.frames(40);
    h.frame_with(pointer_click(row.center(), Modifiers::CTRL));
    assert!(h.app.file_tree.context.is_some(), "Control-primary opens the context menu on macOS");
    assert_eq!(h.app.file_tree.selection.len(), 2, "context-click preserves a selected multi-selection");
}

#[test]
fn secondary_click_preserves_multi_selection_and_non_mac_control_click_toggles() {
    for os in [egui::os::OperatingSystem::Mac, egui::os::OperatingSystem::Windows] {
        let scratch = Scratch::new();
        let a = scratch.file("a.txt", "a");
        let b = scratch.file("b.txt", "b");
        let mut h = clipboard_window(scratch.path());
        h.ctx.set_os(os);
        h.app.file_tree.select_only(&a);
        h.app.file_tree.selection.insert(b.clone());
        let row = crate::scenario::rect_of("files:row:b.txt").unwrap();
        let before = cad_mark(&h);
        h.frame_with(vec![
            egui::Event::PointerMoved(row.center()),
            egui::Event::PointerButton { pos: row.center(), button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE },
            egui::Event::PointerButton { pos: row.center(), button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE },
        ]);
        assert_eq!(h.app.file_tree.selection.len(), 2);
        assert_eq!(h.app.file_tree.context.as_ref().unwrap().path, b);
        assert_eq!(cad_mark(&h), before);
    }
    let scratch = Scratch::new();
    let a = scratch.file("a.txt", "a");
    let b = scratch.file("b.txt", "b");
    let mut h = clipboard_window(scratch.path());
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    h.app.file_tree.select_only(&a);
    let row = crate::scenario::rect_of("files:row:b.txt").unwrap();
    h.frame_with(pointer_click(row.center(), Modifiers::CTRL | Modifiers::COMMAND));
    assert!(h.app.file_tree.selection.contains(&a) && h.app.file_tree.selection.contains(&b));
    assert!(h.app.file_tree.context.is_none(), "other platforms retain Ctrl selection");
}

fn key_event(key: egui::Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    egui::Event::Key { key, pressed, modifiers, physical_key: None, repeat: false }
}

#[test]
fn one_escape_closes_delete_opened_by_keyboard_or_menu_without_mutation() {
    use serde_json::json;
    for menu in [false, true] {
        let scratch = Scratch::new();
        let path = scratch.file("notes.txt", "kept");
        let mut h = clipboard_window(scratch.path());
        h.app.file_tree.select_only(&path);
        h.app.file_tree.focused = true;
        let before = cad_mark(&h);
        if menu {
            let row = crate::scenario::rect_of("files:row:notes.txt").unwrap();
            h.call("ui.click", json!({"x": row.center().x, "y": row.center().y, "button": "right"}));
            h.frames(2);
            h.step(&json!({"click": {"handle": "menu:files_delete"}})).unwrap();
        } else {
            h.frame_with(vec![key_event(egui::Key::Delete, true, Modifiers::NONE), key_event(egui::Key::Delete, false, Modifiers::NONE)]);
        }
        h.frames(2);
        assert!(matches!(h.app.file_tree.modal, Some(Modal::Delete { .. })));
        assert_eq!(cad_mark(&h), before, "opening Delete must not mutate the model");
        h.frame_with(vec![key_event(egui::Key::Escape, true, Modifiers::NONE), key_event(egui::Key::Escape, false, Modifiers::NONE)]);
        assert!(h.app.file_tree.modal.is_none(), "one Escape closes Delete (menu={menu})");
        assert_eq!(fs::read(&path).unwrap(), b"kept");
        assert_eq!(cad_mark(&h), before);
        assert!(!h.app.file_tree.busy);
    }
}

#[test]
fn every_file_modal_has_first_refusal_of_escape_and_clipboard() {
    for variant in ["permanent", "conflict", "replace_open", "reload"] {
        let scratch = Scratch::new();
        let path = scratch.file("source.txt", "kept");
        let mut h = clipboard_window(scratch.path());
        h.app.file_tree.select_only(&path);
        h.app.file_tree.focused = true;
        h.app.file_tree.modal = Some(match variant {
            "permanent" => Modal::Permanent { paths: vec![path.clone()], reason: "no trash here".into() },
            "conflict" => Modal::Conflict {
                sources: vec![path.clone()],
                destination: scratch.path().to_path_buf(),
                cut: false,
                names: vec!["source.txt".into()],
                files_only: true,
                replaceable: Vec::new(),
            },
            "replace_open" => Modal::ReplaceOpen {
                sources: vec![path.clone()],
                destination: scratch.path().to_path_buf(),
                cut: false,
                dirty: vec![path.to_string_lossy().into_owned()],
                replaceable: Vec::new(),
            },
            "reload" => Modal::DiscardReload { path: path.to_string_lossy().into_owned() },
            _ => panic!("unknown modal case: {variant}"),
        });
        h.frames(2);
        let before = cad_mark(&h);
        h.frame_with(vec![egui::Event::Copy]);
        assert!(h.app.file_tree.clip.is_none(), "{variant}: the tree must not copy behind a modal");
        assert_eq!(cad_mark(&h), before);
        h.frame_with(vec![key_event(egui::Key::Escape, true, Modifiers::NONE), key_event(egui::Key::Escape, false, Modifiers::NONE)]);
        assert!(h.app.file_tree.modal.is_none(), "{variant}: a single Escape cancels");
        assert_eq!(cad_mark(&h), before);
        assert_eq!(fs::read(path).unwrap(), b"kept");
    }
}

#[test]
fn macos_chinese_delete_uses_the_platforms_trash_name() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut h = clipboard_window(scratch.path());
    h.ctx.set_os(egui::os::OperatingSystem::Mac);
    h.app.preferences.language = "zh".into();
    h.app.file_tree.select_only(&source);
    h.app.file_tree.request_delete(&[]);
    h.frames(3);
    assert!(crate::scenario::handle_at("dialog:移到废纸篓").is_some(), "macOS uses 废纸篓, not 回收站");
}

/// A folder of this test's own, removed when it drops.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("intcad-file-tree-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn file(&self, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        fs::write(&path, body).unwrap();
        path
    }
    fn dir(&self, name: &str) -> std::path::PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A tree with its root already listing `entries`, without waiting for the scan worker.
fn listing(root: &std::path::Path) -> FileTree {
    let mut tree = FileTree::new();
    tree.set_root(Some(root.to_path_buf()));
    if let Ok(entries) = files::read_dir(root) {
        tree.dirs.insert(root.to_path_buf(), Dir { entries, state: Listing::Ready });
    }
    tree
}

#[test]
fn the_tree_shows_folders_and_files_and_opens_a_folder_on_request() {
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "");
    scratch.file(".hidden.sc", "");
    let folder = scratch.dir("parts");
    let mut tree = listing(scratch.path());
    let rows = tree.rows();
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["parts", "bracket.sc"], "folders first, hidden entries out: {names:?}");
    tree.toggle_dir(&folder);
    tree.dirs.insert(
        folder.clone(),
        Dir {
            entries: vec![files::Entry { name: "pin.sc".into(), path: folder.join("pin.sc"), kind: files::Kind::File, size: 0, modified: None }],
            state: Listing::Ready,
        },
    );
    let rows = tree.rows();
    assert!(rows.iter().any(|r| r.name == "pin.sc" && r.depth == 1), "an open folder shows its children: {rows:?}");
    tree.show_hidden = true;
    assert!(tree.rows().iter().any(|r| r.name == ".hidden.sc"));
    tree.toggle_dir(&folder);
    assert_eq!(tree.rows().len(), 3, "a closed folder shows only its own row");
}

#[test]
fn clicking_selects_one_entry_and_the_modifiers_extend_or_toggle() {
    let scratch = Scratch::new();
    let a = scratch.file("a.sc", "");
    let b = scratch.file("b.sc", "");
    let c = scratch.file("c.sc", "");
    let mut tree = listing(scratch.path());
    let visible: Vec<PathBuf> = tree.rows().iter().map(|r| r.path.clone()).collect();
    tree.click(&a, Modifiers::NONE, &visible);
    assert_eq!(tree.selection.iter().collect::<Vec<_>>(), vec![&a]);
    tree.click(&c, Modifiers { shift: true, ..Default::default() }, &visible);
    assert_eq!(tree.selection.len(), 3, "Shift selects the run from the anchor: {:?}", tree.selection);
    assert!(tree.selection.contains(&b));
    tree.click(&b, Modifiers { command: true, ..Default::default() }, &visible);
    assert_eq!(tree.selection.len(), 2, "Ctrl/Cmd toggles one row");
    assert!(!tree.selection.contains(&b));
}

#[test]
fn a_folder_and_a_child_in_the_selection_are_deleted_once() {
    let scratch = Scratch::new();
    let folder = scratch.dir("parts");
    let child = folder.join("pin.sc");
    fs::write(&child, "").unwrap();
    let other = scratch.file("bracket.sc", "");
    let mut tree = listing(scratch.path());
    tree.selection.insert(folder.clone());
    tree.selection.insert(child);
    tree.selection.insert(other.clone());
    let picked = tree.selected(None);
    assert_eq!(picked, vec![other, folder], "the child of a selected folder is not a second item");
}

#[test]
fn a_name_already_taken_asks_before_it_is_replaced() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    tree.place(vec![source], &into, false, None);
    match tree.modal.as_ref() {
        Some(Modal::Conflict { names, files_only, .. }) => {
            assert_eq!(names, &["bracket.sc".to_string()]);
            assert!(files_only);
        }
        _ => panic!("a conflict asks first: {:?}", tree.modal.is_some()),
    }
    assert!(tree.take_answers().is_empty(), "nothing runs before the answer");
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"old");
}

#[test]
fn an_answer_of_keep_both_leaves_both_files() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    tree.resolve_conflict(vec![source], into.clone(), false, Conflict::KeepBoth, &[]);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 1);
    assert!(answer.results[0].succeeded(), "{:?}", answer.results[0]);
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"old", "the existing file is kept");
    assert_eq!(fs::read(into.join("bracket (copy).sc")).unwrap(), b"new");
}

#[test]
fn a_confirmed_single_replace_still_replaces_the_old_file() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    tree.place(vec![source.clone()], &into, false, None);
    assert!(matches!(tree.modal, Some(Modal::Conflict { .. })), "the conflict is asked first");
    let replaceable = asked_replacements(&tree);
    tree.resolve_conflict(vec![source.clone()], into.clone(), false, Conflict::Replace, &replaceable);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 1);
    assert_eq!(answer.results[0].source, source);
    assert_eq!(answer.results[0].target, into.join("bracket.sc"));
    assert!(answer.results[0].succeeded(), "{:?}", answer.results[0]);
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"new", "the confirmed conflict is replaced");
}

#[test]
fn a_batch_replace_keeps_the_prior_items_output_and_reports_each_target() {
    let scratch = Scratch::new();
    let one = scratch.dir("one");
    let first = one.join("part.sc");
    fs::write(&first, "first").unwrap();
    let two = scratch.dir("two");
    let second = two.join("part.sc");
    fs::write(&second, "second").unwrap();
    let into = scratch.dir("into");
    fs::write(into.join("part.sc"), "old").unwrap();
    let loose = scratch.file("loose.txt", "loose");
    let mut tree = listing(scratch.path());
    let sources = vec![first.clone(), second.clone(), loose.clone()];
    tree.place(sources.clone(), &into, false, None);
    assert!(matches!(tree.modal, Some(Modal::Conflict { .. })), "the same-basename entries ask first");
    let replaceable = asked_replacements(&tree);
    tree.resolve_conflict(sources, into.clone(), false, Conflict::Replace, &replaceable);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 3);
    let result = |source: &std::path::Path| {
        answer.results.iter().find(|result| result.source == source).unwrap_or_else(|| panic!("no result for {}", source.display()))
    };
    let first_result = result(&first);
    let second_result = result(&second);
    assert_eq!(first_result.target, into.join("part.sc"), "the first item takes the confirmed destination");
    assert_eq!(fs::read(&first_result.target).unwrap(), b"first", "a later item must not overwrite the prior result");
    assert!(second_result.succeeded(), "{:?}", second_result);
    assert_ne!(second_result.target, into.join("part.sc"), "the later source keeps its own output");
    assert_eq!(fs::read(&second_result.target).unwrap(), b"second", "the later source is preserved at the reported target");
}

#[test]
fn a_batch_replace_does_not_consent_for_a_destination_that_appears_later() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let loose = scratch.file("notes.txt", "loose");
    let mut tree = listing(scratch.path());
    let sources = vec![source.clone(), loose.clone()];
    tree.place(sources.clone(), &into, false, None);
    assert!(matches!(tree.modal, Some(Modal::Conflict { .. })), "the existing destination asks first");
    // The list is frozen in the question: this destination was not part of it.
    fs::write(into.join("notes.txt"), "appeared after the answer").unwrap();
    let replaceable = asked_replacements(&tree);
    tree.resolve_conflict(sources, into.clone(), false, Conflict::Replace, &replaceable);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 2);
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"new", "the confirmed conflict is replaced");
    assert_eq!(fs::read(into.join("notes.txt")).unwrap(), b"appeared after the answer", "an unasked destination is never replaced");
    let loose_result = answer.results.iter().find(|result| result.source == loose).expect("the non-conflicting item's result");
    assert!(loose_result.succeeded(), "{:?}", loose_result);
    assert_ne!(loose_result.target, into.join("notes.txt"), "the item is placed under a free name");
    assert_eq!(fs::read(&loose_result.target).unwrap(), b"loose", "its bytes are preserved at the reported target");
}

#[test]
fn unknown_entry_identity_is_never_same_or_distinct_evidence() {
    assert_eq!(
        super::known_relation(&super::EntryIdentity::Unknown, &super::EntryIdentity::Unknown),
        None,
        "equal unknown values, including equal metadata on different entries, are not proof"
    );
}

#[test]
fn an_unknown_entry_identity_keeps_both_entries() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    let consent = super::ReplaceConsent { source: source.clone(), identity: super::EntryIdentity::Unknown };
    tree.resolve_conflict(vec![source.clone()], into.clone(), false, Conflict::Replace, &[consent]);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 1);
    let result = &answer.results[0];
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"old", "an unknown identity never replaces the entry");
    assert!(result.succeeded(), "{result:?}");
    assert_ne!(result.target, into.join("bracket.sc"), "the source is placed under a free name");
    assert_eq!(fs::read(&result.target).unwrap(), b"new");
}

#[test]
fn an_unresolved_case_only_spelling_is_an_uncertain_destination_alias() {
    let scratch = Scratch::new();
    let existing = scratch.file("part.sc", "existing");
    let other = scratch.path().join("Part.sc");
    assert!(same_destination(&existing, &other), "an unresolved case-only spelling is kept as an uncertain alias");
}

#[cfg(unix)]
#[test]
fn a_case_alias_of_one_entry_is_one_destination_without_realpath_normalization() {
    let scratch = Scratch::new();
    // Neutral spellings keep this fixture usable on a case-insensitive volume, where a hard link
    // to a case-only name could be refused as EEXIST.
    let first = scratch.file("entry-one", "same entry");
    let second = scratch.path().join("entry-two");
    fs::hard_link(&first, &second).expect("a same-entry fixture with distinct neutral names");
    assert_ne!(first, second, "the two paths have distinct neutral spellings");
    assert!(same_destination(&first, &second), "one entry reached by two spellings is one destination");
    // On the default case-insensitive APFS volume the case-only lookup names this same entry.
    let lower = scratch.file("part.sc", "case alias");
    let upper = scratch.path().join("Part.sc");
    if fs::metadata(&upper).is_ok() {
        assert!(same_destination(&lower, &upper), "a case-only lookup of an existing entry is one destination");
    }
}

#[test]
fn a_batch_replace_does_not_replace_a_destination_that_changed_after_the_question() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "new");
    let into = scratch.dir("parts");
    fs::write(into.join("bracket.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    tree.place(vec![source.clone()], &into, false, None);
    let replaceable = asked_replacements(&tree);
    // The destination the question named is replaced by another entry while the answer is open.
    let newcomer = scratch.file("newcomer.sc", "changed after the question");
    fs::remove_file(into.join("bracket.sc")).unwrap();
    fs::rename(&newcomer, into.join("bracket.sc")).unwrap();
    tree.resolve_conflict(vec![source.clone()], into.clone(), false, Conflict::Replace, &replaceable);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 1);
    let result = &answer.results[0];
    assert_eq!(fs::read(into.join("bracket.sc")).unwrap(), b"changed after the question", "the newcomer is never silently replaced");
    assert!(result.succeeded(), "{result:?}");
    assert_ne!(result.target, into.join("bracket.sc"), "the source is placed under a free name");
    assert_eq!(fs::read(&result.target).unwrap(), b"new", "the source bytes are preserved at the reported target");
}

#[test]
fn a_batch_move_replace_keeps_each_sources_distinct_bytes() {
    let scratch = Scratch::new();
    let one = scratch.dir("one");
    let first = one.join("part.sc");
    fs::write(&first, "first").unwrap();
    let two = scratch.dir("two");
    let second = two.join("part.sc");
    fs::write(&second, "second").unwrap();
    let into = scratch.dir("into");
    fs::write(into.join("part.sc"), "old").unwrap();
    let mut tree = listing(scratch.path());
    let sources = vec![first.clone(), second.clone()];
    tree.place(sources.clone(), &into, true, None);
    let replaceable = asked_replacements(&tree);
    tree.resolve_conflict(sources, into.clone(), true, Conflict::Replace, &replaceable);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 2);
    let result = |source: &std::path::Path| {
        answer.results.iter().find(|result| result.source == source).unwrap_or_else(|| panic!("no result for {}", source.display()))
    };
    let first_result = result(&first);
    let second_result = result(&second);
    assert_eq!(first_result.target, into.join("part.sc"));
    assert_eq!(fs::read(&first_result.target).unwrap(), b"first", "the first move owns the confirmed destination");
    assert!(second_result.succeeded(), "{second_result:?}");
    assert_ne!(second_result.target, into.join("part.sc"));
    assert_eq!(fs::read(&second_result.target).unwrap(), b"second", "the later move is preserved at its reported target");
    assert!(!first.exists() && !second.exists(), "the moved sources leave their folders");
}

#[test]
fn a_batch_replace_keeps_case_only_destinations_distinct_when_the_volume_does() {
    let scratch = Scratch::new();
    let one = scratch.dir("one");
    let lower_source = one.join("part.sc");
    fs::write(&lower_source, "lower-new").unwrap();
    let two = scratch.dir("two");
    let upper_source = two.join("Part.sc");
    fs::write(&upper_source, "upper-new").unwrap();
    let into = scratch.dir("into");
    let lower = into.join("part.sc");
    fs::write(&lower, "lower-old").unwrap();
    let upper = into.join("Part.sc");
    fs::write(&upper, "upper-old").unwrap();
    if fs::read(&lower).unwrap() != b"lower-old" {
        // A case-insensitive volume has one entry, so there are not two distinct destinations.
        return;
    }
    let mut tree = listing(scratch.path());
    let sources = vec![lower_source.clone(), upper_source.clone()];
    tree.place(sources.clone(), &into, false, None);
    let replaceable = asked_replacements(&tree);
    tree.resolve_conflict(sources, into.clone(), false, Conflict::Replace, &replaceable);
    let answer = wait_for_answer(&mut tree);
    let result = |source: &std::path::Path| {
        answer.results.iter().find(|result| result.source == source).unwrap_or_else(|| panic!("no result for {}", source.display()))
    };
    assert_eq!(result(&lower_source).target, lower, "the first spelling keeps its own destination");
    assert_eq!(result(&upper_source).target, upper, "the second spelling keeps its own destination");
    assert_eq!(fs::read(&lower).unwrap(), b"lower-new");
    assert_eq!(fs::read(&upper).unwrap(), b"upper-new");
}

/// The sources the conflict question actually offered for replacement, as the panel would read
/// them when it takes the modal and answers it.
fn asked_replacements(tree: &FileTree) -> Vec<super::ReplaceConsent> {
    match &tree.modal {
        Some(Modal::Conflict { replaceable, .. }) => replaceable.clone(),
        other => panic!("the conflict question is not open: {}", other.is_some()),
    }
}

/// Wait for the operation worker's answer without a wall-clock deadline: the test process has a
/// few frames of its own.
fn wait_for_answer(tree: &mut FileTree) -> OpAnswer {
    for _ in 0..10_000 {
        let answers = tree.take_answers();
        if let Some(answer) = answers.into_iter().next() {
            return answer;
        }
        std::thread::yield_now();
    }
    panic!("the file operation did not finish");
}

#[test]
fn a_design_that_moved_with_its_folder_keeps_its_open_tab_and_dirty_edits() {
    let scratch = Scratch::new();
    let folder = scratch.dir("parts");
    let old = folder.join("bracket.sc");
    let mut session = Session::default();
    session.execute("file.save_as", &serde_json::json!({"path": old.to_string_lossy(), "cache": false})).unwrap();
    session.execute("solid.box", &serde_json::json!({"length": 10, "width": 10, "height": 10})).unwrap();
    assert!(session.is_dirty());
    let mut app = crate::SolveApp::new(session, Default::default());
    let new_folder = scratch.dir("new-parts");
    let new = new_folder.join("bracket.sc");
    fs::rename(&folder, &new_folder).unwrap();
    let count = documents::retarget_paths(&mut app, &folder, &new_folder);
    assert_eq!(count, 1);
    assert_eq!(app.session.path, Some(new.to_string_lossy().into_owned()));
    assert!(app.session.is_dirty(), "the unsaved edit is still there");
    let outcome = files::rename(&new, "renamed.sc", Conflict::Skip);
    assert!(outcome.succeeded(), "{outcome:?}");
    assert_eq!(documents::retarget_paths(&mut app, &new, &outcome.target), 1);
    assert_eq!(app.session.path, Some(outcome.target.to_string_lossy().into_owned()));
    assert_eq!(app.session.doc.name, "renamed", "a name borrowed from the file follows it");
    app.session.execute("file.save", &serde_json::json!({})).unwrap();
    assert!(outcome.target.is_file(), "Save writes the renamed file");
    assert!(!new.exists(), "the old name is not written again");
    assert!(!app.session.is_dirty());
}

#[test]
fn a_new_design_is_written_as_a_loadable_sc_and_asks_to_open() {
    let scratch = Scratch::new();
    let mut tree = listing(scratch.path());
    tree.editing =
        Some(Edit { parent: scratch.path().to_path_buf(), path: None, kind: EditKind::Model, name: "Bracket".into(), error: None, focus: true });
    let open = tree.commit_edit();
    let file = scratch.path().join("Bracket.sc");
    assert_eq!(open.as_deref(), Some(file.as_path()), "the new design is the one to open");
    assert!(file.is_file());
    let loaded = solidcraft_engine::io::read_design_file(&file.to_string_lossy(), None::<fn(solidcraft_engine::io::DesignPreview)>);
    assert!(loaded.is_ok(), "the file loads as a design");
    assert_eq!(tree.selection.iter().next(), Some(&file));
}

#[test]
fn a_name_that_is_taken_keeps_the_editor_open_with_the_reason() {
    let scratch = Scratch::new();
    scratch.file("taken.sc", "");
    let mut tree = listing(scratch.path());
    tree.editing =
        Some(Edit { parent: scratch.path().to_path_buf(), path: None, kind: EditKind::File, name: "taken.sc".into(), error: None, focus: true });
    assert!(tree.commit_edit().is_none());
    let edit = tree.editing.as_ref().expect("the editor stays for another name");
    assert!(edit.error.is_some());
}

#[test]
fn a_rename_reports_the_move_so_open_designs_can_follow() {
    let scratch = Scratch::new();
    let old = scratch.file("bracket.sc", "");
    let mut tree = listing(scratch.path());
    tree.editing = Some(Edit {
        parent: scratch.path().to_path_buf(),
        path: Some(old.clone()),
        kind: EditKind::Rename,
        name: "bracket-v2.sc".into(),
        error: None,
        focus: true,
    });
    tree.commit_edit();
    assert_eq!(tree.take_retargets(), vec![(old, scratch.path().join("bracket-v2.sc"))]);
    assert!(!scratch.path().join("bracket.sc").exists());
}

#[test]
fn deleting_a_dirty_design_names_it_in_the_question_and_a_copy_saves_the_work_first() {
    let scratch = Scratch::new();
    let path = scratch.file("bracket.sc", "old");
    let mut session = Session::default();
    session.path = Some(path.to_string_lossy().into_owned());
    session.execute("solid.box", &serde_json::json!({"length": 4, "width": 4, "height": 4})).unwrap();
    let mut app = crate::SolveApp::new(session, Default::default());
    let mut tree = listing(scratch.path());
    tree.select_only(&path);
    let open: Vec<(String, bool)> = vec![(path.to_string_lossy().into_owned(), true)];
    tree.request_delete(&open);
    match &tree.modal {
        Some(Modal::Delete { dirty, .. }) => assert_eq!(dirty.len(), 1, "the dirty design is named"),
        _ => panic!("the delete question is open"),
    }
    let saved = crate::file_tree::save_copies(&mut app, std::slice::from_ref(&path)).unwrap();
    assert_eq!(saved, 1);
    assert!(scratch.path().join("bracket (copy).sc").is_file(), "the work is kept as a copy");
    assert!(app.session.path.as_deref().is_some_and(|p| p.ends_with("bracket (copy).sc")));
    assert!(!app.session.is_dirty(), "the copy holds the work");
}

#[test]
fn an_operation_that_cannot_trash_asks_before_deleting_for_good() {
    let mut tree = FileTree::new();
    let path = std::path::PathBuf::from("/tmp/intcad-no-trash-test.sc");
    let answer = OpAnswer { action: Action::Trash, results: Vec::new(), no_trash: vec![path.clone()], cancelled: false };
    let _ = tree.apply_answer(answer);
    match &tree.modal {
        Some(Modal::Permanent { paths, .. }) => assert_eq!(paths, &vec![path]),
        _ => panic!("a second question is asked"),
    }
}

#[test]
fn an_external_change_is_shown_and_kept_until_dismissed() {
    let scratch = Scratch::new();
    let path = scratch.file("bracket.sc", "v1");
    let mut tree = listing(scratch.path());
    tree.poll_documents(&[(path.to_string_lossy().into_owned(), false, true, false)], 100.0);
    assert_eq!(tree.external.len(), 1);
    assert!(!tree.external[0].missing);
    tree.dismiss_external(&path.to_string_lossy());
    tree.poll_documents(&[(path.to_string_lossy().into_owned(), false, true, false)], 200.0);
    assert!(tree.external.is_empty(), "a dismissed change is not shown again");
}

#[test]
fn a_missing_design_is_reported_as_gone_rather_than_changed() {
    let scratch = Scratch::new();
    let path = scratch.path().join("gone.sc");
    let mut tree = listing(scratch.path());
    tree.poll_documents(&[(path.to_string_lossy().into_owned(), true, true, true)], 10.0);
    assert!(tree.external[0].missing && tree.external[0].dirty);
}

/// The panel in a real window: its empty state offers a folder, and a chosen folder shows its
/// rows in the right rail.
#[test]
fn the_files_panel_draws_in_the_right_rail_and_lists_the_chosen_folder() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "");
    scratch.dir("parts");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.frames(3);
    assert!(rect_of("files:empty").is_some(), "the empty state offers a folder");
    assert!(rect_of("files_open_folder").is_some());
    h.app.file_tree.set_root(Some(scratch.path().to_path_buf()));
    h.frames(30);
    let root = rect_of("files:root").expect("the folder name is in the panel");
    let parts = rect_of("files:row:parts").expect("a folder row");
    let bracket = rect_of("files:row:bracket.sc").expect("a file row");
    let panel = rect_of("rail:panel").expect("the panel is open");
    assert!(panel.contains_rect(root) && panel.contains_rect(parts) && panel.contains_rect(bracket), "{panel:?}");
    assert!(parts.top() < bracket.top(), "folders stand first");
}

/// A design opened from the tree goes through the document path: its tab shows the file, and a
/// second activation focuses that tab instead of opening a second one.
#[test]
fn opening_a_design_from_the_tree_uses_one_tab() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    let path = scratch.path().join("bracket.sc");
    Session::default().execute("file.save_as", &serde_json::json!({"path": path.to_string_lossy(), "cache": false})).unwrap();
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.app.file_tree.set_root(Some(scratch.path().to_path_buf()));
    h.frames(30);
    assert!(rect_of("files:row:bracket.sc").is_some());
    h.step(&json!({"click": {"handle": "files:row:bracket.sc"}, "double": true})).unwrap();
    h.frames(60);
    assert_eq!(h.app.session.path.as_deref(), Some(path.to_string_lossy().as_ref()));
    let tabs = h.app.docs.count();
    h.step(&json!({"click": {"handle": "files:row:bracket.sc"}, "double": true})).unwrap();
    h.frames(60);
    assert_eq!(h.app.docs.count(), tabs, "the same path focuses its tab");
}

#[test]
fn a_copy_of_a_design_with_unsaved_edits_says_the_copy_uses_the_saved_file() {
    let scratch = Scratch::new();
    let source = scratch.file("bracket.sc", "saved");
    let into = scratch.dir("parts");
    let mut tree = listing(scratch.path());
    let path = source.to_string_lossy().into_owned();
    tree.set_open_docs(&[(path, true, false, false)]);
    tree.place(vec![source], &into, false, None);
    let notice = tree.notice.as_ref().map(|(text, _)| text.as_str()).unwrap_or_default();
    assert!(notice.contains("saved file"), "the copy's content is explained: {notice}");
}

#[test]
fn locating_a_design_opens_the_folders_above_it_and_selects_it() {
    let scratch = Scratch::new();
    let mut parent = scratch.path().to_path_buf();
    for name in ["a", "b", "c"] {
        parent = parent.join(name);
        fs::create_dir(&parent).unwrap();
    }
    let file = parent.join("deep.sc");
    fs::write(&file, "").unwrap();
    let mut tree = listing(scratch.path());
    tree.reveal_path(&file);
    assert!(tree.expanded.contains(&parent), "the folders above it are open");
    assert!(tree.selection.contains(&file));
    assert_eq!(tree.scroll_to.as_deref(), Some(file.as_path()));
}

#[cfg(unix)]
#[test]
fn a_link_to_a_folder_is_not_opened_as_a_folder_in_the_tree() {
    let scratch = Scratch::new();
    let real = scratch.dir("real");
    fs::write(real.join("inside.sc"), "").unwrap();
    let link = scratch.path().join("shortcut");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let mut tree = listing(scratch.path());
    assert!(matches!(tree.kind_at(&link), Some(files::Kind::Link { to_dir: true })));
    let opened = tree.activate(&link).expect("a link is handed to the system, not expanded");
    assert_eq!(opened, link);
    assert!(!tree.expanded.contains(&link), "the link's target is not walked");
}

/// The control channel can browse a folder and read its rows: the same door the desktop E2E
/// uses to drive the panel.
#[test]
fn the_control_channel_reads_the_panels_rows() {
    use crate::scenario::Harness;
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "");
    scratch.dir("parts");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    let reply = h.call("ui.files", json!({}));
    assert_eq!(reply["ok"], true, "{reply}");
    let rows = reply["result"]["rows"].as_array().expect("rows");
    let names: Vec<&str> = rows.iter().filter_map(|row| row["name"].as_str()).collect();
    assert!(names.contains(&"bracket.sc") && names.contains(&"parts"), "{names:?}");
    assert_eq!(reply["result"]["root"], json!(scratch.path().to_string_lossy()));
    let refused = h.call("ui.files", json!({"root": scratch.path().join("missing")}));
    assert_eq!(refused["ok"], false, "a missing folder is refused");
}

/// The `+` menu opens on a click and its New File asks for a name in the tree; Enter makes the
/// file with exactly the typed name.
#[test]
fn the_new_menu_makes_a_file_with_the_typed_name() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    h.step(&json!({"click": {"handle": "files:new"}})).unwrap();
    h.frames(3);
    assert!(rect_of("menu:files_new_file").is_some(), "the new menu opens on a click");
    h.step(&json!({"click": {"handle": "menu:files_new_file"}})).unwrap();
    h.frames(3);
    assert!(rect_of("files:edit").is_some(), "the name field is in the tree");
    h.step(&json!({"text": "notes.txt"})).unwrap();
    h.step(&json!({"key": "Enter"})).unwrap();
    h.frames(5);
    assert!(scratch.path().join("notes.txt").is_file(), "the typed name made the file");
}

/// Moving a folder that holds two open designs moves both tabs, and both keep their unsaved
/// work: the second test of AC-09, which a single open design cannot prove.
#[test]
fn moving_a_folder_moves_every_open_design_inside_it() {
    let scratch = Scratch::new();
    let folder = scratch.dir("parts");
    let first = folder.join("a.sc");
    let second = folder.join("b.sc");
    let mut one = Session::default();
    one.execute("file.save_as", &serde_json::json!({"path": first.to_string_lossy(), "cache": false})).unwrap();
    one.execute("solid.box", &serde_json::json!({"length": 3, "width": 3, "height": 3})).unwrap();
    let mut two = Session::default();
    two.execute("file.save_as", &serde_json::json!({"path": second.to_string_lossy(), "cache": false})).unwrap();
    two.execute("solid.box", &serde_json::json!({"length": 5, "width": 5, "height": 5})).unwrap();
    let mut app = crate::SolveApp::new(one, Default::default());
    crate::documents::open_tab(&mut app, two).unwrap();
    let moved = scratch.dir("moved");
    fs::rename(&folder, &moved).unwrap();
    assert_eq!(documents::retarget_paths(&mut app, &folder, &moved), 2);
    assert_eq!(app.session.path, Some(moved.join("b.sc").to_string_lossy().into_owned()));
    let open = documents::open_sessions(&app);
    let other = open.iter().find(|session| session.path.as_deref() == Some(moved.join("a.sc").to_string_lossy().as_ref()));
    assert!(other.is_some(), "the design in the other tab followed the folder");
    assert!(open.iter().all(|session| session.is_dirty()), "both keep their edits");
}

/// A batch reports each item: the good copy is made and the missing source is a failure, so a
/// partial success is never shown as one success.
#[test]
fn a_batch_reports_the_good_item_and_the_failed_one() {
    let scratch = Scratch::new();
    let good = scratch.file("good.sc", "good");
    let missing = scratch.path().join("missing.sc");
    let into = scratch.dir("parts");
    let mut tree = listing(scratch.path());
    tree.place(vec![good, missing], &into, false, None);
    let answer = wait_for_answer(&mut tree);
    assert_eq!(answer.results.len(), 2);
    assert_eq!(answer.results.iter().filter(|r| r.succeeded()).count(), 1);
    assert_eq!(answer.results.iter().filter(|r| matches!(r.outcome, files::Outcome::Failed(_))).count(), 1);
    assert!(into.join("good.sc").is_file());
    let report = tree.apply_answer(answer);
    let _ = report;
    let report = tree.report.as_ref().unwrap();
    assert_eq!((report.succeeded(), report.failed()), (1, 1));
    assert_eq!(report.retry.len(), 1, "only the failed item can run again");
}

/// The secondary click opens the row menu, and Rename in it opens the name field. The rows are
/// virtualized, so the menu is drawn by the panel rather than by the row that asked for it.
#[test]
fn a_right_click_opens_the_row_menu_and_rename_edits_in_place() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("notes.txt", "");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    let row = rect_of("files:row:notes.txt").expect("the row");
    h.call("ui.click", json!({"x": row.center().x, "y": row.center().y, "button": "right"}));
    h.frames(2);
    let rename = rect_of("menu:files_rename").expect("the row menu opened");
    h.call("ui.click", json!({"x": rename.center().x, "y": rename.center().y}));
    h.frames(2);
    assert!(rect_of("files:edit").is_some(), "Rename opened the name field");
}

/// A click with the command modifier adds the row to the selection and nothing else. On macOS it
/// used to open the row menu as well: the secondary click there is Control-click.
#[test]
fn a_command_click_adds_to_the_selection_without_opening_the_row_menu() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    let first = scratch.file("first.txt", "");
    let second = scratch.file("second.txt", "");
    let mut h = Harness::new();
    h.app.home.open = false;
    // The remote fixture never set the platform, so its macOS branch could not run on Linux.
    // Task 2 decides the click's role from the runtime OS, so name macOS explicitly.
    h.ctx.set_os(egui::os::OperatingSystem::Mac);
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    let row = rect_of("files:row:first.txt").expect("the first row");
    h.call("ui.click", json!({"x": row.center().x, "y": row.center().y}));
    h.frames(2);
    let row = rect_of("files:row:second.txt").expect("the second row");
    h.call("ui.click", json!({"x": row.center().x, "y": row.center().y, "ctrl": true}));
    h.frames(2);
    assert!(h.app.file_tree.selection.contains(&first) && h.app.file_tree.selection.contains(&second), "{:?}", h.app.file_tree.selection);
    assert!(h.app.file_tree.context.is_none(), "no row menu");
    assert!(rect_of("menu:files_rename").is_none());
}

/// The Enter that finishes a rename belongs to the name field: it must not also open the row,
/// which would put a second tab on the renamed file beside the one being edited.
#[test]
fn the_enter_that_renames_a_design_does_not_open_it_again() {
    use crate::scenario::Harness;
    use serde_json::json;
    let scratch = Scratch::new();
    let path = scratch.path().join("bracket.sc");
    Session::default().execute("file.save_as", &json!({"path": path.to_string_lossy(), "cache": false})).unwrap();
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    let row = crate::scenario::rect_of("files:row:bracket.sc").expect("the row");
    h.call("ui.click", json!({"x": row.center().x, "y": row.center().y, "double": true}));
    h.frames(60);
    let before = h.app.docs.count();
    h.call("ui.click", json!({"x": row.center().x, "y": row.center().y}));
    h.frames(2);
    h.step(&json!({"key": "F2"})).unwrap();
    h.frames(2);
    h.step(&json!({"key": "A", "ctrl": true})).unwrap();
    h.step(&json!({"text": "bracket-v2.sc"})).unwrap();
    h.step(&json!({"key": "Enter"})).unwrap();
    h.frames(20);
    assert_eq!(h.app.docs.count(), before, "rename opens no second tab");
    let names: Vec<String> = crate::documents::open_sessions(&h.app).iter().map(|s| s.doc.name.clone()).collect();
    assert!(names.iter().any(|name| name == "bracket-v2"), "{names:?}");
    assert!(!names.iter().any(|name| name == "bracket"), "the old tab was renamed, not kept: {names:?}");
}

/// Cancel is an answer: it closes a file question and changes nothing. Before the fix the
/// confirmation put the same question back, so Cancel appeared to do nothing.
#[test]
fn cancel_closes_the_conflict_question() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("bracket.sc", "");
    scratch.file("notes.txt", "");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    h.step(&json!({"click": {"handle": "files:row:bracket.sc"}})).unwrap();
    h.step(&json!({"key": "C", "ctrl": true})).unwrap();
    h.step(&json!({"click": {"handle": "files:row:notes.txt"}})).unwrap();
    h.step(&json!({"key": "V", "ctrl": true})).unwrap();
    h.frames(2);
    assert!(rect_of("dialog:Keep Both").is_some() || crate::scenario::handle_at("dialog:Keep Both").is_some(), "the conflict question is open");
    h.step(&json!({"click": {"handle": "dialog:Cancel"}})).unwrap();
    h.frames(2);
    assert!(crate::scenario::handle_at("dialog:Keep Both").is_none(), "Cancel closes the question");
    assert!(!scratch.path().join("bracket (copy).sc").exists(), "Cancel copies nothing");
}

/// A file the system cannot open says why in the panel and offers its folder. The desktop's own
/// message is a worker's answer, so the test hands the panel the same answer.
#[test]
fn an_open_failure_names_the_file_and_offers_its_folder() {
    use crate::scenario::{Harness, rect_of};
    use serde_json::json;
    let scratch = Scratch::new();
    let path = scratch.path().join("orphan.bin");
    std::fs::write(&path, [0u8, 1, 2]).unwrap();
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.call("ui.files", json!({"root": scratch.path().to_string_lossy()}));
    h.frames(20);
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send((path.clone(), Err(files::NO_ASSOCIATED_PROGRAM.to_string()))).unwrap();
    h.app.file_tree.open_jobs.push(receiver);
    h.frames(3);
    let state = h.call("ui.files", json!({}));
    let notice = state["result"]["notice"].clone();
    assert_eq!(notice["error"], true, "{notice}");
    assert!(notice["text"].as_str().is_some_and(|text| text.contains("orphan.bin")), "{notice}");
    assert!(!notice["text"].as_str().unwrap().contains(files::NO_ASSOCIATED_PROGRAM), "use a localized reason, not the IO category");
    assert!(rect_of("files:notice").is_some());
    assert!(crate::scenario::handle_at("dialog:button::Show in File Manager").is_some(), "the folder is reachable");
}

/// Presence alone missed a footer below the window edge. A long non-category opener error
/// must retain its diagnostic while the bounded notice and both recovery buttons fit.
#[test]
fn a_long_opener_notice_keeps_reveal_and_dismiss_inside_a_short_panel() {
    use crate::scenario::rect_of;
    use serde_json::json;
    let scratch = Scratch::new();
    let path = scratch.file("orphan.bin", "");
    let mut h = clipboard_window(scratch.path());
    h.call("ui.resize", json!({"width": 850, "height": 480}));
    h.frames(3);
    let error =
        format!("No application knows how to open URL {} (Error Domain=NSOSStatusErrorDomain Code=-10814 {})", path.display(), "detail ".repeat(80));
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send((path.clone(), Err(error.clone()))).unwrap();
    h.app.file_tree.open_jobs.push(receiver);
    h.frames(3);
    assert!(h.app.file_tree.notice.as_ref().unwrap().0.contains(&error), "unknown failures are not misclassified");
    let panel = rect_of("rail:panel").unwrap();
    for handle in ["files:notice", "files:notice_reveal", "files:notice_dismiss"] {
        let rect = rect_of(handle).unwrap_or_else(|| panic!("missing {handle}"));
        assert!(panel.contains_rect(rect), "{handle}: {rect:?} outside {panel:?}");
        assert!(rect.is_positive());
    }
}

/// Click the named row in a Files panel already listing `root`, as a person does, and check the
/// tree took the keys.
fn focus_file_row(h: &mut crate::scenario::Harness, name: &str) {
    let row = crate::scenario::rect_of(&format!("files:row:{name}")).unwrap_or_else(|| panic!("missing row {name}"));
    h.frame_with(pointer_click(row.center(), Modifiers::NONE));
    h.frames(2);
    assert!(h.app.file_tree.focused, "{name}: the click gives the tree the keys");
}

/// A real arrow key in a focused Files tree moves the row's focus, not the view. The view keys
/// used to run before the tree saw the same press: the camera turned and the row moved.
#[test]
fn arrows_in_a_focused_files_tree_do_not_turn_the_view() {
    let scratch = Scratch::new();
    scratch.file("first.txt", "1");
    scratch.file("second.txt", "2");
    let mut h = clipboard_window(scratch.path());
    h.app.file_tree.select_only(&scratch.path().join("first.txt"));
    focus_file_row(&mut h, "first.txt");
    let cam = h.app.cam;
    h.step(&serde_json::json!({"key": "ArrowDown"})).unwrap();
    h.frames(60);
    assert_eq!(h.app.file_tree.focus, Some(scratch.path().join("second.txt")), "the tree moved its focus");
    assert_eq!(h.app.cam, cam, "the view did not turn");
    assert!(h.app.viewport.key_hold.is_none(), "no view key was held");
}

/// F2 in a focused Files tree opens the file's name field. It used to open the selected body's
/// rename as well, leaving two name fields for one press.
#[test]
fn f2_renames_the_focused_file_and_not_the_selected_body() {
    use crate::scenario::Harness;
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.app.file_tree = listing(scratch.path());
    h.app.run("solid.box", json!({"length": 5, "width": 5, "height": 5})).unwrap();
    let body = h.app.session.model.state().bodies.last().unwrap().name.clone();
    h.app.run("select.set", json!({"items": [{"type": "body", "name": body}]})).unwrap();
    h.frames(5);
    focus_file_row(&mut h, "notes.txt");
    assert!(h.app.menu.rename.is_none(), "no model rename is open yet");
    h.step(&json!({"key": "F2"})).unwrap();
    h.frames(2);
    assert!(h.app.file_tree.editing.is_some(), "the file's name field opens");
    assert!(h.app.menu.rename.is_none(), "the selected body does not also open a rename");
}

/// Delete on a file row asks about the file and leaves the body selected in the view alone.
#[test]
fn delete_on_a_file_row_leaves_the_body_selected_in_the_view() {
    use crate::scenario::Harness;
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.app.file_tree = listing(scratch.path());
    h.app.run("solid.box", json!({"length": 5, "width": 5, "height": 5})).unwrap();
    let body = h.app.session.model.state().bodies.last().unwrap().name.clone();
    h.app.run("select.set", json!({"items": [{"type": "body", "name": body}]})).unwrap();
    h.frames(5);
    focus_file_row(&mut h, "notes.txt");
    h.step(&json!({"key": "Delete"})).unwrap();
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_some(), "the body stays");
    assert!(h.app.file_tree.modal.is_some(), "the tree asks about its file");
}

/// A Files panel listing `notes.txt` beside a design with one selected body.
fn cad_body_and_files(scratch: &Scratch) -> (crate::scenario::Harness, String) {
    use crate::scenario::Harness;
    use serde_json::json;
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.app.file_tree = listing(scratch.path());
    h.app.run("solid.box", json!({"length": 5, "width": 5, "height": 5})).unwrap();
    let body = h.app.session.model.state().bodies.last().unwrap().name.clone();
    h.app.run("select.set", json!({"items": [{"type": "body", "name": body}]})).unwrap();
    h.frames(5);
    (h, body)
}

/// On macOS the panel declines Control+Backspace (no Command), and that chord used to reach the
/// CAD selection and delete the body. A destructive chord the tree does not act on must not fall
/// through to CAD, while Control+Arrow stays the view's.
#[test]
fn macos_control_backspace_in_focused_files_does_not_delete_the_selected_body() {
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let (mut h, body) = cad_body_and_files(&scratch);
    h.ctx.set_os(egui::os::OperatingSystem::Mac);
    focus_file_row(&mut h, "notes.txt");
    let control = Modifiers { ctrl: true, ..Default::default() };
    h.frame_with(vec![key_event(egui::Key::Backspace, true, control), key_event(egui::Key::Backspace, false, control)]);
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_some(), "Control+Backspace must not delete the CAD body");
    assert!(h.app.file_tree.modal.is_none(), "the tree declines the Control chord without asking about a file");
    assert!(h.app.file_tree.editing.is_none());
}

/// A Files row click and Delete in one batch: before layout the old CAD focus made the pre-layout
/// guard miss, so Delete deleted the CAD body before Files took the keys.
#[test]
fn same_batch_files_row_click_and_delete_does_not_delete_the_cad_selection() {
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let (mut h, body) = cad_body_and_files(&scratch);
    assert!(!h.app.file_tree.focused, "CAD has the keys before the batch");
    let row = crate::scenario::rect_of("files:row:notes.txt").unwrap();
    let mut events = pointer_click(row.center(), Modifiers::NONE);
    events.push(key_event(egui::Key::Delete, true, Modifiers::NONE));
    events.push(key_event(egui::Key::Delete, false, Modifiers::NONE));
    h.frame_with(events);
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_some(), "the click and Delete in one batch must not delete CAD");
    assert!(h.app.file_tree.focused, "the Files row takes the keys");
    assert!(h.app.file_tree.modal.is_some(), "Files asks about its own file");
}

/// The ordered click position, not the batch's final pointer position, decides that CAD must not
/// delete. When the pointer leaves Files before its handler runs, the same ambiguous batch
/// suppresses both owners: CAD keeps its body and the tree asks about no file. The settled
/// click-then-Delete case without the move keeps the safe Files question below.
#[test]
fn files_row_click_then_delete_then_pointer_away_does_not_delete_cad_or_ask() {
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let (mut h, body) = cad_body_and_files(&scratch);
    let row = crate::scenario::rect_of("files:row:notes.txt").unwrap();
    let mut events = pointer_click(row.center(), Modifiers::NONE);
    events.push(key_event(egui::Key::Delete, true, Modifiers::NONE));
    events.push(key_event(egui::Key::Delete, false, Modifiers::NONE));
    events.push(egui::Event::PointerMoved(egui::pos2(5.0, 5.0)));
    h.frame_with(events);
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_some(), "the final pointer move must not hand the key to CAD");
    assert!(h.app.file_tree.modal.is_none(), "the pointer left Files before its handler ran, so the key is retired from both owners");
    assert!(h.app.file_tree.editing.is_none());
}

/// A completed Files click, then a mouse button outside Files (held, released before Delete, or
/// released after moving back), then Delete: an outside press is not a completed click and must
/// not override the Files click or authorize CAD deletion. The uncertain key is retired for both.
#[test]
fn files_click_then_outside_press_around_delete_retires_both() {
    for (label, release_before, release_after) in
        [("held", false, false), ("released before Delete", true, false), ("released after moving back", false, true)]
    {
        let scratch = Scratch::new();
        scratch.file("notes.txt", "n");
        let (mut h, body) = cad_body_and_files(&scratch);
        let row = crate::scenario::rect_of("files:row:notes.txt").unwrap();
        let outside = egui::pos2(5.0, 5.0);
        let mut events = pointer_click(row.center(), Modifiers::NONE);
        events.push(egui::Event::PointerButton { pos: outside, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        if release_before {
            events.push(egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            });
        }
        events.push(key_event(egui::Key::Delete, true, Modifiers::NONE));
        events.push(key_event(egui::Key::Delete, false, Modifiers::NONE));
        events.push(egui::Event::PointerMoved(row.center()));
        if release_after {
            events.push(egui::Event::PointerButton {
                pos: row.center(),
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Modifiers::NONE,
            });
        }
        h.frame_with(events);
        h.frames(2);
        assert!(h.app.session.model.state().body(&body).is_some(), "{label}: the outside button must not authorize CAD deletion");
        assert!(h.app.file_tree.modal.is_none(), "{label}: the uncertain key is retired for Files too");
    }
}

/// Delete followed by a Files row click must not be retargeted to the file the later click
/// selects. The key is retired for both owners.
#[test]
fn delete_then_files_row_click_does_not_open_a_file_question() {
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let (mut h, body) = cad_body_and_files(&scratch);
    let row = crate::scenario::rect_of("files:row:notes.txt").unwrap();
    let mut events = vec![key_event(egui::Key::Delete, true, Modifiers::NONE), key_event(egui::Key::Delete, false, Modifiers::NONE)];
    events.extend(pointer_click(row.center(), Modifiers::NONE));
    h.frame_with(events);
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_some(), "the key before the click must not delete CAD");
    assert!(h.app.file_tree.modal.is_none(), "the later Files click must not retarget the earlier Delete");
    assert!(h.app.file_tree.editing.is_none());
    assert!(h.app.file_tree.focused, "the click still focuses Files; only the key is retired");
}

/// The reverse transition loses the ambiguous destructive key rather than guessing: a click that
/// closes Files and Delete in one batch changes nothing.
#[test]
fn same_batch_click_away_from_files_and_delete_changes_nothing() {
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let (mut h, body) = cad_body_and_files(&scratch);
    focus_file_row(&mut h, "notes.txt");
    let item = crate::scenario::rect_of("rail:item:files").unwrap();
    let mut events = pointer_click(item.center(), Modifiers::NONE);
    events.push(key_event(egui::Key::Delete, true, Modifiers::NONE));
    events.push(key_event(egui::Key::Delete, false, Modifiers::NONE));
    h.frame_with(events);
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_some(), "the ambiguous Delete deletes neither owner's selection");
    assert!(h.app.file_tree.modal.is_none(), "the closed panel asks about no file");
    assert!(!h.app.file_tree.focused);
}

/// A window too narrow to draw the Files panel has no Files body to own the keys: CAD Delete
/// must work even in the frame the panel disappears.
#[test]
fn a_narrow_frame_that_omits_files_lets_cad_delete() {
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let (mut h, body) = cad_body_and_files(&scratch);
    focus_file_row(&mut h, "notes.txt");
    let wide_panel = h.app.shell.right_rail_panel.expect("the wide Files panel was drawn");
    h.call("ui.resize", json!({"width": 500, "height": 700}));
    assert!(h.app.shell.right_rail_panel.is_none(), "the panel cannot be drawn in this window");
    // The frame that shrinks the window still carries the previous frame's focus and panel:
    // reproduce that one transition frame and feed Delete into it.
    h.app.file_tree.focused = true;
    h.app.shell.right_rail_panel = Some(wide_panel);
    h.frame_with(vec![key_event(egui::Key::Delete, true, Modifiers::NONE), key_event(egui::Key::Delete, false, Modifiers::NONE)]);
    h.frames(3);
    assert!(h.app.session.model.state().body(&body).is_none(), "a panel that cannot be drawn must not suppress CAD Delete");
    assert!(!h.app.file_tree.focused);
    assert!(h.app.file_tree.modal.is_none());
}

/// With the Files panel open but not focused, CAD still owns Delete.
#[test]
fn settled_cad_focus_still_deletes_with_files_panel_open() {
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let (mut h, body) = cad_body_and_files(&scratch);
    assert!(!h.app.file_tree.focused);
    h.step(&serde_json::json!({"key": "Delete"})).unwrap();
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_none(), "the model still owns Delete while Files is open but unfocused");
}

/// Destructive keys are the ambiguity boundary; Control+Arrow is not claimed by Files.
#[test]
fn files_key_ownership_leaves_control_arrows_to_the_view() {
    let control = Modifiers { ctrl: true, ..Default::default() };
    assert!(!crate::file_tree::takes_key(egui::Key::ArrowLeft, control), "Control+Arrow is not a Files key");
    assert!(!crate::file_tree::takes_key(egui::Key::ArrowRight, control));
    assert!(crate::file_tree::takes_key(egui::Key::ArrowLeft, Modifiers::NONE), "a plain arrow is");
}

/// With the Files panel closed the tree has no keys: the view's arrows and the model's Delete
/// work as they always have.
#[test]
fn a_closed_files_panel_gives_arrows_and_delete_back_to_the_model() {
    use crate::scenario::Harness;
    use serde_json::json;
    let scratch = Scratch::new();
    scratch.file("notes.txt", "n");
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true, "right_rail_open": "files"}));
    h.app.file_tree = listing(scratch.path());
    h.app.run("solid.box", json!({"length": 5, "width": 5, "height": 5})).unwrap();
    let body = h.app.session.model.state().bodies.last().unwrap().name.clone();
    h.app.run("select.set", json!({"items": [{"type": "body", "name": body}]})).unwrap();
    h.frames(5);
    focus_file_row(&mut h, "notes.txt");
    crate::right_rail::toggle(&mut h.app, crate::right_rail::Item::Files);
    h.frames(3);
    assert!(!h.app.file_tree.focused, "a closed panel has no keys");
    let cam = h.app.cam;
    h.step(&json!({"key": "ArrowDown"})).unwrap();
    h.frames(60);
    assert_ne!(h.app.cam, cam, "the view's arrow works again");
    h.step(&json!({"key": "Delete"})).unwrap();
    h.frames(2);
    assert!(h.app.session.model.state().body(&body).is_none(), "the model's Delete works again");
}

/// A stand-in for the Windows clipboard, which another program can own: its text refuses a
/// new file list until the clipboard is emptied, or (with `append`) stays beside the list.
#[cfg(not(target_arch = "wasm32"))]
struct FakeSystemClipboard {
    text: bool,
    append: bool,
    unreadable_paths: bool,
    can_reclaim: bool,
    files: bool,
    sets: usize,
    empties: usize,
}

#[cfg(not(target_arch = "wasm32"))]
impl FakeSystemClipboard {
    fn new(text: bool) -> Self {
        Self { text, append: false, unreadable_paths: false, can_reclaim: true, files: false, sets: 0, empties: 0 }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SystemFileClipboard for FakeSystemClipboard {
    fn set_files(&mut self, _paths: &[PathBuf]) -> Result<(), arboard::Error> {
        self.sets += 1;
        if self.unreadable_paths {
            return Err(arboard::Error::ConversionFailure);
        }
        // Another program owns the clipboard; a second format is refused until it is emptied.
        if self.text && !self.append {
            return Err(arboard::Error::ClipboardOccupied);
        }
        self.files = true;
        Ok(())
    }

    fn empty(&mut self) -> Result<(), arboard::Error> {
        self.empties += 1;
        self.text = false;
        self.files = false;
        Ok(())
    }

    fn text_present(&mut self) -> bool {
        self.text
    }

    fn empty_can_help(&self, error: &arboard::Error) -> bool {
        self.can_reclaim && !matches!(error, arboard::Error::ConversionFailure)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_files_copy_takes_the_system_clipboard_from_another_program() {
    let scratch = Scratch::new();
    let source = scratch.file("f.txt", "f");
    let mut clipboard = FakeSystemClipboard::new(true);
    assert!(write_file_list(&mut clipboard, std::slice::from_ref(&source)).is_ok());
    assert_eq!((clipboard.sets, clipboard.empties), (2, 1), "the refused write is followed by one empty and one write");
    assert!(!clipboard.text && clipboard.files, "only the file list is left");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_file_list_written_beside_another_program_text_empties_and_writes_again() {
    let scratch = Scratch::new();
    let source = scratch.file("f.txt", "f");
    let mut clipboard = FakeSystemClipboard::new(true);
    clipboard.append = true;
    assert!(write_file_list(&mut clipboard, std::slice::from_ref(&source)).is_ok());
    assert_eq!((clipboard.sets, clipboard.empties), (2, 1), "text that would survive costs one empty");
    assert!(!clipboard.text, "the stale text is gone");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_file_list_that_replaces_an_empty_clipboard_does_not_empty_it_first() {
    let scratch = Scratch::new();
    let source = scratch.file("f.txt", "f");
    let mut clipboard = FakeSystemClipboard::new(false);
    assert!(write_file_list(&mut clipboard, std::slice::from_ref(&source)).is_ok());
    assert_eq!((clipboard.sets, clipboard.empties), (1, 0), "a clean write keeps the clipboard untouched until it succeeds");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn an_unusable_path_set_does_not_cost_the_old_clipboard() {
    let scratch = Scratch::new();
    let source = scratch.file("f.txt", "f");
    let mut clipboard = FakeSystemClipboard::new(true);
    clipboard.unreadable_paths = true;
    assert!(write_file_list(&mut clipboard, std::slice::from_ref(&source)).is_err());
    assert_eq!((clipboard.sets, clipboard.empties), (1, 0), "emptying cannot make a file list out of no path");
    assert!(clipboard.text, "the old clipboard is still there");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_platform_that_cannot_take_the_clipboard_back_keeps_it() {
    let scratch = Scratch::new();
    let source = scratch.file("f.txt", "f");
    let mut clipboard = FakeSystemClipboard::new(true);
    clipboard.can_reclaim = false;
    assert!(write_file_list(&mut clipboard, std::slice::from_ref(&source)).is_err());
    assert_eq!((clipboard.sets, clipboard.empties), (1, 0), "Linux and macOS replace the clipboard instead of emptying it");
    assert!(clipboard.text, "the old clipboard is still there");
}

#[test]
fn a_failed_system_file_write_is_shown_and_the_panel_copy_still_works() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut tree = listing(scratch.path());
    tree.record_copy(vec![source.clone()], false, false);
    let clip = tree.clip.as_ref().expect("the panel's own copy still works");
    assert_eq!(clip.paths, [source]);
    assert!(!clip.cut);
    let (text, error) = tree.notice.clone().expect("a notice");
    assert!(error, "a failed system write is an error notice, not a silent 'Copied' line");
    assert!(text.contains("system file clipboard"), "{text}");
}

#[test]
fn a_written_system_copy_is_reported_without_an_error() {
    let scratch = Scratch::new();
    let source = scratch.file("source.txt", "source");
    let mut tree = listing(scratch.path());
    tree.record_copy(vec![source.clone()], true, true);
    let clip = tree.clip.as_ref().expect("the panel's clipboard");
    assert_eq!(clip.paths, [source]);
    assert!(clip.cut);
    let (text, error) = tree.notice.clone().expect("a notice");
    assert!(!error);
    assert!(text.contains("Cut"), "{text}");
}
