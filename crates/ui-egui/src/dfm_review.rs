//! The Review panel: run a sheet-metal DFM check by hand, read its report, and hand one report
//! to the Assistant.
//!
//! The check is the host's own manual entry (`dfm.check`), so it runs without the Assistant being
//! open and without a model connection. The report is the project's stored run: this panel reads
//! it with `inspect.dfm` and shows the run id, the outcome, the rule rows and every finding with
//! its measured and required value. Selecting a row locates the finding in the viewport, and a
//! label picked in the viewport selects the same row, because both sides read the one selection
//! the viewport keeps.
//!
//! The card can be dragged onto the Assistant, where it becomes a draft attachment of the next
//! message. The attachment names the run it came from, and the message reads that run at send
//! time, so a newer check does not replace what the person chose to send.

use egui::{Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use serde_json::Value;
use solidcraft_engine::entitlement::{BillingResult, Capability, EntryCategory, OperationId, RefusalCode, ReviewOperation, RunId};

use crate::SolveApp;
use crate::dfm_chat::Card;
use crate::i18n::{format as trf, tr};
use crate::theme::Tokens;

/// How often a running check asks for its report, in seconds.
const POLL_SECONDS: f64 = 0.4;
/// How many findings the list shows; the rest are counted under it, so a huge report stays cheap.
const MAX_ROWS: usize = 200;
/// The finding states in the order the list groups them; any other state is listed last as not
/// covered, so a finding no rule could judge is never dropped from the report.
const GROUPS: [&str; 4] = ["failed", "advisory", "checked", "passed"];

/// What the Review panel keeps between frames.
#[derive(Default)]
pub struct State {
    /// The report on show, if a check has finished for this project.
    pub card: Option<Card>,
    /// The run the card shows.
    pub run: Option<String>,
    /// A check this panel started is still running.
    pub running: bool,
    /// Why the check could not run or did not finish.
    pub error: Option<String>,
    /// When the panel last asked for the report, in the frame clock's seconds.
    polled: f64,
    /// The run id of the card being dragged towards the Assistant.
    pub dragging: Option<String>,
    /// Where the pointer last was while the report was dragged: the release frame may not carry a
    /// position when the drop lands on the Assistant's web view.
    drag_at: Option<egui::Pos2>,
    /// The run that was latest when this panel's check started, to tell its own result apart.
    started_from: Option<String>,
    /// The design whose latest report the panel has looked for, so opening it again does not.
    document: Option<String>,
    /// Polls in a row where the service was idle and no new run had appeared yet: the worker may
    /// not have picked the job up, and a check that never records is told as a failure later.
    quiet: u8,
    /// The "what was not checked" list is unfolded.
    pub not_checked_open: bool,
    /// The billed logical operation for this panel's check, once one was admitted.
    pub(crate) operation: Option<OperationId>,
    /// The run this operation is durably mapped to, when one has been recorded.
    pub(crate) mapped_run: Option<String>,
    /// Whether this operation has already committed or released. A settled operation is never
    /// settled again, even if the same report is attached to the Assistant.
    pub(crate) settled: bool,
}

impl State {
    /// Forget the report: a new design, a new project or a closed panel has nothing to show.
    /// Billing identity is kept: a check that started here still needs its one settlement even
    /// when the report is replaced by another document's.
    pub fn clear(&mut self) {
        let operation = self.operation.take();
        let mapped_run = self.mapped_run.take();
        let settled = self.settled;
        *self = Self { operation, mapped_run, settled, ..Self::default() };
    }
}

/// Whether this panel can run a check here: the desktop build has the background worker, the
/// browser build has neither a project store nor a worker.
pub fn available() -> bool {
    !cfg!(target_arch = "wasm32")
}

fn refusal_name(code: RefusalCode) -> &'static str {
    match code {
        RefusalCode::AuthRequired => "AUTH_REQUIRED",
        RefusalCode::EmailUnverified => "EMAIL_UNVERIFIED",
        RefusalCode::AccountDisabled => "ACCOUNT_DISABLED",
        RefusalCode::CapabilityDenied => "CAPABILITY_DENIED",
        RefusalCode::VersionUnsupported => "VERSION_UNSUPPORTED",
        RefusalCode::ProtocolUnsupported => "PROTOCOL_UNSUPPORTED",
        RefusalCode::UnsafeToStart => "UNSAFE_TO_START",
        RefusalCode::CreditsExhausted => "CREDITS_EXHAUSTED",
        RefusalCode::CreditsHeld => "CREDITS_HELD",
        RefusalCode::OperationConflict => "OPERATION_CONFLICT",
        RefusalCode::OperationFinalized => "OPERATION_FINALIZED",
        RefusalCode::OperationOwnedElsewhere => "OPERATION_OWNED_ELSEWHERE",
        RefusalCode::AuthUnavailable => "AUTH_UNAVAILABLE",
    }
}

fn coded(code: RefusalCode, message: impl std::fmt::Display) -> String {
    format!("{}: {message}", refusal_name(code))
}

/// Why a Review press cannot start. This is checked before any operation is minted, so a missing
/// authority never reaches an execution path and never runs a check unbilled.
fn authority_error(app: &SolveApp) -> Option<String> {
    if !app.budget.is_verified(Capability::Review, EntryCategory::ReviewPanel) {
        return Some(coded(RefusalCode::UnsafeToStart, "this Review path has no verified safe-stop bound"));
    }
    if app.billing.is_none() {
        return Some(coded(RefusalCode::AuthUnavailable, "no host billing authority is installed for the Review panel"));
    }
    None
}

/// Mint and admit one logical operation for a Review press. A previous press that never settled
/// blocks a new one instead of risking a second charge for the same work.
pub(crate) fn begin_review(app: &mut SolveApp) -> Result<OperationId, String> {
    if !app.dfm_review.settled
        && let Some(operation) = app.dfm_review.operation.as_ref()
    {
        return Err(coded(RefusalCode::UnsafeToStart, format!("the previous Review operation {} has no confirmed settlement", operation.as_str())));
    }
    if let Some(error) = authority_error(app) {
        return Err(error);
    }
    let operation = crate::control::mint_operation()
        .ok_or_else(|| coded(RefusalCode::AuthUnavailable, "the host could not mint an operation for the Review check"))?;
    let grant = app
        .billing
        .as_mut()
        .ok_or_else(|| coded(RefusalCode::AuthUnavailable, "no host billing authority is installed for the Review panel"))?
        .admit(&operation, Capability::Review, EntryCategory::ReviewPanel)
        .map_err(|error| coded(error.code, error.message))?;
    drop(grant); // Execution authority, never a value the panel stores or logs.
    app.dfm_review.operation = Some(operation.clone());
    app.dfm_review.mapped_run = None;
    app.dfm_review.settled = false;
    Ok(operation)
}

/// Persist the run-to-operation identity before any settlement. The mapping is the only durable
/// way a restart can settle the original operation instead of minting a new one or re-running.
pub(crate) fn bind_review_run(app: &mut SolveApp, run: &str) -> Result<(), String> {
    let Some(operation) = app.dfm_review.operation.clone() else { return Ok(()) };
    if run.is_empty() {
        return Err(coded(RefusalCode::UnsafeToStart, "the Review check has no run identity to bind"));
    }
    if app.dfm_review.mapped_run.as_deref() == Some(run) {
        return Ok(());
    }
    if let Some(store) = app.review_store.as_mut() {
        store
            .record_review(&ReviewOperation::new(RunId::new(run), operation.clone()))
            .map_err(|error| coded(RefusalCode::AuthUnavailable, format!("the Review ledger could not record this run: {error:?}")))?;
    }
    app.dfm_review.mapped_run = Some(run.to_string());
    Ok(())
}

/// Settle one Review run once. `produced` is true only when the panel read a finished check's
/// report (violations or not). A cancelled, failed or unreadable check releases its hold.
///
/// The mapping is checked first. On restart the state may be gone but the durable mapping names
/// the original operation; a terminal ledger row is never settled again.
pub(crate) fn settle_review_run(app: &mut SolveApp, run: &str, produced: bool) -> Result<(), String> {
    if app.dfm_review.settled {
        return Ok(());
    }
    let operation = match app.dfm_review.operation.clone() {
        Some(operation) => {
            bind_review_run(app, run)?;
            operation
        }
        None => {
            let Some(store) = app.review_store.as_ref() else { return Ok(()) };
            match store.review_operation(&RunId::new(run)) {
                Ok(Some(operation)) => {
                    app.dfm_review.mapped_run = Some(run.to_string());
                    operation
                }
                Ok(None) => return Ok(()),
                Err(error) => {
                    return Err(coded(RefusalCode::AuthUnavailable, format!("the Review ledger could not read this run: {error:?}")));
                }
            }
        }
    };
    if let Some(store) = app.review_store.as_ref() {
        match store.load(&operation) {
            Ok(Some(record)) if record.status.is_terminal() => {
                app.dfm_review.settled = true;
                return Ok(());
            }
            Ok(Some(_)) => {}
            Ok(None) => {
                return Err(coded(
                    RefusalCode::AuthUnavailable,
                    format!("the Review operation {} has no durable row; it cannot settle safely", operation.as_str()),
                ));
            }
            Err(error) => {
                return Err(coded(RefusalCode::AuthUnavailable, format!("the Review ledger is unavailable: {error:?}")));
            }
        }
    }
    let Some(billing) = app.billing.as_mut() else {
        return Err(coded(RefusalCode::AuthUnavailable, "no host billing authority is installed for the Review panel"));
    };
    let outcome = if produced {
        billing.commit(&operation, BillingResult::Succeeded).map(|_| ())
    } else {
        billing.release(&operation, RefusalCode::UnsafeToStart)
    };
    match outcome {
        Ok(()) => {
            app.dfm_review.settled = true;
            Ok(())
        }
        Err(error) => Err(coded(
            error.code,
            format!("the Review result was handled but settlement of operation {} was not confirmed; do not run it again", operation.as_str()),
        )),
    }
}

fn release_unstarted(app: &mut SolveApp, reason: RefusalCode) -> Result<(), String> {
    if app.dfm_review.settled {
        return Ok(());
    }
    let Some(operation) = app.dfm_review.operation.clone() else { return Ok(()) };
    let Some(billing) = app.billing.as_mut() else {
        return Err(coded(RefusalCode::AuthUnavailable, "no host billing authority is installed for the Review panel"));
    };
    billing.release(&operation, reason).map_err(|error| {
        coded(error.code, format!("operation {} could not be released; it stays for recovery and must not be rerun", operation.as_str()))
    })?;
    app.dfm_review.settled = true;
    Ok(())
}

/// Start one check from the panel and keep it on show while it runs. Nothing of the Assistant is
/// needed: the entry is the host's own, and a refusal is the panel's error, not a chat notice.
pub fn start(app: &mut SolveApp) {
    app.dfm_review.error = None;
    app.dfm_review.dragging = None;
    if !available() {
        app.dfm_review.error = Some(tr("ui.dfm_review_unavailable").to_string());
        return;
    }
    if app.dfm_review.running {
        // One press is one logical operation; a second click while it runs starts nothing.
        return;
    }
    if let Err(error) = begin_review(app) {
        app.dfm_review.error = Some(error);
        return;
    }
    // What was latest is read before the job is submitted: a fast check can finish before this
    // frame ends, and its own run is then the one to show.
    let was = crate::agent_chat::last_dfm_run();
    app.dfm_review.started_from = was;
    if crate::agent_chat::run_dfm(app) {
        app.dfm_review.running = true;
        app.dfm_review.polled = 0.0;
        app.dfm_review.quiet = 0;
        app.dfm_review.error = None;
    } else {
        let release = release_unstarted(app, RefusalCode::UnsafeToStart);
        app.dfm_review.error = Some(match release {
            Ok(()) => tr("ui.dfm_review_start_failed").to_string(),
            Err(error) => error,
        });
    }
}

/// Stop the live check. Its record can never take `latest` back, so the panel keeps the report it
/// already shows. A cancelled check produced no report and releases its hold once.
pub fn cancel(app: &mut SolveApp) {
    if available() {
        crate::agent_chat::cancel_dfm(app);
    }
    app.dfm_review.running = false;
    if let Err(error) = release_unstarted(app, RefusalCode::UnsafeToStart) {
        app.dfm_review.error = Some(error);
    }
}

/// Read the project's newest report into the panel, without running anything. A project with no
/// finished check leaves the panel empty, ready for its Run button.
pub fn reload(app: &mut SolveApp) {
    if !available() {
        return;
    }
    let card = crate::dfm_chat::refresh_run(app, None);
    if card.available {
        adopt(app, card);
    }
}

/// While a check this panel started runs, read its report until it finishes. A frame that finds
/// it finished shows the new run, or the reason it failed. The panel asks every [`POLL_SECONDS`],
/// not every frame: the worker writes its record once, and each ask is a project read.
pub fn poll(app: &mut SolveApp, ctx: &egui::Context) {
    if !app.dfm_review.running || !available() {
        return;
    }
    let now = ctx.input(|i| i.time);
    let waited = now - app.dfm_review.polled;
    if (0.0..POLL_SECONDS).contains(&waited) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(POLL_SECONDS - waited));
        return;
    }
    app.dfm_review.polled = now;
    ctx.request_repaint();
    let status = crate::agent_chat::dfm_state();
    if status.running {
        // The worker names its run as soon as the validation job exists. Bind it before the
        // result can settle, so a restart can find the original operation.
        if let Some(run) = status.active_run.as_deref()
            && let Err(error) = bind_review_run(app, run)
        {
            app.dfm_review.running = false;
            app.dfm_review.error = Some(error);
            return;
        }
        app.dfm_review.quiet = 0;
        return;
    }
    // The run this press submitted is the one the service names once it finishes; before that,
    // and while the worker has not picked the job up, the service is idle with the old run.
    let finished = status.last_run.filter(|run| !run.is_empty() && Some(run) != app.dfm_review.started_from.as_ref());
    let Some(finished) = finished else {
        if let Some(error) = status.error {
            app.dfm_review.running = false;
            let settle = app
                .dfm_review
                .mapped_run
                .clone()
                .or_else(|| status.active_run.clone())
                .map(|run| settle_review_run(app, &run, false))
                .unwrap_or_else(|| release_unstarted(app, RefusalCode::UnsafeToStart));
            app.dfm_review.error = Some(settle.err().unwrap_or(error));
            return;
        }
        app.dfm_review.quiet = app.dfm_review.quiet.saturating_add(1);
        if app.dfm_review.quiet < 5 {
            // Give the worker its moment before calling the check lost.
            return;
        }
        app.dfm_review.running = false;
        let settle = app
            .dfm_review
            .mapped_run
            .clone()
            .map(|run| settle_review_run(app, &run, false))
            .unwrap_or_else(|| release_unstarted(app, RefusalCode::UnsafeToStart));
        app.dfm_review.error = Some(settle.err().unwrap_or_else(|| tr("ui.dfm_review_did_not_finish").to_string()));
        return;
    };
    app.dfm_review.running = false;
    if let Err(error) = bind_review_run(app, &finished) {
        app.dfm_review.error = Some(error);
        return;
    }
    let card = crate::dfm_chat::refresh_run(app, Some(&finished));
    let produced = card.status == "done" && card.available && card.run_id == finished;
    let settle = settle_review_run(app, &finished, produced);
    if card.status == "error" || !card.available {
        app.dfm_review.error = settle.err().or_else(|| card.error.clone()).or_else(|| Some(tr("ui.dfm_review_unreadable").to_string()));
        return;
    }
    app.dfm_review.error = settle.err();
    adopt(app, card);
}

/// Show `card` and let the viewport draw its marks, so a row and a label address one finding.
fn adopt(app: &mut SolveApp, card: Card) {
    app.dfm_review.run = Some(card.run_id.clone()).filter(|run| !run.is_empty());
    crate::dfm_chat::show_marks(app, &card);
    app.dfm_chat.card = Some(card.clone());
    app.dfm_review.card = Some(card);
}

/// The report card's run as the Assistant's attachment: the run to read again when the message is
/// sent, and what the chip says.
pub fn attachment(app: &mut SolveApp, run: &str) -> Result<crate::agent_chat::attach::Attachment, String> {
    // The card on show is the report itself; only a run the panel does not show is read back.
    let card = match app.dfm_review.card.clone().filter(|card| card.run_id == run) {
        Some(card) => card,
        None => crate::dfm_chat::refresh_run(app, Some(run)),
    };
    if card.status == "error" {
        return Err(card.error.unwrap_or_else(|| tr("ui.dfm_review_unreadable").to_string()));
    }
    if card.run_id.is_empty() || card.run_id != run {
        return Err(trf("ui.dfm_review_no_such_run", &[("run", run)]));
    }
    Ok(crate::agent_chat::attach::Attachment::dfm_report(&card))
}

/// The Assistant's panel takes a dropped report: say so while the card is dragged over it.
pub fn drag_hint(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(_run) = app.dfm_review.dragging.as_deref() else { return };
    // While the pointer is over the Assistant's web view the app may not be told a position on
    // the frame the button comes up, so the drag remembers the last one it was told.
    if let Some(at) = ctx.input(|i| i.pointer.latest_pos()) {
        app.dfm_review.drag_at = Some(at);
    }
    let Some(rect) = app.agent_chat.drop_rect else { return };
    let t = Tokens::get();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("sc_dfm_review_drop_hint")));
    let zone = rect.shrink(6.0);
    painter.rect(zone, t.radius_xl, t.accent.gamma_multiply(0.16), Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    let font = egui::FontId::proportional(t.font_heading);
    let text = painter.layout(tr("ui.dfm_review_drop_hint").to_string(), font, t.text, (zone.width() - 24.0).max(40.0));
    painter.galley(zone.center() - text.size() / 2.0, text, t.text);
    crate::scenario::publish_rect("dfm_review:drop_hint", zone);
}

/// The pointer was released after dragging a report: over the Assistant it becomes an attachment,
/// anywhere else the drag is dropped.
pub fn release(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(run) = app.dfm_review.dragging.clone() else { return };
    let (released, at) = ctx.input(|i| (i.pointer.any_released(), i.pointer.latest_pos()));
    if !released {
        return;
    }
    app.dfm_review.dragging = None;
    let at = at.or(app.dfm_review.drag_at.take());
    if let (Some(at), Some(rect)) = (at, app.agent_chat.drop_rect)
        && rect.contains(at)
    {
        attach_to_assistant(app, &run);
    }
    ctx.request_repaint();
}

/// Put the report on the Assistant's next message, or say why it cannot be.
pub fn attach_to_assistant(app: &mut SolveApp, run: &str) {
    match attachment(app, run) {
        Ok(file) => {
            if let Err(reason) = crate::agent_chat::attach_ready(app, file) {
                crate::agent_chat::notify(app, reason, true);
            }
        }
        Err(reason) => crate::agent_chat::notify(app, reason, true),
    }
}

/// The panel's contents, below the rail's title strip.
pub fn panel(app: &mut SolveApp, ui: &mut egui::Ui, area: Rect) {
    // A design the panel has not seen yet: show the report the project already has, if any. A
    // report of another design is never shown for this one.
    let document = app.session.document_stamp().document_id;
    if app.dfm_review.document.as_deref() != Some(document.as_str()) {
        app.dfm_review.clear();
        app.dfm_review.document = Some(document);
        reload(app);
    }
    let t = Tokens::get();
    let mut child =
        ui.new_child(egui::UiBuilder::new().max_rect(area.shrink2(vec2(t.space_12, t.space_8))).layout(egui::Layout::top_down(egui::Align::Min)));
    let ui = &mut child;
    actions(app, ui);
    ui.add_space(t.space_8);
    if let Some(error) = app.dfm_review.error.clone() {
        crate::ui_kit::text(ui, &error, crate::ui_kit::Text::Error(&error));
        ui.add_space(t.space_8);
    }
    let Some(card) = app.dfm_review.card.clone() else {
        if !app.dfm_review.running {
            crate::ui_kit::text(ui, tr("ui.dfm_review_empty"), crate::ui_kit::Text::Note(tr("ui.dfm_review_empty")));
        }
        return;
    };
    card_header(app, ui, &card);
    ui.add_space(t.space_8);
    // The list takes the room left above the footer; "not checked" is a folded line, because the
    // report's own list of what no rule covered is long and is read when someone looks for it.
    let footer = t.space_16 * 2.0;
    let list_height = (ui.available_height() - footer).max(t.control_height * 2.0);
    findings(app, ui, &card, list_height);
    let missing = card.not_checked.lines().filter(|line| !line.trim().is_empty()).count();
    if missing > 0 {
        ui.add_space(t.space_4);
        let label = trf("ui.dfm_review_not_checked", &[("n", &missing.to_string())]);
        let open = app.dfm_review.not_checked_open;
        let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), t.control_height * 0.8), Sense::click());
        let icon = Rect::from_min_size(rect.min, vec2(rect.height(), rect.height())).shrink(t.form_icon_inset);
        let fold = crate::ui_kit::disclosure_button_at(ui, icon, open, &label);
        let galley = ui.painter().layout_no_wrap(label.clone(), egui::FontId::proportional(t.font_caption), t.text_dim);
        ui.painter().galley(pos2(icon.right() + t.space_4, rect.center().y - galley.size().y / 2.0), galley, t.text_dim);
        if row.clicked() || fold.clicked() {
            app.dfm_review.not_checked_open = !open;
        }
        if open {
            ui.label(RichText::new(&card.not_checked).font(egui::FontId::proportional(t.font_caption)).color(t.text_dim));
        }
    }
}

/// Run, cancel and the running state.
fn actions(app: &mut SolveApp, ui: &mut egui::Ui) {
    let t = Tokens::get();
    if app.dfm_review.running {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(tr("ui.dfm_review_running")).font(egui::FontId::proportional(t.font_small)).color(t.text_dim));
        });
        if crate::ui_kit::button(ui, tr("ui.dfm_review_cancel"), crate::ui_kit::Button::Normal).clicked() {
            cancel(app);
        }
        return;
    }
    let label = if app.dfm_review.card.is_some() { tr("ui.dfm_review_run_again") } else { tr("ui.dfm_review_run") };
    let response = crate::ui_kit::button(ui, label, crate::ui_kit::Button::Primary);
    crate::scenario::publish_target("review:run", response.rect);
    if response.clicked() {
        start(app);
    }
}

/// The card's top: its run, outcome and counts, and the grip that drags it to the Assistant.
fn card_header(app: &mut SolveApp, ui: &mut egui::Ui, card: &Card) {
    let t = Tokens::get();
    let run = card.run_id.clone();
    let outcome = card.status.clone();
    let freshness = card.freshness.clone();
    let counts = card.counts();
    let state = match outcome.as_str() {
        "running" => tr("ui.dfm_review_running"),
        "cancelled" => tr("ui.dfm_review_cancelled"),
        _ => tr("ui.dfm_review_done"),
    };
    let summary = trf(
        "ui.dfm_review_counts",
        &[
            ("failed", &count(&counts, "failed")),
            ("advisory", &count(&counts, "advisory")),
            ("checked", &count(&counts, "checked")),
            ("not_covered", &count(&counts, "unsupported")),
        ],
    );
    let drag = format!("{}\n{}", tr("ui.dfm_review_drag"), run);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), t.control_height * 2.0), Sense::click_and_drag());
    let painter = ui.painter();
    let fill = if response.hovered() || response.dragged() { t.hover } else { t.panel };
    painter.rect_filled(rect, t.radius_md, fill);
    painter.rect_stroke(rect, t.radius_md, t.primary_border(t.border), egui::StrokeKind::Inside);
    let ink = t.text;
    let mut top = rect.top() + t.space_6;
    let title = painter.layout_no_wrap(state.to_string(), egui::FontId::proportional(t.font_label), ink);
    painter.galley(pos2(rect.left() + t.space_8, top), title.clone(), ink);
    top += title.size().y + t.space_2;
    let line = painter.layout_no_wrap(summary, egui::FontId::proportional(t.font_caption), t.text_dim);
    painter.galley(pos2(rect.left() + t.space_8, top), line.clone(), t.text_dim);
    top += line.size().y + t.space_2;
    let stale = matches!(freshness.as_str(), "staleModel" | "staleConfig" | "staleBoth");
    let note = if stale { format!("{} · {}", short_run(&run), tr("ui.dfm_review_stale")) } else { short_run(&run) };
    let ink = if stale { t.warning } else { t.text_dim };
    let g = painter.layout_no_wrap(note, egui::FontId::proportional(t.font_caption), ink);
    painter.galley(pos2(rect.left() + t.space_8, top), g, ink);
    response.clone().on_hover_text(drag);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, tr("ui.dfm_review_card")));
    crate::scenario::publish_target("review:card", rect);
    if response.drag_started() {
        app.dfm_review.dragging = Some(run.clone());
    }
    if response.dragged()
        && let Some(at) = ui.ctx().pointer_latest_pos()
    {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("sc_dfm_review_ghost")));
        let text = painter.layout_no_wrap(tr("ui.dfm_review_dragging").to_string(), egui::FontId::proportional(t.font_small), t.on_accent);
        let size = text.size() + vec2(t.space_12, t.space_6);
        let at = at + vec2(t.space_12, t.space_12);
        let frame = Rect::from_min_size(at, size);
        painter.rect(frame, t.radius_md, t.form_primary_fill, Stroke::NONE, egui::StrokeKind::Inside);
        painter.galley(at + vec2(t.space_6, t.space_3), text, t.on_accent);
    }
}

fn count(counts: &Value, key: &str) -> String {
    counts.get(key).and_then(Value::as_u64).unwrap_or(0).to_string()
}

fn short_run(run: &str) -> String {
    run.chars().take(24).collect()
}

/// The rule rows, then the findings grouped by what they say: failures first, then advice, then
/// what was checked, and last what passed. A row selects its finding in the viewport.
fn findings(app: &mut SolveApp, ui: &mut egui::Ui, card: &Card, height: f32) {
    let t = Tokens::get();
    let selected = app.viewport.dfm.selected_key().map(|(run, id)| (run.to_string(), id.to_string())).or_else(|| app.dfm_chat.selected.clone());
    egui::ScrollArea::vertical().id_salt("sc_dfm_review_findings").max_height(height).auto_shrink([false, false]).show(ui, |ui| {
        let mut shown = 0;
        let mut states: Vec<&str> = GROUPS.to_vec();
        for finding in &card.findings {
            if !states.contains(&finding.state.as_str()) {
                states.push(finding.state.as_str());
            }
        }
        for state in states {
            let rows: Vec<&crate::dfm_chat::Finding> = card.findings.iter().filter(|finding| finding.state == state).collect();
            if rows.is_empty() {
                continue;
            }
            ui.add_space(t.space_6);
            ui.label(
                RichText::new(format!("{} {}", rows.len(), state_word(state))).font(egui::FontId::proportional(t.font_caption)).color(t.text_dim),
            );
            for finding in rows {
                if shown >= MAX_ROWS {
                    return;
                }
                shown += 1;
                let is_selected = selected.as_ref().is_some_and(|(run, id)| run == &card.run_id && id == &finding.id);
                if row(ui, finding, is_selected) && available() {
                    // The row is this card's own: it never depends on which run the Assistant
                    // happens to show, so a check started from Chat does not stop these rows.
                    app.dfm_chat.selected = Some((card.run_id.clone(), finding.id.clone()));
                    app.viewport.select_dfm_finding(Some(&finding.id));
                }
            }
        }
        if shown == 0 {
            crate::ui_kit::text(ui, tr("ui.dfm_review_no_findings"), crate::ui_kit::Text::Note(tr("ui.dfm_review_no_findings")));
        } else if card.findings.len() > shown {
            let left = (card.findings.len() - shown).to_string();
            let more = trf("ui.dfm_review_more", &[("n", &left)]);
            crate::ui_kit::text(ui, &more, crate::ui_kit::Text::Note(&more));
        }
    });
}

fn state_word(state: &str) -> &'static str {
    match state {
        "failed" => tr("ui.dfm_review_failed"),
        "advisory" => tr("ui.dfm_review_advisory"),
        "checked" => tr("ui.dfm_review_checked"),
        "passed" => tr("ui.dfm_review_passed"),
        _ => tr("ui.dfm_review_not_covered"),
    }
}

fn state_ink(state: &str) -> Color32 {
    let t = Tokens::get();
    match state {
        "failed" => t.error,
        "advisory" => t.warning,
        "passed" | "checked" => t.text_dim,
        _ => t.warning,
    }
}

/// One finding: its state, its rule and what it applies to, and the measured and required values.
fn row(ui: &mut egui::Ui, finding: &crate::dfm_chat::Finding, selected: bool) -> bool {
    let t = Tokens::get();
    let width = ui.available_width();
    let lines: Vec<(String, Color32)> = {
        let mut lines = Vec::new();
        let mut head = format!("{} · {}", finding.rules.join(", "), finding.body);
        if let Some(feature) = &finding.feature
            && !feature.kind.is_empty()
        {
            head.push_str(&format!(" · {}{}", feature.kind, feature.number.map(|n| format!(" {n}")).unwrap_or_default()));
        }
        lines.push((head, t.text));
        if let Some(measured) = &finding.measured {
            let value = format!("{} {}", measured.value, measured.unit);
            let line = match &finding.requirement {
                Some(req) => trf("ui.dfm_review_value", &[("measured", &value), ("required", &format!("{} {}", req.value, req.unit))]),
                None => trf("ui.dfm_review_value_no_requirement", &[("measured", &value)]),
            };
            lines.push((line, t.text_dim));
        } else if let Some(reason) = &finding.reason {
            lines.push((reason.clone(), t.text_dim));
        }
        lines
    };
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(t.font_caption)));
    let height = t.space_4 * 2.0 + line_height * lines.len() as f32;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, t.radius_sm, t.selected_bg);
    } else if response.hovered() {
        painter.rect_filled(rect, t.radius_sm, t.hover);
    }
    let ink = if selected { t.selected_ink } else { t.text };
    let mut y = rect.top() + t.space_4;
    for (at, (text, tone)) in lines.iter().enumerate() {
        let colour = if selected {
            t.selected_ink
        } else if at == 0 {
            unit_ink(&finding.state, ink)
        } else {
            *tone
        };
        let galley = painter.layout(text.clone(), egui::FontId::proportional(t.font_caption), colour, (rect.width() - t.space_8 * 2.0).max(20.0));
        painter.galley(pos2(rect.left() + t.space_8, y), galley, colour);
        y += line_height;
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &finding.id));
    crate::scenario::publish_target(&format!("review:finding:{}", finding.id), rect);
    response.clicked()
}

fn unit_ink(state: &str, ink: Color32) -> Color32 {
    match state {
        "failed" | "advisory" => state_ink(state),
        _ => ink,
    }
}

#[cfg(test)]
#[path = "dfm_review_tests.rs"]
mod tests;
