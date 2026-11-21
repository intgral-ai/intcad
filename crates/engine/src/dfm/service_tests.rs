//! The manual host entry end to end: a saved project design, `dfm.check` on the window thread,
//! the worker, and `inspect.dfm` after it finishes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::service;
use crate::dfm::run::RunStore;

/// The service keeps process-wide roots, so the one end-to-end test runs alone.
static GUARD: Mutex<()> = Mutex::new(());

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("solidcraft-dfm-service-{tag}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn sheet_design(session: &mut crate::Session) {
    session.execute("sketch.create", &serde_json::json!({"plane": "XY", "name": "Base"})).unwrap();
    session.execute("sketch.rectangle.two_point", &serde_json::json!({"p0": [0, 0], "p1": [40, 25]})).unwrap();
    session.execute("sketch.finish", &serde_json::json!({})).unwrap();
    session.execute("sheet.flange", &serde_json::json!({"sketch": "Base"})).unwrap();
}

#[test]
fn a_manual_check_without_ai_writes_a_report_that_inspect_reopens() {
    let _guard = GUARD.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = scratch("manual");
    let candidates = root.join("candidates");
    let validation = root.join("validation");
    crate::cmd::set_candidate_store(Some(candidates));
    service::set_store_root(Some(validation));

    let created = crate::project::create(&root, "Bracket", "Bracket").unwrap();
    let mut session = crate::Session::default();
    sheet_design(&mut session);
    session.execute("file.save_as", &serde_json::json!({"path": created.design.to_string_lossy(), "overwrite": true})).unwrap();
    assert!(session.path.is_some(), "the session is saved in the project");

    let ticket = session.execute("dfm.check", &serde_json::json!({})).unwrap();
    assert_eq!(ticket["sheets"], 1, "one native sheet body");
    assert_eq!(ticket["projectId"], created.project.manifest().id);

    let deadline = Instant::now() + Duration::from_secs(60);
    let mut status = service::status();
    while status.running || (status.last_run.is_none() && status.error.is_none()) {
        assert!(Instant::now() < deadline, "the check did not finish: {status:?}");
        std::thread::sleep(Duration::from_millis(10));
        status = service::status();
    }
    assert!(status.error.is_none(), "{status:?}");
    assert_eq!(status.last_outcome.as_deref(), Some("current"), "{status:?}");
    let run_id = status.last_run.expect("a finished run id");

    let report = session.execute("inspect.dfm", &serde_json::json!({})).unwrap();
    assert_eq!(report["available"], true);
    assert_eq!(report["runId"], run_id);
    assert_eq!(report["outcome"], "current");
    assert_eq!(report["freshness"], "current", "the same saved revision is current");
    let findings = &report["findings"];
    assert_eq!(findings["schemaVersion"], 2);
    assert_eq!(findings["runId"], run_id);
    assert!(findings["findings"].as_array().is_some_and(|rows| !rows.is_empty()), "the thickness gate is a finding");
    assert!(
        findings["rules"].as_array().is_some_and(|rows| rows.iter().any(|row| row["ruleId"] == "app.flat_no_overlap")),
        "the App's flat self-overlap check runs in the same report"
    );
    assert!(
        findings["findings"].as_array().is_some_and(|rows| rows
            .iter()
            .any(|row| row["rules"].as_array().is_some_and(|rules| rules.iter().any(|rule| rule == "app.flat_no_overlap")))),
        "a flat-overlap result is part of the findings"
    );
    assert!(report["summary"].as_str().is_some_and(|summary| summary.contains("Not checked")));

    // Reopening the project finds the same run and the same findings, with no AI connection.
    let store = RunStore::new(created.project.root());
    let latest = store.latest().unwrap().expect("latest");
    assert_eq!(latest.run_id, run_id);
    let saved = store.load_findings(&run_id).unwrap();
    assert_eq!(serde_json::to_value(&saved).unwrap(), *findings);
    assert_eq!(store.load_meta(&run_id).unwrap().outcome, crate::dfm::run::RunOutcome::Current);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn controlled_starts_never_replace_an_active_controlled_run() {
    use super::service::{StartPlan, start_plan};
    assert_eq!(start_plan(false, false, true), Ok(StartPlan::Start));
    assert_eq!(start_plan(false, false, false), Ok(StartPlan::Start));
    assert_eq!(start_plan(true, false, false), Ok(StartPlan::ReplaceFree));
    assert!(start_plan(true, true, false).is_err(), "a free start never replaces an active controlled run");
    assert!(start_plan(true, true, true).is_err(), "a controlled start never replaces an active controlled run");
    assert!(start_plan(true, false, true).is_err(), "a controlled start never replaces an active free run");
}
