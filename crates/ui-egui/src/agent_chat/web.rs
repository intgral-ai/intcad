//! The Assistant's conversation for a web view: where a desktop host lays the view over the
//! panel, what the page is told (the transcript and the composer's state, as JSON), and the few
//! things the page may ask for. The host owns the web view itself; the page is not trusted with
//! more than the actions listed in [`act`], and every action is checked against the chat's state.

use serde_json::{Value, json};

use super::*;

/// Longest raw tool input or output sent to the page (characters), as the panel's own Details.
const MAX_DETAIL: usize = 8000;
/// Longest message the page may send (characters).
const MAX_MESSAGE: usize = 100_000;

/// Why the desktop host has no web view to offer. The panel shows this as a diagnostic, never as
/// a second conversation: the Assistant has one interface, the page in the host's view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Linux opened a native Wayland window, which has no X11 child window for WebKitGTK.
    NativeWayland,
    /// The view could not be made at all (a missing WebView2 runtime, a GTK/WebKitGTK fault).
    Failed,
}

/// The desktop host says whether it can show the conversation in a web view. Without it, and on
/// the browser build, the panel gives up its conversation to the old egui panel; a desktop host
/// that cannot make a view says why with [`set_problem`] instead.
pub fn set_host(app: &mut SolveApp, available: bool) {
    app.agent_chat.web_host = available;
    if available {
        app.agent_chat.web_problem = None;
    }
}

/// Tell the panel why the host has no view, so it can say what to do instead of drawing a second
/// chat interface.
pub fn set_problem(app: &mut SolveApp, problem: Problem) {
    app.agent_chat.web_problem = Some(problem);
}

/// The panel's Retry: ask the host to try to make the view again, keeping the conversation.
pub fn request_retry(app: &mut SolveApp) {
    app.agent_chat.web_retry = true;
}

/// The host takes a pending Retry, once.
pub fn take_retry(app: &mut SolveApp) -> bool {
    std::mem::take(&mut app.agent_chat.web_retry)
}

/// The host, with the app's logic: the agent's messages are taken in whether or not the window
/// is painted, so a conversation goes on while the app is behind another window. (The panel's
/// own pass takes them in too, when it runs.)
pub fn advance(app: &mut SolveApp, ctx: &egui::Context) {
    if app.agent_chat.web_host {
        poll(app, ctx);
    }
}

/// Whether the panel hands its conversation and composer to the host's web view: the host has
/// one. The Assistant is then a panel of the app's own window and never a window of its own.
pub(crate) fn active(app: &SolveApp, ctx: &egui::Context) -> bool {
    app.agent_chat.web_host && ctx.viewport_id() == egui::ViewportId::ROOT
}

/// The corner radius of the floating panel that holds the web view: the app's largest, and where
/// the theme rounds at all, no less than the 16 points of Mastra's large surfaces.
pub(crate) fn frame_radius() -> u8 {
    match crate::theme::Tokens::get().radius_xl {
        0 => 0,
        radius => radius.max(16),
    }
}

/// The floating panel's frame around the page: rounder, and filled with the page's own surface
/// once the page has said what that is.
pub(crate) fn frame(app: &SolveApp, frame: egui::Frame) -> egui::Frame {
    frame.corner_radius(frame_radius()).fill(app.agent_chat.web_canvas.unwrap_or(frame.fill))
}

/// How wide the strip at the floating panel's inner edge is that the web view leaves free: the
/// panel is made wider or narrower by dragging it, and a native view would take the drag.
/// The colour behind the page where the host's view cannot be seen through (an X11 child
/// window): the page's own surface once the page has said it, the panel's fill until then.
pub fn backing(app: &SolveApp, ctx: &egui::Context) -> egui::Color32 {
    app.agent_chat.web_canvas.unwrap_or_else(|| ctx.global_style().visuals.panel_fill)
}
pub(crate) fn rail() -> f32 {
    crate::theme::Tokens::get().space_8
}

/// How the current selection reads, for the page to show as what the agent can see in the
/// design. Described again only when the selection or the design has changed.
fn selection(app: &mut SolveApp) -> Context {
    let doc = Arc::as_ptr(&app.session.doc) as usize;
    if let Some((selection, at, described)) = &app.agent_chat.selected_context
        && *at == doc
        && *selection == app.session.selection
    {
        return described.clone();
    }
    let described = context::describe_selection(&mut app.session);
    app.agent_chat.selected_context = Some((app.session.selection.clone(), doc, described.clone()));
    described
}

/// The panel, each frame it gives up `rect` to the web view.
pub(crate) fn place(app: &mut SolveApp, ui: &egui::Ui, rect: egui::Rect) {
    app.agent_chat.web_place = Some((rect, ui.layer_id()));
}

/// The host, once a frame after the app's UI: where the web view goes, in the window's egui
/// points, or `None` to hide it. A native view sits above everything the app draws, so it is
/// hidden while a menu, a tooltip, a dialog or another floating panel lies over its place, and
/// while the picture viewer is open.
pub fn placement(app: &mut SolveApp, ctx: &egui::Context) -> Option<egui::Rect> {
    let (rect, layer) = app.agent_chat.web_place.take()?;
    // Where the application bar is the title bar, the window's edges resize it. A native view
    // takes every press on it, so it stays off them: a panel docked to the window's edge keeps
    // that edge for resizing.
    let rect = if app.custom_titlebar { rect.intersect(crate::titlebar::inside_resize_zones(ctx)) } else { rect };
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return None;
    }
    // The picture viewer has the window and its keys (Esc, the arrows) while it is open: the
    // page gives up its place, and with it the keyboard, even where the viewer would not reach it.
    if app.agent_chat.gallery.is_some() {
        return None;
    }
    let covered = ctx.memory(|memory| {
        // A panel docked to the window's edge is under every floating layer; a floating panel
        // is under the layers that come after it.
        let mut above = layer.order == egui::Order::Background;
        let mut covered = false;
        for other in memory.layer_ids() {
            if other == layer {
                above = true;
            } else if above && memory.areas().is_visible(&other) && memory.area_rect(other.id).is_some_and(|area| area.intersects(rect)) {
                covered = true;
            }
        }
        covered
    });
    (!covered).then_some(rect)
}

fn clipped(text: &str) -> String {
    if text.chars().count() > MAX_DETAIL { text.chars().take(MAX_DETAIL).chain(std::iter::once('…')).collect() } else { text.to_string() }
}

/// What went with a message, as the page shows it: a group for each kind with its count and a
/// chip for each of its first items (each with its place among all the items), and what was
/// left out, in words.
fn context_json(context: &Context) -> Value {
    let more = |n: usize| trf("ui.agent_chat_ctx_more", &[("n", &n.to_string())]);
    let groups: Vec<Value> = context::Kind::ALL
        .into_iter()
        .filter_map(|kind| {
            let items: Vec<(usize, &context::Entry)> = context.items.iter().enumerate().filter(|(_, entry)| entry.kind == kind).collect();
            (!items.is_empty()).then(|| {
                json!({
                    "heading": format!("{} {}", tr(kind.heading()), items.len()),
                    "items": items.iter().take(panel::MAX_CHIPS).map(|(index, entry)| json!({"index": index, "label": entry.label, "detail": entry.detail})).collect::<Vec<_>>(),
                    "more": (items.len() > panel::MAX_CHIPS).then(|| more(items.len() - panel::MAX_CHIPS)),
                })
            })
        })
        .collect();
    json!({"groups": groups, "omitted": (context.omitted > 0).then(|| more(context.omitted))})
}

/// The name the page asks the host for a picture by: the bytes' length and a spread of them, so
/// one picture is not shown for another. It says nothing of where the bytes are kept.
fn picture_key(bytes: &[u8]) -> String {
    let step = (bytes.len() / 4096).max(1);
    let spread = bytes.iter().step_by(step).fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3));
    format!("{:x}-{spread:016x}", bytes.len())
}

/// The pictures the page may show now, by the name [`state`] gives each: those sent with the
/// conversation's messages, those tool calls handed back, and those attached to the next one.
/// The host serves the bytes itself; they are never part of what the page is told.
pub fn pictures(app: &SolveApp) -> Vec<(String, Arc<Vec<u8>>)> {
    let chat = &app.agent_chat;
    let sent = chat.items.iter().filter_map(|item| match item {
        Item::User { pictures, .. } => Some(pictures.iter().map(|picture| &picture.bytes)),
        _ => None,
    });
    let next = chat.attachments.iter().filter_map(|file| match &file.content {
        attach::Content::Picture { bytes, .. } => Some(bytes),
        _ => None,
    });
    let handed = chat.tool_pictures.iter().map(|(_, picture)| &picture.bytes);
    sent.flatten().chain(handed).chain(next).map(|bytes| (picture_key(bytes), Arc::clone(bytes))).collect()
}

/// The host: files were dropped on the web view. They are attached to the next message as files
/// dropped on the panel are; one that is refused says why in the conversation.
pub fn dropped(app: &mut SolveApp, paths: Vec<std::path::PathBuf>) {
    let _language = crate::i18n::scope(&app.preferences.language);
    app.agent_chat.web_drag = false;
    attach_dropped(app, paths.into_iter());
}

/// The host: files are being dragged over the web view (or no longer are), so the page says
/// what dropping them does.
pub fn dragging(app: &mut SolveApp, over: bool) {
    app.agent_chat.web_drag = over;
}

/// The permission mode the next message runs under, as the composer's menu shows it.
fn mode(app: &SolveApp) -> agents::Mode {
    let agent = if app.agent_chat.state().running() { Some(app.agent_chat.running_id.clone()) } else { agents::current(app).map(|entry| entry.id) };
    agent.as_deref().map_or(app.preferences.ai_permissions, |id| agents::mode_for(&app.preferences, id))
}

/// What the page shows, as JSON: the transcript, the composer's state, and the words in the
/// app's language for the parts of the page that are ours.
pub fn state(app: &mut SolveApp) -> String {
    // The host calls outside the app's own passes: the page's words are in the app's language.
    let _language = crate::i18n::scope(&app.preferences.language);
    let selected = selected_count(app);
    let stamp = app.session.document_stamp();
    let custom_models = custom_models_accepted(app);
    let selection = selection(app);
    // The page rounds its own corners inside the floating panel's frame, which keeps its border
    // and shadow; docked, the panel meets the window's edge and has none.
    let radius = if crate::ui_kit::Container::for_layout(app.ui.layout, crate::ui_kit::Surface::Panel).floating() {
        let tokens = crate::theme::Tokens::get();
        (f32::from(frame_radius()) - tokens.primary_border(tokens.dialog_border).width).max(0.0)
    } else {
        0.0
    };
    let chat = &app.agent_chat;
    let units = summary::Units { units: &app.session.doc.units, decimals: app.preferences.length_decimals };
    let current_doc = &app.session.doc;
    let items: Vec<Value> = chat
        .items
        .iter()
        .enumerate()
        .map(|(index, item)| match item {
            Item::User { text, image, pictures, context, files } => json!({
                "kind": "user", "index": index, "text": text, "image": image, "files": files,
                "pictures": pictures.iter().map(|picture| json!({"key": picture_key(&picture.bytes), "note": picture.note, "view": picture.view})).collect::<Vec<_>>(),
                "context": context_json(context),
            }),
            Item::Agent { thought, text } => json!({"kind": "text", "index": index, "text": text, "thought": thought, "objects": []}),
            // The names the host can select, in the order a click names them by: the page marks
            // them in the answer, and the host checks a click against this same list.
            Item::AgentRefs { text, refs } => json!({
                "kind": "text", "index": index, "text": text, "thought": false,
                "objects": refs.iter().map(|reference| reference.label.as_str()).collect::<Vec<_>>(),
            }),
            Item::Review(card) => json!({
                "kind": "review", "index": index, "review": card.id, "steps": card.words, "more": card.more,
                "made": card.made.iter().map(|b| json!({"name": b.name, "volume": b.after})).collect::<Vec<_>>(),
                "changed": card.changed.iter().map(|b| json!({"name": b.name, "before": b.before, "after": b.after})).collect::<Vec<_>>(),
                "removed": card.removed.iter().map(|b| json!({"name": b.name, "volume": b.before})).collect::<Vec<_>>(),
                "outcome": format!("{:?}", card.outcome),
            }),
            Item::Tool { id, title, kind, status, input, output } if kind == "dfm.check" => {
                let card: Value = output.as_deref().and_then(|text| serde_json::from_str(text).ok()).unwrap_or(Value::Null);
                json!({
                    "kind": "check", "id": id, "status": match status {
                        Some(backend::ToolStatus::Completed) => "done",
                        Some(backend::ToolStatus::Failed) => "error",
                        Some(backend::ToolStatus::InProgress) => "running",
                        _ => "pending",
                    },
                    "card": card,
                    "display": app.dfm_chat.display.id(),
                    "selected": selected_finding(app).map(|(run, id)| json!({"runId": run, "findingId": id})),
                })
            }
            Item::Tool { id, title, kind, status, input, output } => {
                let done = *status == Some(backend::ToolStatus::Completed);
                json!({
                    "kind": "tool", "id": id, "title": title, "toolKind": kind,
                    "status": match status {
                        Some(backend::ToolStatus::Completed) => "completed",
                        Some(backend::ToolStatus::Failed) => "failed",
                        Some(backend::ToolStatus::InProgress) => "running",
                        _ => "pending",
                    },
                    // What it did in words, as the panel's own tool rows say it.
                    "words": summary::summary(title, input.as_ref(), output.as_deref(), done, units),
                    "input": input.as_ref().map(|value| clipped(&serde_json::to_string_pretty(value).unwrap_or_default())),
                    "output": output.as_deref().map(clipped),
                    // A picture the call handed back, opened as a message's picture is (`picture`).
                    "index": index,
                    "pictures": chat.tool_pictures.iter().filter(|(call, _)| call == id).map(|(_, picture)| json!({"key": picture_key(&picture.bytes), "note": picture.note, "view": false})).collect::<Vec<_>>(),
                })
            }
            Item::Plan(steps) => {
                json!({"kind": "plan", "steps": steps.iter().map(|(text, status)| json!({"text": text, "status": status})).collect::<Vec<_>>()})
            }
            Item::Skill { title, version, document_id, revision, model, selection, support, terminal } => {
                let mut line = trf(
                    "ui.agent_chat_skill_binding",
                    &[("skill", title), ("version", version), ("document", document_id), ("revision", &revision.to_string()), ("model", model)],
                );
                if *selection > 0 {
                    line.push_str(&trf("ui.agent_chat_skill_selection", &[("n", &selection.to_string())]));
                }
                let mut warnings: Vec<&str> = Vec::new();
                if *support == skill::SkillSupport::Unverified {
                    warnings.push(tr("ui.agent_chat_skill_unverified"));
                }
                if *document_id != stamp.document_id || *revision != stamp.revision {
                    warnings.push(tr("ui.agent_chat_skill_binding_stale"));
                }
                json!({"kind": "skill", "line": line, "warnings": warnings, "terminal": terminal, "revalidate": terminal.is_some() && chat.task.is_some()})
            }
            Item::Permission { title, options, answer, .. } => json!({
                "kind": "permission", "index": index,
                "heading": trf("ui.agent_chat_permission", &[("title", title)]),
                "answered": answer.as_ref().map(|option| trf("ui.agent_chat_answered", &[("option", option)])),
                "options": options.iter().map(|option| json!({"id": option.id, "name": option.name, "kind": option.kind})).collect::<Vec<_>>(),
            }),
            Item::Approval { id, title, answer, .. } => json!({"kind": "approval", "id": id, "title": title, "answer": answer}),
            Item::Changes { steps, doc, undone } => {
                json!({
                    "kind": "changes", "index": index, "label": trf("ui.agent_chat_changes", &[("n", &steps.to_string())]),
                    "undone": undone, "undoable": !undone && Arc::ptr_eq(doc, current_doc),
                })
            }
            Item::Notice { text, error } => json!({"kind": "notice", "text": text, "error": error}),
            // A notice to a page that reads no more of it; `problem` says which way out to offer.
            Item::Problem { kind, prefix, name } => json!({
                "kind": "notice", "text": kind.words(name), "error": true,
                "problem": {"kind": kind.id(), "prefix": prefix, "name": name},
            }),
        })
        .collect();
    let turn = chat.turn.as_ref().map(|turn| {
        let n = turn.changes.to_string();
        match app.preferences.agent_chat_turn_limit {
            0 => trf("ui.agent_chat_changes_n", &[("n", &n)]),
            limit => trf("ui.agent_chat_changes_of", &[("n", &n), ("limit", &limit.to_string())]),
        }
    });
    let dfm = json!({
        "display": app.dfm_chat.display.id(),
        "selected": selected_finding(app).map(|(run, id)| json!({"runId": run, "findingId": id})),
    });
    let state = chat.state();
    let entries = agents::entries(app);
    let current = agents::current(app);
    // An advertised (even empty) option list replaces the agent's older mode and model pickers.
    let modern = chat.config_options.is_some();
    let models: Vec<Value> = if !modern && models_shown(app) {
        grouped_models(app).into_iter().map(|(model, name, group, tip)| json!({"id": model.id, "name": name, "group": group, "tip": tip})).collect()
    } else {
        Vec::new()
    };
    let mut notes: Vec<String> = Vec::new();
    if let Some(warning) = settings::model_warning(app) {
        notes.push(warning);
    }
    // A model list that could not be had, or a typed model the agent refused.
    if let Some(note) = &chat.models_note {
        notes.push(note.clone());
    }
    if chat.mastra {
        if let Some(runtime) = chat.runtime {
            notes.push(runtime.label().to_string());
        }
        notes.push(
            tr(match chat.connection_auth {
                Some(true) => "ui.mastra_connection_ready",
                Some(false) => "ui.mastra_connection_signed_out",
                None => "ui.mastra_connection_unknown",
            })
            .to_string(),
        );
    }
    if let Some(status) = &chat.auth_status {
        notes.push(status.clone());
    }
    // The agent's own sign-in, when it asks for one: how each method is done.
    let auth = (state == State::NeedsAuth).then(|| {
        let base = chat.argv.iter().map(|arg| panel::quote(arg)).collect::<Vec<_>>().join(" ");
        chat.auth
            .iter()
            .map(|method| {
                let description = method.description.trim();
                let label = if description.is_empty() || description == method.name { method.name.clone() } else { format!("{} — {description}", method.name) };
                match &method.kind {
                    acp::AuthKind::Agent => json!({"id": method.id, "label": label, "kind": "agent", "action": trf("ui.agent_chat_auth_agent", &[("name", &method.name)])}),
                    acp::AuthKind::Terminal { args, .. } => {
                        let command = format!("{base} {}", args.iter().map(|arg| panel::quote(arg)).collect::<Vec<_>>().join(" "));
                        json!({"id": method.id, "label": label, "kind": "terminal", "command": command})
                    }
                    acp::AuthKind::EnvVar { vars } => json!({"id": method.id, "label": label, "kind": "env", "note": trf("ui.agent_chat_auth_env", &[("vars", &vars.join(", "))])}),
                }
            })
            .collect::<Vec<_>>()
    });
    // The known agents not found. A basic one is listed by the page as not ready, with `body`.
    // Beside the built-in agent there is nothing to install, whatever was found before.
    let builtin = agents::is_builtin(app);
    let missing: Vec<Value> = chat
        .detection
        .as_ref()
        .filter(|_| !builtin)
        .map(|found| {
            let listed = |agent: &super::discover::Missing| {
                let body = agent.basic.then(|| trf("ui.agent_chat_install_body", &[("agent", agent.name)]));
                json!({"id": agent.id, "name": agent.name, "install": agent.install, "basic": agent.basic, "body": body})
            };
            found.missing.iter().map(listed).collect()
        })
        .unwrap_or_default();
    let strings: serde_json::Map<String, Value> = [
        ("placeholder", "ui.agent_chat_placeholder"),
        ("placeholderBusy", "ui.agent_chat_placeholder_busy"),
        ("send", "ui.agent_chat_send"),
        ("stop", "ui.agent_chat_stop_turn_tip"),
        ("working", "ui.agent_chat_state_busy"),
        ("ready", "ui.agent_chat_state_ready"),
        ("off", "ui.agent_chat_state_off"),
        ("starting", "ui.agent_chat_state_starting"),
        ("auth", "ui.agent_chat_state_auth"),
        ("failed", "ui.agent_chat_state_failed"),
        ("exited", "ui.agent_chat_state_exited"),
        ("undone", "ui.agent_chat_undone"),
        ("undo", "ui.agent_chat_undo_changes"),
        ("allowOnce", "ui.agent_chat_allow_once"),
        ("alwaysAllow", "ui.ai_always_allow"),
        ("deny", "ui.agent_chat_deny"),
        ("allowed", "ui.agent_chat_allowed"),
        ("denied", "ui.agent_chat_denied"),
        ("approval", "ui.agent_chat_approval"),
        ("title", "ui.assistant"),
        ("add", "ui.agent_chat_add_tip"),
        ("addFile", "ui.agent_chat_add_file"),
        ("pastePicture", "ui.agent_chat_paste_picture"),
        ("attachView", "ui.agent_chat_attach_view"),
        ("attachSelection", "ui.agent_chat_attach_selection"),
        ("pickContext", "ui.agent_chat_pick_context"),
        ("modeAsk", "ui.agent_chat_mode_ask"),
        ("modeEdits", "ui.agent_chat_mode_edits"),
        ("modeAll", "ui.agent_chat_mode_all"),
        ("queued", "ui.agent_chat_queued"),
        ("viewAttached", "ui.agent_chat_view_attached"),
        ("waiting", "ui.agent_chat_waiting"),
        ("connect", "ui.agent_chat_connect"),
        ("disconnect", "ui.agent_chat_stop_tip"),
        ("retry", "ui.agent_chat_retry"),
        ("newChat", "ui.agent_chat_new"),
        ("newChatConfirm", "ui.agent_chat_new_confirm"),
        ("cancel", "ui.agent_chat_cancel"),
        ("close", "ui.agent_chat_close"),
        ("minimize", "ui.agent_chat_minimize"),
        ("dockLeft", "ui.agent_chat_dock_left"),
        ("dockRight", "ui.agent_chat_dock_right"),
        ("addAgent", "ui.ai_add_agent"),
        ("settings", "ui.ai_settings"),
        ("switchAgent", "ui.ai_switch_tip"),
        ("detecting", "ui.agent_chat_detecting"),
        ("noneFound", "ui.agent_chat_none_found_short"),
        ("notInstalled", "ui.agent_chat_not_installed"),
        ("notReady", "ui.agent_chat_not_ready"),
        ("notSetUp", "ui.agent_chat_not_set_up"),
        ("refresh", "ui.agent_chat_refresh"),
        ("authTitle", "ui.agent_chat_auth_title"),
        ("authNeeded", "ui.agent_chat_auth_needed"),
        ("authNone", "ui.agent_chat_auth_none"),
        ("authTerminal", "ui.agent_chat_auth_terminal"),
        ("authRetry", "ui.agent_chat_auth_retry"),
        ("setupTitle", "ui.agent_chat_setup_title"),
        ("setupBody", "ui.agent_chat_setup_body"),
        ("openSettings", "ui.agent_chat_open_settings"),
        ("model", "ui.ai_model"),
        ("queuedStopped", "ui.agent_chat_queued_stopped"),
        ("sendNow", "ui.agent_chat_send_now"),
        ("sendNowTip", "ui.agent_chat_send_now_tip"),
        ("edit", "ui.agent_chat_edit"),
        ("remove", "ui.agent_chat_remove"),
        ("save", "ui.agent_chat_save"),
        ("modelSearch", "ui.agent_chat_model_search"),
        ("modelCustom", "ui.agent_chat_model_custom"),
        ("modelNoMatch", "ui.agent_chat_model_no_match"),
        ("revalidate", "ui.agent_chat_task_revalidate"),
        ("dropHint", "ui.agent_chat_drop_hint"),
        ("removeChip", "ui.agent_chat_remove_chip"),
        ("openPicture", "ui.agent_chat_open_picture"),
        ("selectedNow", "ui.agent_chat_web_selected"),
        ("messageBox", "ui.agent_chat_web_message"),
        ("composer", "ui.agent_chat_web_composer"),
        ("withNext", "ui.agent_chat_web_with_next"),
        ("latest", "ui.agent_chat_web_latest"),
        ("turnLabel", "ui.agent_chat_web_turn"),
        ("reviewTitle", "ui.agent_chat_review_title"),
        ("reviewAccept", "ui.agent_chat_review_accept"),
        ("reviewReject", "ui.agent_chat_review_reject"),
        ("reviewMade", "ui.agent_chat_review_made"),
        ("reviewChanged", "ui.agent_chat_review_changed"),
        ("reviewRemoved", "ui.agent_chat_review_removed"),
        ("reviewAccepted", "ui.agent_chat_review_accepted"),
        ("reviewRejected", "ui.agent_chat_review_rejected"),
        ("reviewMore", "ui.agent_chat_review_more_steps"),
        ("dfmCheckTitle", "ui.dfm_check.title"),
        ("dfmRunning", "ui.dfm_check.running"),
        ("dfmDone", "ui.dfm_check.done"),
        ("dfmCancelled", "ui.dfm_check.cancelled"),
        ("dfmError", "ui.dfm_check.error"),
        ("dfmNotChecked", "ui.dfm_check.not_checked"),
        ("dfmRefresh", "ui.agent_chat_refresh"),
        ("dfmStale", "ui.dfm_check.stale"),
        ("dfmFindings", "ui.dfm_check.findings"),
        ("dfmAdvisory", "ui.dfm_check.advisory"),
        ("dfmFailed", "ui.dfm_check.failed"),
        ("dfmChecked", "ui.dfm_check.checked"),
        ("dfmNotCovered", "ui.dfm_check.not_covered"),
        ("dfmMeasured", "ui.dfm_check.measured"),
        ("dfmRequirement", "ui.dfm_check.requirement"),
        ("dfmSource", "ui.dfm_check.source"),
        ("dfmOverride", "ui.dfm_check.override"),
        ("dfmAll", "ui.all"),
        ("dfmSelected", "ui.dfm_check.selected"),
        ("dfmOff", "ui.dfm_check.off"),
        ("dfmSelect", "ui.dfm_check.selected"),
        // The built-in agent's connections (`connect`, below): its page, the sign-ins' own
        // names, and the sentences of a turn its connection refused.
        ("connTitle", "ui.agent_chat_conn_title"),
        ("connNoneTitle", "ui.agent_chat_conn_none_title"),
        ("connBack", "ui.agent_chat_conn_back"),
        ("connPlans", "ui.agent_chat_conn_plans"),
        ("connKeys", "ui.agent_chat_conn_keys"),
        ("connCustom", "ui.agent_chat_conn_custom"),
        ("connSignedIn", "ui.agent_chat_conn_signed_in"),
        ("connExpired", "ui.agent_chat_conn_expired"),
        ("connSigningIn", "ui.agent_chat_conn_signing_in"),
        ("connSignIn", "ui.agent_chat_conn_sign_in"),
        ("connSignInAgain", "ui.agent_chat_conn_sign_in_again"),
        ("connSignOut", "ui.agent_chat_conn_sign_out"),
        ("connUse", "ui.agent_chat_conn_use"),
        ("connInUse", "ui.agent_chat_conn_in_use"),
        ("connShadowed", "ui.agent_chat_conn_shadowed"),
        ("connKeySaved", "ui.agent_chat_conn_key_saved"),
        ("connKeyAdd", "ui.agent_chat_conn_key_add"),
        ("connKeyUpdate", "ui.agent_chat_conn_key_update"),
        ("connRemove", "ui.agent_chat_conn_remove"),
        ("connKeyTitle", "ui.agent_chat_conn_key_title"),
        ("connKeyBody", "ui.agent_chat_conn_key_body"),
        ("connKeyPlaceholder", "ui.agent_chat_conn_key_placeholder"),
        ("connSave", "ui.agent_chat_conn_save"),
        ("connCustomAdd", "ui.agent_chat_conn_custom_add"),
        ("connCustomName", "ui.agent_chat_conn_custom_name"),
        ("connCustomUrl", "ui.agent_chat_conn_custom_url"),
        ("connCustomKey", "ui.agent_chat_conn_custom_key"),
        ("connCustomModels", "ui.agent_chat_conn_custom_models"),
        ("connCustomNeedBoth", "ui.agent_chat_conn_custom_need_both"),
        ("connCustomNeedModel", "ui.agent_chat_conn_custom_need_model"),
        ("connCustomTooMany", "ui.agent_chat_conn_custom_too_many"),
        ("connCustomPlainUrl", "ui.agent_chat_conn_custom_plain_url"),
        ("connCustomExists", "ui.agent_chat_conn_custom_exists"),
        ("connAdd", "ui.agent_chat_conn_add"),
        ("connWaiting", "ui.agent_chat_conn_waiting"),
        ("connStarting", "ui.agent_chat_conn_starting"),
        ("connReopen", "ui.agent_chat_conn_reopen"),
        ("connCodeHint", "ui.agent_chat_conn_code_hint"),
        ("connCodePlaceholder", "ui.agent_chat_conn_code_placeholder"),
        ("connCodeIncomplete", "ui.agent_chat_conn_code_incomplete"),
        ("connCodeDone", "ui.agent_chat_conn_code_done"),
        ("connDeviceHint", "ui.agent_chat_conn_device_hint"),
        ("connProblemPort", "ui.agent_chat_conn_problem_port"),
        ("connProblemOffline", "ui.agent_chat_conn_problem_offline"),
        ("connProblemFailed", "ui.agent_chat_conn_problem_failed"),
        ("connButton", "ui.agent_chat_conn_button"),
        ("connButtonNone", "ui.agent_chat_conn_button_none"),
        ("connButtonTip", "ui.agent_chat_conn_button_tip"),
        ("planOpenaiCodex", "ui.agent_chat_plan_openai_codex"),
        ("planAnthropic", "ui.agent_chat_plan_anthropic"),
        ("planKimiForCoding", "ui.agent_chat_plan_kimi_for_coding"),
        ("planXai", "ui.agent_chat_plan_xai"),
        ("planGithubCopilot", "ui.agent_chat_plan_github_copilot"),
        ("problemAuth", "ui.agent_chat_problem_auth"),
        ("problemKey", "ui.agent_chat_problem_key"),
        ("problemQuota", "ui.agent_chat_problem_quota"),
        ("problemRate", "ui.agent_chat_problem_rate"),
        ("problemNetwork", "ui.agent_chat_problem_network"),
        ("problemModel", "ui.agent_chat_problem_model"),
        // The levels of an agent's reasoning effort, by the value it names them with.
        ("effort", "ui.agent_chat_effort"),
        ("effortOff", "ui.agent_chat_effort_off"),
        ("effortLow", "ui.agent_chat_effort_low"),
        ("effortMedium", "ui.agent_chat_effort_medium"),
        ("effortHigh", "ui.agent_chat_effort_high"),
        ("effortXhigh", "ui.agent_chat_effort_xhigh"),
        ("effortMax", "ui.agent_chat_effort_max"),
    ]
    .into_iter()
    .map(|(name, key)| (name.to_string(), Value::from(tr(key))))
    .collect();
    json!({
        "items": items,
        "state": state.id(),
        "waiting": chat.waiting(),
        "stopped": chat.stopped,
        "pending": chat.pending.iter().map(|message| json!({
            "text": message.text, "image": message.image, "context": context_json(&message.context),
            "files": message.files.iter().map(|file| file.name.as_str()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "attachments": chat.attachments.iter().map(|file| json!({
            "name": file.name,
            "picture": match &file.content {
                attach::Content::Picture { bytes, .. } => Some(picture_key(bytes)),
                _ => None,
            },
            // A DFM report is not a file: the page draws it with its own mark and says which run.
            "report": file.report_run(),
        })).collect::<Vec<_>>(),
        "loading": chat.loading.iter().map(|file| file.name.clone()).collect::<Vec<_>>(),
        "picked": context_json(&chat.picked_context),
        "dragging": chat.web_drag,
        "attachView": chat.attach_view,
        "attachSelection": app.preferences.agent_chat_attach_selection,
        "images": chat.images || !state.running(),
        "selected": selected,
        "selection": context_json(&selection),
        "radius": radius,
        "permissions": mode(app).id(),
        "skill": chat.skill.id(),
        "skills": skill::Skill::ALL.iter().map(|skill| json!({"id": skill.id(), "title": skill.title()})).collect::<Vec<_>>(),
        "turn": turn,
        // `hint`: what is known of an installed agent without starting it, in a word beside its name.
        "agents": entries
            .iter()
            .map(|entry| json!({"id": entry.id, "name": entry.name, "detail": entry.detail(), "hint": entry.status().map(tr)}))
            .collect::<Vec<_>>(),
        "agent": current.as_ref().map(|entry| entry.id.clone()),
        "detecting": chat.detecting.is_some(),
        "running": state.running(),
        "modes": if modern { Vec::new() } else { chat.modes.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>() },
        "mode": chat.mode,
        "models": models,
        "model": chat.model,
        "customModels": custom_models,
        "config": chat.config_options,
        "configPending": chat.config_pending,
        "notes": notes,
        "auth": auth,
        // The built-in agent's connections: an object exactly while it is the agent, so the
        // page shows its Connections page then and is as it was otherwise.
        "connect": connections::told_of(app),
        "missing": missing,
        "confirmNew": chat.confirm_new,
        "left": app.preferences.agent_chat_side == "left",
        "dark": crate::theme::is_dark(),
        "strings": strings,
        "dfm": dfm,
    })
    .to_string()
}

/// One message from the page, as JSON `{"type": …}`. Only these are acted on, each checked
/// against the chat as it is now. The conversation: `send` (`text`; `now` also sends what is
/// queued, stopping the running step), `stop`, `approve` (`id`, `allow`: true, false or
/// `"always"`), `permission` (`index`, `option`), `undo` (`index`), `new_chat`, `confirm_new`
/// (`yes`), `object` (`index`, `ref`, `label`: select the object an answer names), `picture`
/// (`index`, `at`: open a message's picture in the viewer), `revalidate`. The queue: `send_now`,
/// `remove_pending` (`index`, `text`), `edit_pending` (`index`, `was`, `text`); a queued message
/// has no name of its own, so the page says which text it means and a queue that has moved on
/// refuses. The composer: `permissions` (`mode`), `attach_view` / `attach_selection` (`on`),
/// `pick_context`, `remove_picked` (`index`, `label`), `add_file`, `paste_picture`, `remove_file`
/// (`index`), `skill` (`id`). The agent: `agent` (`id`, one of the listed agents), `connect`,
/// `disconnect`, `agent_mode` (`id`), `model` (`id`), `custom_model` (`text`), `config` (`id`,
/// `value`), `auth` (`id`), `refresh`, `add_agent`, `settings`. The built-in agent's connections
/// ([`connections::act`]): `conn_login` (`id`), `conn_code` (`id`, `code`), `conn_reopen`,
/// `conn_cancel`, `conn_logout` (`id`), `conn_key` (`provider`, `key`), `conn_key_remove`
/// (`provider`), `conn_custom` (`name`, `url`, `key`, `models`), `conn_custom_remove` (`id`),
/// `conn_retry`. The panel: `close`, `minimize`,
/// `side` (`left`); the page cannot move the Assistant to a window of its own. And `surface`
/// (`color`, `#rrggbb`: the page's own surface, for the frame around it), `copy` (`text`) and
/// `open_link` (`url`, web addresses only). Anything else is refused with the reason.
pub fn act(app: &mut SolveApp, ctx: &egui::Context, message: &str) -> std::result::Result<(), String> {
    let _language = crate::i18n::scope(&app.preferences.language);
    if message.len() > MAX_MESSAGE * 4 {
        return Err("message too long".into());
    }
    let action: Value = serde_json::from_str(message).map_err(|error| format!("not JSON: {error}"))?;
    let text = |name: &str| action.get(name).and_then(Value::as_str);
    let index = |name: &str| action.get(name).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
    match text("type").ok_or("no type")? {
        "dfm_check" => run_check(app),
        "dfm_poll" => poll_check(app),
        "finding" => {
            let run = text("runId").ok_or("finding: no runId")?;
            let id = text("id").ok_or("finding: no id")?;
            crate::dfm_chat::select(app, run, id)?;
            // The list and the model address the same run-scoped finding.
            app.viewport.select_dfm_finding(Some(id));
        }
        "dfm_display" => {
            let mode = text("mode").ok_or("dfm_display: no mode")?;
            let display = crate::dfm_chat::Display::from_id(mode).ok_or("dfm_display: mode must be all, selected or off")?;
            app.dfm_chat.display = display;
            app.viewport.set_dfm_display(match display {
                crate::dfm_chat::Display::All => crate::dfm_overlay::DfmDisplay::All,
                crate::dfm_chat::Display::Selected => crate::dfm_overlay::DfmDisplay::Selected,
                crate::dfm_chat::Display::Off => crate::dfm_overlay::DfmDisplay::Off,
            });
        }
        "send" => {
            let body = text("text").ok_or("send: no text")?;
            if body.chars().count() > MAX_MESSAGE {
                return Err("send: text too long".into());
            }
            // A message waits for files still being read, and files alone make a message.
            if !app.agent_chat.loading.is_empty() {
                return Err("send: files are still being read".into());
            }
            if body.trim().is_empty() && app.agent_chat.attachments.is_empty() {
                return Err("send: nothing to send".into());
            }
            // A check asked for in words runs the button's command, not a model turn.
            if crate::dfm_chat::is_check_request(body) {
                run_check(app);
                return Ok(());
            }
            send(app, ctx, body.trim_end_matches('\n').to_string());
            // Ctrl+Enter: what is queued goes now, this message with it.
            if action.get("now").and_then(Value::as_bool) == Some(true) {
                send_now(app, ctx);
            }
        }
        "send_now" => {
            if app.agent_chat.pending.is_empty() {
                return Err("send_now: nothing is queued".into());
            }
            send_now(app, ctx);
        }
        "remove_pending" => {
            let at = index("index").ok_or("remove_pending: no index")?;
            let named = text("text").ok_or("remove_pending: no text")?;
            if app.agent_chat.pending.get(at).is_none_or(|message| message.text != named) {
                return Err("remove_pending: the queue has changed".into());
            }
            remove_pending(app, at);
        }
        "edit_pending" => {
            let at = index("index").ok_or("edit_pending: no index")?;
            let (was, body) = (text("was").ok_or("edit_pending: no was")?, text("text").ok_or("edit_pending: no text")?);
            if body.trim().is_empty() || body.chars().count() > MAX_MESSAGE {
                return Err("edit_pending: a queued message is neither empty nor that long".into());
            }
            match app.agent_chat.pending.get_mut(at) {
                Some(message) if message.text == was => message.text = body.to_string(),
                _ => return Err("edit_pending: the queue has changed".into()),
            }
        }
        "object" => {
            let at = index("index").ok_or("object: no index")?;
            let (place, label) = (index("ref").ok_or("object: no ref")?, text("label").ok_or("object: no label")?);
            // Only a name the host listed for that answer, by the place it listed it at.
            let reference = match app.agent_chat.items.get(at) {
                Some(Item::AgentRefs { refs, .. }) => refs.get(place).filter(|reference| reference.label == label).cloned(),
                _ => None,
            };
            let reference = reference.ok_or("object: not an object that answer names")?;
            match object_ref::resolve(&mut app.session, &reference) {
                Ok(selection) => crate::viewport::select(app, Some(selection), false, false),
                Err(reason) => app.agent_chat.notice(reason, true),
            }
        }
        "picture" => {
            let (at, shown) = (index("index").ok_or("picture: no index")?, index("at").ok_or("picture: no at")?);
            let pictures = match app.agent_chat.items.get(at) {
                Some(Item::User { pictures, .. }) if shown < pictures.len() => pictures.clone(),
                // A tool call's one picture.
                Some(Item::Tool { id, .. }) if shown == 0 && app.agent_chat.tool_pictures.iter().any(|(call, _)| call == id) => {
                    app.agent_chat.tool_pictures.iter().filter(|(call, _)| call == id).map(|(_, picture)| picture.clone()).collect()
                }
                _ => return Err("picture: that message has no such picture".into()),
            };
            app.agent_chat.gallery = Some(Gallery { pictures, at: shown });
        }
        "revalidate" => {
            let offered =
                app.agent_chat.task.is_some() && app.agent_chat.items.iter().any(|item| matches!(item, Item::Skill { terminal: Some(_), .. }));
            if !offered {
                return Err("revalidate: no finished task to check again".into());
            }
            if let Err(error) = task_runtime::revalidate(app) {
                app.agent_chat.notice(error, true);
            }
        }
        "stop" => cancel(app),
        "approve" => {
            let id = action.get("id").and_then(Value::as_u64).ok_or("approve: no id")?;
            if !app.agent_chat.held.iter().any(|held| held.id == id) {
                return Err("approve: that request is no longer waiting".into());
            }
            match action.get("allow") {
                Some(Value::Bool(allow)) => approve(app, id, *allow),
                Some(Value::String(always)) if always == "always" => approve_always(app, id),
                _ => return Err("approve: allow must be true, false or \"always\"".into()),
            }
        }
        "review" => {
            let id = action.get("id").and_then(Value::as_u64).ok_or("review: no id")?;
            match action.get("accept").and_then(Value::as_bool) {
                Some(true) => super::review::accept(app, id)?,
                Some(false) => super::review::reject(app, id),
                None => return Err("review: accept must be true or false".into()),
            }
        }
        "permission" => {
            let at = index("index").ok_or("permission: no index")?;
            let option = text("option").ok_or("permission: no option")?;
            let offered = matches!(app.agent_chat.items.get(at), Some(Item::Permission { answer: None, options, .. }) if options.iter().any(|o| o.id == option));
            if !offered {
                return Err("permission: that request is not waiting for this answer".into());
            }
            answer_permission(app, at, Some(option));
        }
        "undo" => {
            let at = index("index").ok_or("undo: no index")?;
            let undoable =
                matches!(app.agent_chat.items.get(at), Some(Item::Changes { undone: false, doc, .. }) if Arc::ptr_eq(doc, &app.session.doc));
            if !undoable {
                return Err("undo: those changes can no longer be undone as one step".into());
            }
            undo_changes(app, at);
        }
        "permissions" => {
            let wanted = text("mode").and_then(agents::Mode::parse).ok_or("permissions: mode must be ask, edits or all")?;
            let agent =
                if app.agent_chat.state().running() { Some(app.agent_chat.running_id.clone()) } else { agents::current(app).map(|entry| entry.id) };
            match agent {
                Some(id) => {
                    app.preferences.agent_permissions.insert(id, wanted);
                }
                None => app.preferences.ai_permissions = wanted,
            }
        }
        "attach_view" => app.agent_chat.attach_view = action.get("on").and_then(Value::as_bool).ok_or("attach_view: no on")?,
        "attach_selection" => {
            app.preferences.agent_chat_attach_selection = action.get("on").and_then(Value::as_bool).ok_or("attach_selection: no on")?
        }
        "pick_context" => {
            if app.dialog.is_some() {
                return Err("pick_context: another dialog is open".into());
            }
            start_pick(app);
        }
        "remove_picked" => {
            let at = index("index").ok_or("remove_picked: no index")?;
            let label = text("label").ok_or("remove_picked: no label")?;
            if app.agent_chat.picked_context.items.get(at).is_none_or(|entry| entry.label != label) {
                return Err("remove_picked: the picked things have changed".into());
            }
            remove_picked(app, at);
        }
        "add_file" => {
            app.pick_file(tr("ui.agent_chat_attachable"), &attach::extensions(), |app, path| {
                if let Some(path) = path
                    && let Err(error) = attach_path(app, &path)
                {
                    app.agent_chat.notice(error, true);
                }
            });
        }
        "paste_picture" => {
            if let Err(error) = paste_picture(app) {
                app.agent_chat.notice(error, true);
            }
        }
        "remove_file" => {
            let at = index("index").filter(|at| *at < app.agent_chat.attachments.len()).ok_or("remove_file: no such file")?;
            app.agent_chat.attachments.remove(at);
        }
        "skill" => {
            if app.agent_chat.state() == State::Busy {
                return Err("skill: the running turn keeps its skill".into());
            }
            let id = text("id").ok_or("skill: no id")?;
            app.agent_chat.skill = skill::Skill::ALL.into_iter().find(|skill| skill.id() == id).ok_or("skill: unknown")?;
        }
        "new_chat" => panel::request_new_chat(app),
        "confirm_new" => {
            let yes = action.get("yes").and_then(Value::as_bool).ok_or("confirm_new: no yes")?;
            if !app.agent_chat.confirm_new {
                return Err("confirm_new: nothing was asked".into());
            }
            if yes {
                new_chat(app);
            }
            app.agent_chat.confirm_new = false;
        }
        "agent" => {
            let id = text("id").ok_or("agent: no id")?;
            if !agents::entries(app).iter().any(|entry| entry.id == id) {
                return Err("agent: not one of the listed agents".into());
            }
            // As the panel's own switcher: another agent ends the running one.
            if agents::current(app).is_none_or(|current| current.id != id) {
                disconnect(app);
                app.agent_chat.agent_id = Some(id.to_string());
            }
        }
        "connect" => {
            if !app.agent_chat.state().running() {
                connect(app, ctx);
            }
        }
        "disconnect" => disconnect(app),
        "agent_mode" => {
            let id = text("id").ok_or("agent_mode: no id")?;
            if !app.agent_chat.modes.iter().any(|(mode, _)| mode == id) {
                return Err("agent_mode: not one of the agent's modes".into());
            }
            set_mode(app, id);
        }
        "model" => {
            let id = text("id").ok_or("model: no id")?;
            if !grouped_models(app).iter().any(|(model, _, _, _)| model.id == id) {
                return Err("model: not one of the listed models".into());
            }
            set_model(app, id);
        }
        "custom_model" => {
            let typed = text("text").ok_or("custom_model: no text")?;
            if !set_custom_model(app, typed) {
                return Err("custom_model: this agent takes no such model now".into());
            }
        }
        "config" => {
            let (id, value) = (text("id").ok_or("config: no id")?, text("value").ok_or("config: no value")?);
            let listed = app
                .agent_chat
                .config_options
                .as_ref()
                .is_some_and(|options| options.iter().any(|option| option.id == id && option.options.iter().any(|choice| choice.value == value)));
            if !listed {
                return Err("config: not a listed choice".into());
            }
            set_config_option(app, id, value);
        }
        "auth" => {
            let id = text("id").ok_or("auth: no id")?;
            let offered = app.agent_chat.state() == State::NeedsAuth
                && app.agent_chat.auth.iter().any(|method| method.id == id && method.kind == acp::AuthKind::Agent);
            if !offered {
                return Err("auth: the agent does not offer that sign-in now".into());
            }
            app.agent_chat.authenticate(id);
        }
        kind if connections::ACTIONS.contains(&kind) => connections::act(app, ctx, kind, &action)?,
        "refresh" => refresh(app, ctx),
        "add_agent" => settings::open_add(app),
        "settings" => crate::prefs::open_at(app, crate::prefs::PAGE_ASSISTANT),
        "close" => app.agent_chat.open = false,
        "minimize" => app.preferences.agent_chat_minimized = true,
        "side" => {
            let left = action.get("left").and_then(Value::as_bool).ok_or("side: no left")?;
            app.preferences.agent_chat_side = if left { "left" } else { "right" }.into();
        }
        "surface" => {
            let hex = text("color").and_then(|colour| colour.strip_prefix('#')).filter(|hex| hex.len() == 6);
            let rgb = hex.and_then(|hex| u32::from_str_radix(hex, 16).ok()).ok_or("surface: color must be #rrggbb")?;
            let [_, r, g, b] = rgb.to_be_bytes();
            app.agent_chat.web_canvas = Some(egui::Color32::from_rgb(r, g, b));
        }
        "copy" => {
            let body = text("text").ok_or("copy: no text")?;
            ctx.copy_text(body.chars().take(MAX_MESSAGE).collect());
        }
        "open_link" => {
            let url = text("url").ok_or("open_link: no url")?;
            if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 2048 || url.chars().any(char::is_control) {
                return Err("open_link: only web addresses open".into());
            }
            ctx.open_url(egui::OpenUrl::new_tab(url));
        }
        other => return Err(format!("unknown action {other}")),
    }
    ctx.request_repaint();
    Ok(())
}

/// Run `dfm.check` for the button and the chat phrase, and keep one Tool row per run so the page
/// can show running, done or error.
fn run_check(app: &mut SolveApp) {
    let n = app.dfm_chat.next;
    app.dfm_chat.next = app.dfm_chat.next.saturating_add(1);
    let id = format!("dfm-check-{n}");
    app.agent_chat.push(Item::Tool {
        id: id.clone(),
        title: tr("ui.dfm_check.title").to_string(),
        kind: "dfm.check".into(),
        status: Some(backend::ToolStatus::InProgress),
        input: None,
        output: None,
    });
    let card = crate::dfm_chat::run(app);
    let failed = card.status == "error";
    set_check_item(app, &id, card);
    if !failed {
        // The run may already be finished (a small sheet); ask once, then the page polls.
        poll_check(app);
    }
    app.agent_chat.notice(tr("ui.dfm_check.title"), failed);
}

/// Ask `inspect.dfm` for the live report and update the newest check row. The page calls this
/// while the row says running.
fn poll_check(app: &mut SolveApp) {
    let Some(id) = app.agent_chat.items.iter().rev().find_map(|item| match item {
        Item::Tool { id, kind, .. } if kind == "dfm.check" => Some(id.clone()),
        _ => None,
    }) else {
        return;
    };
    let card = crate::dfm_chat::refresh(app);
    set_check_item(app, &id, card);
}

/// While a check row says running, refresh it from the saved report on each frame; the page's
/// poll action is not the only driver (a host-sent shortcut starts the row itself).
pub(crate) fn poll_running_checks(app: &mut SolveApp) {
    let running = app
        .agent_chat
        .items
        .iter()
        .rev()
        .any(|item| matches!(item, Item::Tool { kind, status: Some(backend::ToolStatus::InProgress), .. } if kind == "dfm.check"));
    if running {
        poll_check(app);
    }
}

/// A model tool call to `dfm.check` arrives as a tool row. Recognise its reply (project id and
/// sheet count), make it a check row and fill it from the saved report. An agent may reach the
/// command through a direct tool (no command kind) or through the generic Code Mode `execute`
/// tool (kind `execute`); both carry the same reply.
pub(crate) fn adopt_model_check(app: &mut SolveApp) {
    let found = app.agent_chat.items.iter().rev().find_map(|item| match item {
        Item::Tool { id, output: Some(output), .. } => {
            let value: Value = serde_json::from_str(output).ok()?;
            (value.get("projectId").is_some() && value.get("sheets").is_some()).then(|| id.clone())
        }
        _ => None,
    });
    let Some(id) = found else { return };
    for item in app.agent_chat.items.iter_mut().rev() {
        if let Item::Tool { id: shown, kind, status, .. } = item
            && *shown == id
        {
            *kind = "dfm.check".into();
            *status = Some(backend::ToolStatus::Completed);
            break;
        }
    }
    poll_check(app);
}

/// Give the viewport the same report the card shows, so a row and a mark address one finding.
/// The selected finding, whichever side chose it: the page row or a viewport label. The viewport
/// is the host's state, so a label click updates the list too.
fn selected_finding(app: &SolveApp) -> Option<(String, String)> {
    app.viewport.dfm.selected_key().map(|(run, id)| (run.to_string(), id.to_string())).or_else(|| app.dfm_chat.selected.clone())
}

fn sync_dfm_marks(app: &mut SolveApp, card: &crate::dfm_chat::Card) {
    crate::dfm_chat::show_marks(app, card);
}

fn set_check_item(app: &mut SolveApp, id: &str, card: crate::dfm_chat::Card) {
    let status = match card.status.as_str() {
        "running" => backend::ToolStatus::InProgress,
        "error" => backend::ToolStatus::Failed,
        _ => backend::ToolStatus::Completed,
    };
    let output = serde_json::to_string(&crate::dfm_chat::page(&card)).unwrap_or_default();
    sync_dfm_marks(app, &card);
    app.dfm_chat.card = Some(card);
    for item in app.agent_chat.items.iter_mut().rev() {
        if let Item::Tool { id: shown, status: shown_status, output: shown_output, .. } = item
            && *shown == id
        {
            *shown_status = Some(status);
            *shown_output = Some(output.clone());
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `inspect.dfm` reply: a finished current run with its CONTRACT §2 findings object.
    fn report() -> Value {
        json!({
            "available": true, "running": false, "runId": "run-1", "outcome": "current",
            "modelVersion": "model-1", "candidateDigest": "candidate-1", "configDigest": "config-1",
            "freshness": "staleModel", "summary": "2 findings", "sourceRevision": "r1",
            "findings": {
                "schemaVersion": 2, "runId": "run-1", "candidateDigest": "candidate-1",
                "profileDigest": "profile-1", "configDigest": "config-1", "modelVersion": "model-1",
                "projectId": "project-1", "source": {"kind": "native"},
                "findings": [{
                    "id": "sheet-1/F2", "sheet": 1, "body": "Plate", "from": "rules", "rules": ["sm.hole_min_diameter"],
                    "state": "advisory", "status": "needs_review", "code": null,
                    "measured": {"value": 2.4, "unit": "mm"},
                    "requirement": {"value": 3.0, "unit": "mm", "layer": "factory", "confirmed": "factory_confirmed", "source": "shop"},
                    "feature": {"kind": "hole", "number": 2}, "anchor": [1.0, 2.0, 3.0], "flat": [1.0, 2.0],
                    "faces": ["Face1"], "outline": [], "reason": null
                }],
                "rules": [
                    {"ruleId": "sm.hole_min_diameter", "state": "advisory", "count": 1},
                    {"ruleId": "sm.hole_to_edge", "state": "unsupported", "count": 0}
                ],
                "notChecked": "Not checked here."
            }
        })
    }

    /// The nested findings object is read, not a bare array: counts, row values and freshness.
    #[test]
    fn the_inspect_reply_fills_the_card_and_keeps_the_stale_banner() {
        let card = crate::dfm_chat::Card::from_inspect(&report());
        assert_eq!(card.status, "done");
        assert_eq!(card.freshness, "staleModel", "the page shows the stale banner");
        assert_eq!(card.run_id, "run-1");
        assert_eq!(card.project_id, "project-1");
        assert_eq!(card.findings.len(), 1);
        let counts = card.counts();
        assert_eq!(counts["advisory"], 1);
        assert_eq!(counts["unsupported"], 1);
        assert_eq!(card.findings[0].measured.as_ref().map(|m| m.value), Some(2.4));
        assert_eq!(card.findings[0].requirement.as_ref().map(|r| r.value), Some(3.0));
        assert_eq!(card.not_checked, "Not checked here.");

        let page = crate::dfm_chat::page(&card).to_string();
        assert!(page.contains("\"status\":\"done\""));
        assert!(page.contains("\"findings\""));
        assert!(!page.contains("\"requirements\""));
        let model = crate::dfm_chat::model_context(&card).to_string();
        assert!(model.contains("\"requirement\""), "each finding keeps its own number");
        assert!(!model.contains("\"requirements\""));
        assert!(!model.contains("confirmation"), "the stored row identity stays home");
    }

    /// The card the page draws carries its counts: without them the page threw on
    /// `card.counts.failed` and left the card body black.
    #[test]
    fn the_pages_card_carries_the_counts() {
        let card = crate::dfm_chat::Card::from_inspect(&report());
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        app.agent_chat.push(Item::Tool {
            id: "row".into(),
            title: "DFM Check".into(),
            kind: "dfm.check".into(),
            status: Some(backend::ToolStatus::InProgress),
            input: None,
            output: None,
        });
        set_check_item(&mut app, "row", card);
        let output = app
            .agent_chat
            .items
            .iter()
            .rev()
            .find_map(|item| match item {
                Item::Tool { id, output: Some(output), .. } if id == "row" => Some(output.clone()),
                _ => None,
            })
            .expect("the row keeps its card");
        assert!(output.contains("\"counts\""), "{output}");
        assert!(output.contains("\"failed\""), "{output}");
    }

    /// A model's `dfm.check` reply becomes the check card whether the agent used a direct tool
    /// (no command kind) or the generic Code Mode `execute` tool, so both paths show the report.
    #[test]
    fn a_model_check_row_becomes_the_card() {
        for (call, kind) in [("call-1", ""), ("call-2", "execute")] {
            let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
            app.agent_chat.push(Item::Tool {
                id: call.into(),
                title: "execute: dfm.check".into(),
                kind: kind.into(),
                status: Some(backend::ToolStatus::Completed),
                input: None,
                output: Some(r#"{"projectId":"p","projectName":"Hole Tier","sheets":2}"#.into()),
            });
            adopt_model_check(&mut app);
            let row = app
                .agent_chat
                .items
                .iter()
                .rev()
                .find_map(|item| match item {
                    Item::Tool { id, kind, status, .. } if id == call => Some((kind.clone(), *status)),
                    _ => None,
                })
                .expect("the adopted row");
            assert_eq!(row.0, "dfm.check", "{kind}");
            assert!(matches!(row.1, Some(backend::ToolStatus::Completed | backend::ToolStatus::Failed)), "the row stops saying running: {:?}", row.1);
        }
    }

    /// A queued run reads as running, and a failed or replaced run keeps its state and reason.
    #[test]
    fn the_card_reads_running_cancelled_and_failed_runs() {
        let running = crate::dfm_chat::Card::from_inspect(&json!({"available": false, "running": true, "runId": "run-2"}));
        assert_eq!(running.status, "running");

        let cancelled = crate::dfm_chat::Card::from_inspect(&json!({
            "available": true, "running": false, "runId": "run-3", "outcome": "cancelled",
            "reason": "cancelled by the user",
            "findings": {"runId": "run-3", "findings": [], "rules": [], "notChecked": ""}
        }));
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(cancelled.reason.as_deref(), Some("cancelled by the user"));

        let failed = crate::dfm_chat::Card::from_inspect(&json!({
            "available": true, "running": false, "runId": "run-4", "outcome": "failed",
            "reason": "the sheet could not be read",
            "findings": {"runId": "run-4", "findings": [], "rules": [], "notChecked": ""}
        }));
        assert_eq!(failed.status, "error");
        assert_eq!(failed.error.as_deref(), Some("the sheet could not be read"));

        let queued = crate::dfm_chat::Card::from_check(&json!({"projectId": "p", "candidateDigest": "c", "configDigest": "g", "sheets": 1}));
        assert_eq!(queued.status, "running");
        assert_eq!(queued.project_id, "p");
    }

    /// Chat's "检查一下当前设计" calls the same entry as the card's refresh button; the sheet
    /// metal wording the same, while a negative, a question about an earlier check and a
    /// repeated phrase are not requests.
    #[test]
    fn the_chat_phrase_runs_the_same_check_entry_as_the_button() {
        for request in [
            "检查一下当前设计",
            "检查当前设计",
            "检查一下当前钣金件",
            "检查当前钣金件",
            "检查一下钣金件",
            "检查钣金件",
            "帮我检查一下当前钣金件",
            "检查一下当前钣金件。",
            "Check this design.",
            "check the sheet metal part",
        ] {
            assert!(crate::dfm_chat::is_check_request(request), "{request}");
        }
        for not_request in [
            "检查一下这个零件的孔",
            "不要检查一下当前钣金件",
            "别检查当前钣金件",
            "不需要检查",
            "取消检查",
            "上次检查的结果是什么",
            "之前的检查结果怎么样",
            "检查结果是什么",
            "检查一下当前钣金件的结果",
            "检查一下当前钣金件检查一下当前钣金件",
        ] {
            assert!(!crate::dfm_chat::is_check_request(not_request), "{not_request}");
        }
        let mut app = SolveApp::new(solidcraft_engine::Session::default(), Default::default());
        let before = app.agent_chat.items.len();
        act(&mut app, &egui::Context::default(), r#"{"type":"send","text":"检查一下当前钣金件"}"#).expect("send");
        assert_eq!(app.agent_chat.items.len(), before + 2, "a check row and its notice");
        assert_eq!(app.agent_chat.items.iter().filter(|item| matches!(item, Item::Tool { kind, .. } if kind == "dfm.check")).count(), 1);
        // The command is C's; until it lands the card says why, and the row is an error, not a result.
        let card = app.dfm_chat.card.as_ref().expect("a card");
        assert_eq!(card.status, "error");
        assert!(card.error.as_deref().is_some_and(|why| why.contains("dfm.check")));
        act(&mut app, &egui::Context::default(), r#"{"type":"dfm_display","mode":"off"}"#).expect("display");
        assert_eq!(app.dfm_chat.display.id(), "off");
        assert!(act(&mut app, &egui::Context::default(), r#"{"type":"dfm_display","mode":"nope"}"#).is_err());
        assert!(act(&mut app, &egui::Context::default(), r#"{"type":"finding","runId":"run-1","id":"sheet-1/F2"}"#).is_err());
    }
}
