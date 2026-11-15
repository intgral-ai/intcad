//! The Review panel's own behaviour: the check entry, the report it shows, a row that chooses a
//! finding, and the card dragged onto the Assistant as an attachment.

use crate::dfm_chat::{Card, Finding, Measure, Requirement};
use crate::scenario::{Harness, rect_of};
use crate::shell::LayoutMode;
use serde_json::json;

fn window(mode: LayoutMode) -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    h.call("ui.prefs", json!({"right_rail": true}));
    h.call("ui.resize", json!({"width": 1500, "height": 900}));
    h.call("ui.set", json!({"layout": mode}));
    h.app.run("solid.box", json!({"length": 40, "width": 30, "height": 20})).expect("a body");
    h.settle();
    h.step(&json!({"click": {"handle": "rail:item:review"}})).expect("the Review item");
    h.settle();
    h
}

/// One finished current run with a failed hole finding that has an anchor, and one passed finding
/// without one, as the project's report gives them.
fn card() -> Card {
    Card {
        run_id: "run-a".into(),
        candidate_digest: "candidate-a".into(),
        config_digest: "config-a".into(),
        model_version: "model-a".into(),
        project_id: "project-a".into(),
        freshness: "current".into(),
        status: "done".into(),
        available: true,
        not_checked: "Bend sequence was not checked.".into(),
        rules: vec![crate::dfm_chat::RuleRow { rule_id: "sm.hole_min_diameter".into(), state: "failed".into(), count: 1 }],
        findings: vec![
            Finding {
                id: "sheet-1/F10".into(),
                sheet: 0,
                body: "Body1".into(),
                from: "rules".into(),
                rules: vec!["sm.hole_min_diameter".into()],
                state: "failed".into(),
                status: "needs_review".into(),
                code: Some("HOLE_DIAMETER_BELOW_LIMIT".into()),
                measured: Some(Measure { value: 3.0, unit: "mm".into() }),
                requirement: Some(Requirement {
                    value: 3.5,
                    unit: "mm".into(),
                    layer: "factory".into(),
                    confirmed: Some("factory_confirmed".into()),
                    source: "shop".into(),
                }),
                feature: Some(crate::dfm_chat::Feature { kind: "hole".into(), number: Some(2) }),
                anchor: Some([16.5, 33.5, 1.0]),
                ..Finding::default()
            },
            Finding {
                id: "sheet-1/F11".into(),
                sheet: 0,
                body: "Body1".into(),
                from: "rules".into(),
                rules: vec!["sm.hole_to_edge".into()],
                state: "passed".into(),
                status: "pass".into(),
                feature: Some(crate::dfm_chat::Feature { kind: "hole".into(), number: Some(3) }),
                ..Finding::default()
            },
        ],
        ..Card::default()
    }
}

/// Show `card` for this design without reading a project.
fn show(h: &mut Harness, card: Card) {
    let document = h.app.session.document_stamp().document_id;
    h.app.dfm_review.document = Some(document);
    h.app.dfm_review.run = Some(card.run_id.clone());
    crate::dfm_chat::show_marks(&mut h.app, &card);
    h.app.dfm_chat.card = Some(card.clone());
    h.app.dfm_review.card = Some(card);
    h.settle();
}

/// With no report yet the panel offers the check itself, without the Assistant: a press starts
/// one run of the host's manual entry, and the panel says so while it runs.
#[test]
fn a_project_with_no_report_offers_the_check() {
    let h = window(LayoutMode::Docked);
    let run = rect_of("review:run").expect("the Run button");
    let panel = rect_of("rail:panel").expect("the Review panel is open");
    assert!(panel.contains_rect(run), "{run:?} in {panel:?}");
    assert!(rect_of("review:card").is_none(), "no report is on show");
    assert!(h.app.dfm_review.card.is_none());
}

/// A finished report lists its run, its counts and the findings that failed, with the measured and
/// required value, and a row chooses the same finding in the viewport.
#[test]
fn a_report_row_shows_the_values_and_chooses_the_finding() {
    let mut h = window(LayoutMode::Docked);
    show(&mut h, card());
    assert!(rect_of("review:card").is_some(), "the report's card");
    let failed = rect_of("review:finding:sheet-1/F10").expect("the failed finding's row");
    let passed = rect_of("review:finding:sheet-1/F11").expect("the passed finding's row");
    assert!(failed.top() < passed.top(), "failures stand above what passed: {failed:?} {passed:?}");
    let panel = rect_of("rail:panel").expect("the panel");
    assert!(panel.contains_rect(failed) && panel.contains_rect(passed), "{panel:?}");
    // The Assistant may be showing another run's card; the Review rows are this card's own.
    let mut other = card();
    other.run_id = "run-other".into();
    h.app.dfm_chat.card = Some(other);
    h.step(&json!({"click": {"handle": "review:finding:sheet-1/F10"}})).expect("the row");
    h.settle();
    assert_eq!(h.app.viewport.dfm.selected_key(), Some(("run-a", "sheet-1/F10")), "the row chooses the finding in the viewport");
    assert_eq!(h.app.dfm_chat.selected.as_ref().map(|(run, id)| (run.as_str(), id.as_str())), Some(("run-a", "sheet-1/F10")));
}

/// A finding no rule could judge is listed under its own heading, never dropped: a report that
/// hides what it could not check reads as if everything else were fine.
#[test]
fn a_finding_no_rule_could_judge_is_listed_as_not_covered() {
    let mut h = window(LayoutMode::Docked);
    let mut shown = card();
    shown.findings.push(Finding {
        id: "sheet-1/F5".into(),
        sheet: 0,
        body: "Body1".into(),
        rules: vec!["sm.countersink_depth".into()],
        state: "unsupported".into(),
        status: "not_checked".into(),
        feature: Some(crate::dfm_chat::Feature { kind: "countersink".into(), number: None }),
        reason: Some("the App did not measure countersinks".into()),
        ..Finding::default()
    });
    show(&mut h, shown);
    let row = rect_of("review:finding:sheet-1/F5").expect("the finding no rule judged still has a row");
    let panel = rect_of("rail:panel").expect("the panel");
    assert!(panel.contains_rect(row), "{row:?} in {panel:?}");
}

/// The card is dragged onto the Assistant: one report attachment, named after its run, and the
/// report is the one on show.
#[test]
fn the_card_dropped_on_the_assistant_becomes_an_attachment() {
    let mut h = window(LayoutMode::Docked);
    h.app.agent_chat.open = true;
    show(&mut h, card());
    let from = rect_of("review:card").expect("the report's card");
    let to = rect_of("agent_chat:panel").expect("the Assistant is open");
    h.step(&json!({"drag": [[from.center().x as f64, from.center().y as f64], [to.center().x as f64, to.center().y as f64]], "steps": 12}))
        .expect("the drag");
    h.settle();
    assert!(h.app.dfm_review.dragging.is_none(), "the drag ended");
    let reports: Vec<&crate::agent_chat::attach::Attachment> = h.app.agent_chat.attachments.iter().filter(|file| file.is_report()).collect();
    assert_eq!(reports.len(), 1, "one report attachment: {:?}", h.app.agent_chat.attachments);
    assert_eq!(reports[0].report_run(), Some("run-a"));
    assert!(reports[0].name.contains("run-a"), "the chip names the run: {}", reports[0].name);
}

/// A drag that ends away from the Assistant leaves nothing behind.
#[test]
fn a_card_dropped_elsewhere_leaves_no_attachment() {
    let mut h = window(LayoutMode::Docked);
    h.app.agent_chat.open = true;
    show(&mut h, card());
    let from = rect_of("review:card").expect("the report's card");
    let view = h.app.viewport.rect.expect("the view");
    h.step(&json!({"drag": [[from.center().x as f64, from.center().y as f64], [view.center().x as f64, view.center().y as f64]], "steps": 12}))
        .expect("the drag");
    h.settle();
    assert!(h.app.dfm_review.dragging.is_none(), "the drag ended");
    assert!(h.app.agent_chat.attachments.is_empty(), "{:?}", h.app.agent_chat.attachments);
}
