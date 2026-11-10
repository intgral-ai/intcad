//! Real, headless CAD ports for the native AI harness task controller.
//!
//! This crate is UI-independent: it uses the engine session, the existing snapshot command and the
//! existing `ValidationPort`. Both the native Chat adapter (`ui-egui`) and a headless test/CLI can
//! drive the same `TaskController` through these ports; neither owns a second agent loop, provider,
//! session or tool registry.
#![cfg(not(target_arch = "wasm32"))]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use solidcraft_agent_task as task;
use solidcraft_agent_task::{ToolPort as _, ValidationPort as _};
use solidcraft_engine::Session;
use solidcraft_engine::schema::snapshot::CandidateRef;
use solidcraft_engine::validation::{
    CheckProfile, LiveDocument, MeasurementRequirement, ParameterRequirement, SkillValidationRequest, ValidationPort as EngineValidationPort,
    request_key, text_to_cad_profile, text_to_cad_profile_ref, validate_skill_snapshot,
};

#[cfg(test)]
/// The candidate store is a process-global host setting; every test that uses it serializes here,
/// including tests in different modules of this binary.
static SNAPSHOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

/// The CAD ports over one live engine session and one ValidationPort. The session is the host's
/// existing document; the controller never opens a file or a second document.
pub struct EngineTaskPorts {
    session: Session,
    validation: EngineTaskValidation,
}

/// The trusted validation half, sharing the snapshot lookup with the tool half.
pub struct EngineTaskValidation {
    port: EngineValidationPort,
    snapshots: Arc<Mutex<BTreeMap<String, CandidateRef>>>,
    live: Arc<Mutex<Option<task::DocumentRef>>>,
    profile: CheckProfile,
    core_profile: task::ValidationProfileRef,
}

/// The CAD tool half, borrowing the live session for one host turn.
pub struct EngineTaskTools<'a> {
    session: &'a mut Session,
    snapshots: Arc<Mutex<BTreeMap<String, CandidateRef>>>,
    live: Arc<Mutex<Option<task::DocumentRef>>>,
}

impl EngineTaskPorts {
    /// Open the ports and freeze the host-owned #65 profile identity. The candidate root is the
    /// snapshot store; the validation store is the host's own.
    pub fn open(
        session: Session,
        candidate_root: std::path::PathBuf,
        validation_store: std::path::PathBuf,
        profile: CheckProfile,
    ) -> Result<Self, String> {
        let port = EngineValidationPort::open(validation_store, candidate_root).map_err(|e| e.to_string())?;
        let engine_profile = text_to_cad_profile_ref(&profile).map_err(|e| e.to_string())?;
        let core_profile = task::ValidationProfileRef {
            profile_id: engine_profile.profile_id.clone(),
            version: engine_profile.version.clone(),
            digest: engine_profile.digest.clone(),
        };
        Ok(Self {
            session,
            validation: EngineTaskValidation {
                port,
                snapshots: Arc::new(Mutex::new(BTreeMap::new())),
                live: Arc::new(Mutex::new(None)),
                profile,
                core_profile,
            },
        })
    }

    pub fn profile(&self) -> &task::ValidationProfileRef {
        &self.validation.core_profile
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Run one host command on the live session. Host code uses the same command path as the
    /// control channel; the task controller only decides whether it may run.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        self.session.execute(id, params).map_err(|e| e.to_string())
    }

    /// Run a write through the engine's guarded executor: the command only runs when the exact
    /// document id and revision still match the host's expectation, and an undoable command
    /// records its usual one-step checkpoint. A changed document fails before anything runs.
    pub fn apply_expected(&mut self, id: &str, params: &Value, expected: &task::DocumentRef) -> Result<task::DocumentRef, String> {
        let stamp = solidcraft_engine::DocumentStamp { document_id: expected.document_id.clone(), revision: expected.version };
        match self.session.execute_if_current(id, params, &stamp) {
            Ok((_value, after)) => Ok(task::DocumentRef { project_id: "local".into(), document_id: after.document_id, version: after.revision }),
            Err(solidcraft_engine::ConditionedError::Changed { .. }) => Err("stale document: the expected revision is no longer current".into()),
            Err(solidcraft_engine::ConditionedError::Command(error)) => Err(error.to_string()),
        }
    }

    /// Split the two ports for a turn without aliasing the session and the validation state.
    pub fn with_parts<T>(&mut self, run: impl FnOnce(&mut EngineTaskTools<'_>, &mut EngineTaskValidation) -> T) -> T {
        let validation = &mut self.validation;
        let snapshots = Arc::clone(&validation.snapshots);
        let live = Arc::clone(&validation.live);
        let mut tools = EngineTaskTools { session: &mut self.session, snapshots, live };
        run(&mut tools, validation)
    }
}

impl EngineTaskTools<'_> {
    fn remember_live(&self, document: &task::DocumentRef) {
        if let Ok(mut live) = self.live.lock() {
            *live = Some(document.clone());
        }
    }

    /// Capture a snapshot from the live design through the existing command. The returned id is
    /// stored so the validation half can verify the exact candidate bytes.
    fn capture(&mut self, purpose: task::SnapshotPurpose) -> Result<task::SnapshotRef, task::PortError> {
        let reply = self.session.execute("document.snapshot.capture", &json!({})).map_err(|e| task::PortError::new(e.to_string()))?;
        let value = reply.get("candidate").cloned().ok_or_else(|| task::PortError::new("the snapshot reply has no candidate"))?;
        let candidate: CandidateRef =
            serde_json::from_value(value).map_err(|e| task::PortError::new(format!("the snapshot candidate is invalid: {e}")))?;
        let document = task::DocumentRef {
            project_id: "local".into(),
            document_id: candidate.source.document_id.clone(),
            version: candidate.source.revision.parse::<u64>().unwrap_or(0),
        };
        if let Ok(mut snapshots) = self.snapshots.lock() {
            snapshots.insert(candidate.candidate_id.clone(), candidate.clone());
        }
        self.remember_live(&document);
        Ok(task::SnapshotRef { id: candidate.candidate_id, document, digest: candidate.candidate_digest, purpose })
    }
}

impl task::ToolPort for EngineTaskTools<'_> {
    fn inspect(&mut self, _document: &task::DocumentRef) -> Result<String, task::PortError> {
        let reply = self.session.execute("document.inspect", &json!({"measure": false})).map_err(|e| task::PortError::new(e.to_string()))?;
        let text = reply.to_string();
        Ok(text.chars().take(2000).collect())
    }

    fn current_document(&mut self) -> Result<task::DocumentRef, task::PortError> {
        let stamp = self.session.document_stamp();
        let document = task::DocumentRef { project_id: "local".into(), document_id: stamp.document_id, version: stamp.revision };
        self.remember_live(&document);
        Ok(document)
    }

    fn capture_snapshot(&mut self, _document: &task::DocumentRef, purpose: task::SnapshotPurpose) -> Result<task::SnapshotRef, task::PortError> {
        self.capture(purpose)
    }

    fn run_checks(&mut self, _document: &task::DocumentRef, _required: &[String]) -> Result<Vec<task::CheckResult>, task::PortError> {
        // #65's versioned profile owns every required check and produces them from the immutable
        // snapshot. Returning empty host probes here is deliberate: this adapter never fabricates
        // `passed: true` for a check it did not run. The controller reads the real checks from the
        // validated report.
        Ok(Vec::new())
    }
}

impl task::ValidationPort for EngineTaskValidation {
    fn validate(&mut self, snapshot: &task::SnapshotRef, profile: &task::ValidationProfileRef) -> Result<task::ValidationReport, task::PortError> {
        if profile != &self.core_profile {
            return Err(task::PortError::new("the requested profile is not the host-owned #65 profile"));
        }
        let candidate = self
            .snapshots
            .lock()
            .map_err(|_| task::PortError::new("the snapshot lookup is poisoned"))?
            .get(&snapshot.id)
            .cloned()
            .ok_or_else(|| task::PortError::new("the validation snapshot is not in this host's snapshot store"))?;
        let live = self
            .live
            .lock()
            .map_err(|_| task::PortError::new("the live document lookup is poisoned"))?
            .clone()
            .map(|document| LiveDocument::new(document.document_id, document.version));
        let mut request = SkillValidationRequest::new(&candidate, self.profile.clone(), request_key(now_ms()));
        if let Some(live) = live {
            request = request.with_live(live);
        }
        let report = validate_skill_snapshot(&self.port, request).map_err(|e| task::PortError::new(e.to_string()))?;
        let checks = report
            .checks
            .iter()
            .map(|check| task::CheckResult { check_id: check.check_id.clone(), passed: check.passed, detail: check.state.clone() })
            .collect();
        Ok(task::ValidationReport {
            run_id: report.run_id,
            profile: task::ValidationProfileRef {
                profile_id: report.profile.profile_id,
                version: report.profile.version,
                digest: report.profile.digest,
            },
            snapshot: snapshot.clone(),
            document: snapshot.document.clone(),
            // Only #65's approved report is success: the exact snapshot, live revision, profile
            // and every required check must match. A snapshot-only pass is evidence, not approval.
            passed: report.approved,
            checks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("solidcraft-agent-task-engine-{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The CTO acceptance model: an L-shaped bracket 5 mm thick with four mounting holes. Its
    /// area is 1800 mm²; four 5 mm holes remove 4π(2.5)² mm².
    fn bracket(thickness: &str) -> Session {
        let mut s = Session::default();
        s.execute("parameters.add", &json!({"name": "thickness", "expression": thickness})).unwrap();
        s.execute("sketch.create", &json!({"plane": "XY", "name": "Profile"})).unwrap();
        s.execute("sketch.line", &json!({"points": [[0, 0], [60, 0], [60, 40], [30, 40], [30, 20], [0, 20]], "closed": true})).unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute("solid.extrude", &json!({"distance": "thickness", "body_name": "Bracket"})).unwrap();
        s.execute("sketch.create", &json!({"plane": "XY", "name": "Mounting holes"})).unwrap();
        for (x, y) in [(10.0, 10.0), (20.0, 10.0), (40.0, 10.0), (50.0, 10.0)] {
            s.execute("sketch.point", &json!({"point": [x, y]})).unwrap();
        }
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute("solid.hole", &json!({"sketch": "Mounting holes", "diameter": 2})).unwrap();
        s
    }

    fn expected_volume(thickness: f64) -> f64 {
        let area = 1800.0 - 4.0 * std::f64::consts::PI;
        area * thickness
    }

    fn measured_volume(s: &mut Session) -> f64 {
        let measured = s.execute("inspect.measure", &json!({})).unwrap();
        measured.pointer("/bodies/0/volume_mm3").and_then(Value::as_f64).expect("a closed bracket body")
    }

    /// The four hole points stay in one parametric Hole feature after the thickness change.
    fn mounting_hole_points(s: &Session) -> usize {
        s.doc
            .features
            .iter()
            .find_map(|feature| match &feature.kind {
                solidcraft_engine::doc::FeatureKind::Hole { points: Some(points), .. } => Some(points.ids.len()),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn profile(expect_thickness: f64) -> CheckProfile {
        l_bracket_profile(expect_thickness).unwrap()
    }

    fn permission() -> task::PermissionContext {
        task::PermissionContext { principal: "test".into(), approval_mode: task::ApprovalMode::Ask, allow_list: Vec::new(), turn_valid: true }
    }

    fn doc_of(s: &mut Session) -> task::DocumentRef {
        let stamp = s.document_stamp();
        task::DocumentRef { project_id: "local".into(), document_id: stamp.document_id, version: stamp.revision }
    }

    fn spec(document: task::DocumentRef, repairs: u32) -> task::TaskSpec {
        task::TaskSpec {
            skill_id: task::TEXT_TO_CAD_ID.into(),
            input: "Create an L-shaped bracket 5 mm thick with four 2 mm mounting holes; change the thickness to 8 mm".into(),
            document,
            selection: Vec::new(),
            requirements: None,
            budget: task::Budget::finite(4, 16, repairs, 120_000),
        }
    }

    fn open_ports(mut session: Session, name: &str, expect_thickness: f64) -> (EngineTaskPorts, task::DocumentRef, task::skill::SkillRegistry) {
        let rid = scratch(name);
        let candidates = rid.join("candidates");
        let store = rid.join("validation");
        solidcraft_engine::cmd::set_candidate_store(Some(candidates.clone()));
        let document = doc_of(&mut session);
        let ports = EngineTaskPorts::open(session, candidates, store, profile(expect_thickness)).unwrap();
        let registry = task::skill::builtins(ports.profile().clone(), task::Budget::finite(4, 16, 2, 120_000)).unwrap();
        (ports, document, registry)
    }

    fn headless_controller(registry: &task::skill::SkillRegistry, document: task::DocumentRef, repairs: u32) -> task::TaskController {
        task::TaskController::start(
            registry,
            spec(document, repairs),
            permission(),
            Arc::new(task::SystemClock),
            Box::new(task::InMemoryStore::new()),
        )
        .unwrap()
    }

    #[test]
    fn the_real_bracket_is_5_mm_and_four_holes_are_parametric() {
        let mut s = bracket("5 mm");
        assert!((measured_volume(&mut s) - expected_volume(5.0)).abs() < 1e-3);
        assert_eq!(mounting_hole_points(&s), 4, "four parametric hole points");
        let design = s.execute("document.inspect", &json!({"measure": true})).unwrap();
        let text = design.to_string();
        assert!(text.contains("Mounting holes") || text.contains("Hole") || text.contains("holes"), "{text}");
        s.execute("parameters.change", &json!({"name": "thickness", "expression": "8 mm"})).unwrap();
        assert!((measured_volume(&mut s) - expected_volume(8.0)).abs() < 1e-3);
        assert_eq!(mounting_hole_points(&s), 4, "the same four holes survive the rebuild");
        assert_eq!(s.doc.param("thickness").map(|p| p.expr.as_str()), Some("8 mm"));
    }

    /// A completely headless invocation of the same TaskController and real engine ports:
    /// inspect -> snapshot -> apply -> check -> host validation -> terminal. No egui.
    #[test]
    fn headless_controller_runs_the_real_bracket_and_validates_it() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let session = bracket("5 mm");
        let (mut ports, document, registry) = open_ports(session, "bracket-e2e", 8.0);
        let mut controller = headless_controller(&registry, document, 1);
        ports.with_parts(|tools, _| controller.begin(tools)).unwrap();

        let decision = ports.with_parts(|tools, _| controller.on_tool_request("inspect_design", &json!({}), tools));
        assert_eq!(decision, task::ToolDecision::Allow);
        let change = json!({"name": "thickness", "expression": "8 mm"});
        let decision =
            ports.with_parts(|tools, _| controller.on_tool_request("execute", &json!({"command": "parameters.change", "params": change}), tools));
        assert_eq!(decision, task::ToolDecision::Allow);
        let expected = controller.expected_document();
        ports.apply_expected("parameters.change", &change, &expected).unwrap();
        controller.on_tool_result("execute", task::ToolOutcome::Completed, "thickness changed");

        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
        assert_eq!(result.terminal, task::TaskTerminal::ValidatedSuccess, "{result:?}");
        assert!(result.artifacts.iter().any(|a| a.kind == "pre_apply_snapshot"));
        assert!(result.artifacts.iter().any(|a| a.kind == "validation_snapshot"));
        assert!(result.artifacts.iter().any(|a| a.kind == "validation_report"));

        assert_eq!(ports.session().doc.param("thickness").map(|p| p.expr.as_str()), Some("8 mm"));
        let measured = ports.execute("inspect.measure", &json!({})).unwrap();
        let volume = measured.pointer("/bodies/0/volume_mm3").and_then(Value::as_f64).expect("a closed bracket body");
        assert!((volume - expected_volume(8.0)).abs() < 1e-3);
        assert_eq!(mounting_hole_points(ports.session()), 4, "the four holes are still parametric");
        // The task's one write is a normal undo step; undo restores the pre-task thickness.
        let final_document = controller.final_document().cloned().expect("a final document");
        ports.apply_expected("edit.undo", &json!({}), &final_document).unwrap();
        assert_eq!(ports.session().doc.param("thickness").map(|p| p.expr.as_str()), Some("5 mm"));
        let events = controller.events();
        assert!(events.iter().any(|e| matches!(e, task::TaskEvent::Snapshot { snapshot } if snapshot.purpose == task::SnapshotPurpose::PreApply)));
        assert!(events.iter().any(|e| matches!(e, task::TaskEvent::Snapshot { snapshot } if snapshot.purpose == task::SnapshotPurpose::Validation)));
        assert!(events.iter().any(|e| matches!(e, task::TaskEvent::ValidationReport { report } if report.passed)));
    }

    #[test]
    fn headless_repair_round_passes_after_a_failed_first_write() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let session = bracket("5 mm");
        let (mut ports, document, registry) = open_ports(session, "bracket-repair", 8.0);
        let mut controller = headless_controller(&registry, document, 1);
        ports.with_parts(|tools, _| controller.begin(tools)).unwrap();
        ports.with_parts(|tools, _| controller.on_tool_request("inspect_design", &json!({}), tools));
        // First turn: a real CAD write, but the wrong thickness. Trusted validation fails.
        let six = json!({"name": "thickness", "expression": "6 mm"});
        assert_eq!(
            ports.with_parts(|tools, _| controller.on_tool_request("execute", &json!({"command": "parameters.change", "params": six}), tools)),
            task::ToolDecision::Allow
        );
        let expected = controller.expected_document();
        ports.apply_expected("parameters.change", &six, &expected).unwrap();
        controller.on_tool_result("execute", task::ToolOutcome::Completed, "thickness 6 mm");
        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Repair { prompt } = action else { panic!("expected a bounded repair, got {action:?}") };
        assert!(prompt.contains("repair round 1"));
        // Repair turn: a new explicit plan, the same backend/controller/ports, the requested value.
        controller.on_turn_start().unwrap();
        let eight = json!({"name": "thickness", "expression": "8 mm"});
        assert_eq!(
            ports.with_parts(|tools, _| controller.on_tool_request("execute", &json!({"command": "parameters.change", "params": eight}), tools)),
            task::ToolDecision::Allow
        );
        let expected = controller.expected_document();
        ports.apply_expected("parameters.change", &eight, &expected).unwrap();
        controller.on_tool_result("execute", task::ToolOutcome::Completed, "thickness 8 mm");
        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
        assert_eq!(result.terminal, task::TaskTerminal::ValidatedSuccess, "{result:?}");
        assert_eq!(ports.session().doc.param("thickness").map(|p| p.expr.as_str()), Some("8 mm"));
        let measured = ports.execute("inspect.measure", &json!({})).unwrap();
        let volume = measured.pointer("/bodies/0/volume_mm3").and_then(Value::as_f64).expect("a closed bracket body");
        assert!((volume - expected_volume(8.0)).abs() < 1e-3);
    }

    #[test]
    fn headless_validation_failure_is_not_success() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let session = bracket("5 mm");
        let (mut ports, document, registry) = open_ports(session, "bracket-mismatch", 8.0);
        let mut controller = headless_controller(&registry, document, 0);
        ports.with_parts(|tools, _| controller.begin(tools)).unwrap();
        ports.with_parts(|tools, _| controller.on_tool_request("inspect_design", &json!({}), tools));
        let decision = ports.with_parts(|tools, _| controller.on_tool_request("execute", &json!({"command": "solid.box"}), tools));
        assert_eq!(decision, task::ToolDecision::Allow);
        let expected = controller.expected_document();
        ports.apply_expected("solid.box", &json!({"length": 1, "width": 1, "height": 1}), &expected).unwrap();
        controller.on_tool_result("execute", task::ToolOutcome::Completed, "a box, not the requested thickness");
        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
        assert_ne!(result.terminal, task::TaskTerminal::ValidatedSuccess);
        assert_eq!(result.terminal, task::TaskTerminal::QualityFailed, "{result:?}");
    }

    #[test]
    fn headless_cancel_blocks_the_write_and_never_succeeds() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let session = bracket("5 mm");
        let (mut ports, document, registry) = open_ports(session, "bracket-cancel", 8.0);
        let mut controller = headless_controller(&registry, document, 1);
        ports.with_parts(|tools, _| controller.begin(tools)).unwrap();
        controller.on_cancel("the user stopped the message");
        let decision = ports.with_parts(|tools, _| {
            controller.on_tool_request(
                "execute",
                &json!({"command": "parameters.change", "params": {"name": "thickness", "expression": "8 mm"}}),
                tools,
            )
        });
        assert!(matches!(decision, task::ToolDecision::Deny(_)));
        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal") };
        assert_eq!(result.terminal, task::TaskTerminal::Cancelled);
        assert_eq!(ports.session().doc.param("thickness").map(|p| p.expr.as_str()), Some("5 mm"));
    }

    #[test]
    fn headless_stale_document_before_the_first_write_is_refused() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let session = bracket("5 mm");
        let (mut ports, document, registry) = open_ports(session, "bracket-stale", 8.0);
        let mut controller = headless_controller(&registry, document, 0);
        ports.with_parts(|tools, _| controller.begin(tools)).unwrap();
        ports.with_parts(|tools, _| controller.on_tool_request("inspect_design", &json!({}), tools));
        // Someone else changes the live design before the task's first write.
        ports.execute("solid.box", &json!({"length": 1, "width": 1, "height": 1})).unwrap();
        let decision = ports.with_parts(|tools, _| {
            controller.on_tool_request(
                "execute",
                &json!({"command": "parameters.change", "params": {"name": "thickness", "expression": "8 mm"}}),
                tools,
            )
        });
        assert!(matches!(decision, task::ToolDecision::Deny(reason) if reason.contains("stale")));
        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal") };
        assert_eq!(result.terminal, task::TaskTerminal::Incomplete);
        assert_eq!(ports.session().doc.param("thickness").map(|p| p.expr.as_str()), Some("5 mm"));
    }

    #[test]
    fn headless_unknown_write_is_terminal_unknown_and_not_replayed() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let session = bracket("5 mm");
        let (mut ports, document, registry) = open_ports(session, "bracket-unknown", 8.0);
        let mut controller = headless_controller(&registry, document, 1);
        ports.with_parts(|tools, _| controller.begin(tools)).unwrap();
        ports.with_parts(|tools, _| controller.on_tool_request("inspect_design", &json!({}), tools));
        assert_eq!(
            ports.with_parts(|tools, _| controller.on_tool_request(
                "execute",
                &json!({"command": "parameters.change", "params": {"name": "thickness", "expression": "8 mm"}}),
                tools
            )),
            task::ToolDecision::Allow
        );
        controller.on_tool_result("execute", task::ToolOutcome::Unknown, "the control reply was lost");
        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal") };
        assert_eq!(result.terminal, task::TaskTerminal::Unknown);
        assert_eq!(result.unknown_writes, 1);
    }

    #[test]
    fn headless_wall_time_expiry_is_not_success() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let session = bracket("5 mm");
        let (mut ports, document, registry) = open_ports(session, "bracket-wall", 8.0);
        let clock = Arc::new(task::ManualClock::new(1_760_000_000_000));
        let mut controller =
            task::TaskController::start(&registry, spec(document, 0), permission(), clock.clone(), Box::new(task::InMemoryStore::new())).unwrap();
        ports.with_parts(|tools, _| controller.begin(tools)).unwrap();
        ports.with_parts(|tools, _| controller.on_tool_request("inspect_design", &json!({}), tools));
        assert_eq!(
            ports.with_parts(|tools, _| controller.on_tool_request("execute", &json!({"command": "solid.box"}), tools)),
            task::ToolDecision::Allow
        );
        let expected = controller.expected_document();
        ports.apply_expected("solid.box", &json!({"length": 1, "width": 1, "height": 1}), &expected).unwrap();
        controller.on_tool_result("execute", task::ToolOutcome::Completed, "a box");
        clock.advance(120_001);
        let action = ports.with_parts(|tools, validation| controller.on_turn_end(tools, validation));
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal") };
        assert_eq!(result.terminal, task::TaskTerminal::Error);
        assert!(result.reason.contains("wall-time"));
    }
}

/// The Beta Text-to-CAD profile identity, from #65's fixed contract.
pub const BETA_PROFILE_ID: &str = solidcraft_engine::validation::TEXT_TO_CAD_PROFILE_ID;
/// The Beta Text-to-CAD profile version, from #65's fixed contract.
pub const BETA_PROFILE_VERSION: &str = solidcraft_engine::validation::TEXT_TO_CAD_PROFILE_VERSION;

/// The L-bracket acceptance profile: four 2 mm holes in the 1800 mm² L profile, at the requested
/// thickness. This is #65's fixed Beta profile, not a parallel report type.
pub fn l_bracket_profile(thickness_mm: f64) -> Result<CheckProfile, String> {
    let area = 1800.0 - 4.0 * std::f64::consts::PI;
    text_to_cad_profile(
        vec![ParameterRequirement { name: "thickness".into(), value: thickness_mm, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/bodies/0/volume_mm3".into(),
            expected: area * thickness_mm,
            abs_tolerance: 0.05,
            rel_tolerance: 1e-4,
        }],
    )
    .map_err(|error| error.to_string())
}

/// The host's explicit revalidation of the live revision (after a manual parameter change).
/// It is a validation result, never model prose.
pub struct Revalidation {
    pub document: task::DocumentRef,
    pub snapshot: task::SnapshotRef,
    pub report: task::ValidationReport,
    pub passed: bool,
}

/// A host task bound to the live session. The UI passes the one session it owns to each call;
/// the controller never opens a second document and the validation half reads the immutable
/// snapshot it captured.
pub struct LiveTask {
    controller: task::TaskController,
    validation: EngineTaskValidation,
    snapshots: Arc<Mutex<BTreeMap<String, CandidateRef>>>,
    live: Arc<Mutex<Option<task::DocumentRef>>>,
    profile_ref: task::ValidationProfileRef,
}

impl LiveTask {
    /// Open a task over the live session. The caller supplies the frozen spec, permission and
    /// host-owned #65 profile; nothing here is derived from model prose.
    pub fn start(
        session: &mut Session,
        candidate_root: std::path::PathBuf,
        validation_store: std::path::PathBuf,
        profile: CheckProfile,
        spec: task::TaskSpec,
        permission: task::PermissionContext,
    ) -> Result<Self, String> {
        let port = EngineValidationPort::open(validation_store, candidate_root).map_err(|e| e.to_string())?;
        let engine_profile = text_to_cad_profile_ref(&profile).map_err(|e| e.to_string())?;
        let core_profile =
            task::ValidationProfileRef { profile_id: engine_profile.profile_id, version: engine_profile.version, digest: engine_profile.digest };
        let snapshots = Arc::new(Mutex::new(BTreeMap::new()));
        let live = Arc::new(Mutex::new(None));
        let validation = EngineTaskValidation {
            port,
            snapshots: Arc::clone(&snapshots),
            live: Arc::clone(&live),
            profile: profile.clone(),
            core_profile: core_profile.clone(),
        };
        let registry = task::skill::builtins(core_profile.clone(), spec.budget.clone()).map_err(|e| e.to_string())?;
        let controller = task::TaskController::start(&registry, spec, permission, Arc::new(task::SystemClock), Box::new(task::InMemoryStore::new()))
            .map_err(|e| e.to_string())?;
        let mut task = Self { controller, validation, snapshots, live, profile_ref: core_profile };
        task.begin(session)?;
        Ok(task)
    }

    fn tools<'a>(&self, session: &'a mut Session) -> EngineTaskTools<'a> {
        EngineTaskTools { session, snapshots: Arc::clone(&self.snapshots), live: Arc::clone(&self.live) }
    }

    pub fn begin(&mut self, session: &mut Session) -> Result<(), String> {
        let mut tools = self.tools(session);
        self.controller.begin(&mut tools).map_err(|e| e.to_string())
    }

    pub fn on_tool_request(&mut self, session: &mut Session, tool: &str, arguments: &Value) -> task::ToolDecision {
        let mut tools = self.tools(session);
        self.controller.on_tool_request(tool, arguments, &mut tools)
    }

    pub fn on_tool_result(&mut self, tool: &str, outcome: task::ToolOutcome, detail: &str) {
        self.controller.on_tool_result(tool, outcome, detail);
    }

    /// The host's exact classification of one request, so the adapter can tell a read from a
    /// write without a second tool catalog.
    pub fn classify(&self, tool: &str, arguments: &Value) -> task::ToolKind {
        self.controller.skill().classify(tool, arguments)
    }

    /// Account for a new model turn before the host prompts the same native backend.
    pub fn on_turn_start(&mut self) -> Result<(), String> {
        self.controller.on_turn_start().map_err(|error| error.to_string())
    }

    /// Capture a fresh result snapshot at the host boundary and record it as task evidence. It
    /// is deliberately not the controller's validation snapshot: more writes may follow in the
    /// same turn, and the controller owns the final validation capture itself.
    pub fn capture_result(&mut self, session: &mut Session) -> Result<task::SnapshotRef, String> {
        let mut tools = self.tools(session);
        let current = tools.current_document().map_err(|error| error.to_string())?;
        let snapshot = tools.capture_snapshot(&current, task::SnapshotPurpose::Validation).map_err(|error| error.to_string())?;
        self.controller.record_host_snapshot(snapshot.clone());
        Ok(snapshot)
    }

    /// Validate the live revision explicitly, after a person changed a parameter by hand. The
    /// controller's task is terminal by then, so this is a separate host validation over the
    /// same profile and snapshot store, not a model turn and not a second agent loop.
    pub fn revalidate(&mut self, session: &mut Session) -> Result<Revalidation, String> {
        let required = self.controller.skill().required_checks.clone();
        let profile = self.profile_ref.clone();
        let Self { validation, snapshots, live, .. } = self;
        let mut tools = EngineTaskTools { session, snapshots: Arc::clone(snapshots), live: Arc::clone(live) };
        let current = tools.current_document().map_err(|error| error.to_string())?;
        let snapshot = tools.capture_snapshot(&current, task::SnapshotPurpose::Validation).map_err(|error| error.to_string())?;
        let report = validation.validate(&snapshot, &profile).map_err(|error| error.to_string())?;
        let passed = report.passed
            && report.profile == profile
            && report.document == current
            && report.snapshot.document == current
            && report.snapshot.id == snapshot.id
            && required.iter().all(|required| report.checks.iter().any(|check| &check.check_id == required && check.passed));
        Ok(Revalidation { document: current, snapshot, report, passed })
    }

    /// Finish a host-aborted turn as incomplete with an explicit budget reason.
    pub fn stop_incomplete(&mut self, reason: &str) -> task::TaskResult {
        self.controller.stop_incomplete(reason)
    }

    pub fn on_turn_end(&mut self, session: &mut Session) -> task::TurnAction {
        let Self { controller, validation, snapshots, live, .. } = self;
        let mut tools = EngineTaskTools { session, snapshots: Arc::clone(snapshots), live: Arc::clone(live) };
        controller.on_turn_end(&mut tools, validation)
    }

    pub fn on_cancel(&mut self, reason: &str) {
        self.controller.on_cancel(reason);
    }

    pub fn set_turn_valid(&mut self, valid: bool) {
        self.controller.set_turn_valid(valid);
    }

    pub fn result(&self) -> Option<&task::TaskResult> {
        self.controller.result()
    }

    pub fn events(&self) -> Vec<task::TaskEvent> {
        self.controller.events()
    }

    pub fn final_document(&self) -> Option<&task::DocumentRef> {
        self.controller.final_document()
    }

    pub fn expected_document(&self) -> task::DocumentRef {
        self.controller.expected_document()
    }

    pub fn profile(&self) -> &task::ValidationProfileRef {
        &self.profile_ref
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use solidcraft_engine::doc::FeatureKind;

    fn scratch(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("solidcraft-live-task-{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bracket(thickness: &str) -> Session {
        let mut s = Session::default();
        s.execute("parameters.add", &json!({"name": "thickness", "expression": thickness})).unwrap();
        s.execute("sketch.create", &json!({"plane": "XY", "name": "Profile"})).unwrap();
        s.execute("sketch.line", &json!({"points": [[0, 0], [60, 0], [60, 40], [30, 40], [30, 20], [0, 20]], "closed": true})).unwrap();
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute("solid.extrude", &json!({"distance": "thickness", "body_name": "Bracket"})).unwrap();
        s.execute("sketch.create", &json!({"plane": "XY", "name": "Mounting holes"})).unwrap();
        for (x, y) in [(10.0, 10.0), (20.0, 10.0), (40.0, 10.0), (50.0, 10.0)] {
            s.execute("sketch.point", &json!({"point": [x, y]})).unwrap();
        }
        s.execute("sketch.finish", &json!({})).unwrap();
        s.execute("solid.hole", &json!({"sketch": "Mounting holes", "diameter": 2})).unwrap();
        s
    }

    fn doc_of(s: &mut Session) -> task::DocumentRef {
        let stamp = s.document_stamp();
        task::DocumentRef { project_id: "local".into(), document_id: stamp.document_id, version: stamp.revision }
    }

    fn spec(document: task::DocumentRef, repairs: u32) -> task::TaskSpec {
        task::TaskSpec {
            skill_id: task::TEXT_TO_CAD_ID.into(),
            input: "Create an L bracket 5 mm thick with four 2 mm holes; change the thickness to 8 mm".into(),
            document,
            selection: Vec::new(),
            requirements: None,
            budget: task::Budget::finite(4, 16, repairs, 120_000),
        }
    }

    fn permission() -> task::PermissionContext {
        task::PermissionContext { principal: "test".into(), approval_mode: task::ApprovalMode::Ask, allow_list: Vec::new(), turn_valid: true }
    }

    fn open(session: &mut Session, name: &str, thickness: f64, repairs: u32) -> (LiveTask, std::path::PathBuf) {
        let root = scratch(name);
        let candidates = root.join("candidates");
        let store = root.join("validation");
        solidcraft_engine::cmd::set_candidate_store(Some(candidates.clone()));
        let document = doc_of(session);
        let profile = l_bracket_profile(thickness).unwrap();
        let task = LiveTask::start(session, candidates, store, profile, spec(document, repairs), permission()).unwrap();
        (task, root)
    }

    fn write(task: &mut LiveTask, session: &mut Session, thickness: &str) -> task::TurnAction {
        let change = json!({"name": "thickness", "expression": thickness});
        assert_eq!(task.on_tool_request(session, "execute", &json!({"command": "parameters.change", "params": change})), task::ToolDecision::Allow);
        let expected = task.expected_document();
        session
            .execute_if_current(
                "parameters.change",
                &change,
                &solidcraft_engine::DocumentStamp { document_id: expected.document_id, revision: expected.version },
            )
            .unwrap();
        task.on_tool_result("execute", task::ToolOutcome::Completed, "thickness changed");
        // The host boundary captures a fresh result snapshot after every completed write or
        // repair candidate; the controller then captures its own validation snapshot at turn end.
        task.capture_result(session).expect("a result snapshot");
        task.on_turn_end(session)
    }

    #[test]
    fn live_task_creates_and_revalidates_the_8_mm_bracket() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let mut session = bracket("5 mm");
        let (mut task, root) = open(&mut session, "create", 8.0, 1);
        let action = write(&mut task, &mut session, "8 mm");
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
        assert_eq!(result.terminal, task::TaskTerminal::ValidatedSuccess, "{result:?}");
        assert!(result.artifacts.iter().any(|a| a.kind == "validation_snapshot"));
        assert!(result.artifacts.iter().any(|a| a.kind == "validation_report"));
        assert_eq!(session.doc.param("thickness").map(|p| p.expr.as_str()), Some("8 mm"));
        let measured = session.execute("inspect.measure", &json!({})).unwrap();
        let volume = measured.pointer("/bodies/0/volume_mm3").and_then(Value::as_f64).unwrap();
        assert!((volume - (1800.0 - 4.0 * std::f64::consts::PI) * 8.0).abs() < 1e-3);
        let holes = session
            .doc
            .features
            .iter()
            .find_map(|feature| match &feature.kind {
                FeatureKind::Hole { points: Some(points), .. } => Some(points.ids.len()),
                _ => None,
            })
            .unwrap_or(0);
        assert_eq!(holes, 4, "the four 2 mm holes stay parametric");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn live_task_repair_success_is_a_separate_terminal() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let mut session = bracket("5 mm");
        let (mut task, root) = open(&mut session, "repair-ok", 8.0, 1);
        let action = write(&mut task, &mut session, "6 mm");
        let task::TurnAction::Repair { prompt } = action else { panic!("expected repair, got {action:?}") };
        assert!(prompt.contains("repair round 1"));
        let action = write(&mut task, &mut session, "8 mm");
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
        assert_eq!(result.terminal, task::TaskTerminal::ValidatedSuccess, "{result:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn live_task_with_no_repair_round_is_quality_failed_at_once() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let mut session = bracket("5 mm");
        let (mut task, root) = open(&mut session, "repair-zero", 8.0, 0);
        let action = write(&mut task, &mut session, "6 mm");
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
        assert_eq!(result.terminal, task::TaskTerminal::QualityFailed, "{result:?}");
        assert!(result.reason.contains("repair budget"), "{}", result.reason);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn manual_parameter_change_captures_the_new_revision_and_revalidates() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let mut session = bracket("5 mm");
        let (mut task, root) = open(&mut session, "manual-revalidate", 8.0, 0);
        let task::TurnAction::Terminal(result) = write(&mut task, &mut session, "8 mm") else { panic!("expected success") };
        assert_eq!(result.terminal, task::TaskTerminal::ValidatedSuccess, "{result:?}");

        session.execute("parameters.change", &json!({"name": "thickness", "expression": "7 mm"})).unwrap();
        let revision = session.document_stamp().revision;
        let changed = task.revalidate(&mut session).unwrap();
        assert!(!changed.passed, "the manual 7 mm revision is not the requested 8 mm");
        assert_eq!(changed.snapshot.document.version, revision, "the new revision is captured");
        assert_eq!(changed.report.checks.len(), 4, "the #65 profile reports all four required checks");
        assert!(changed.report.checks.iter().any(|check| check.check_id == "parameters.required" && !check.passed), "{:?}", changed.report.checks);

        session.execute("parameters.change", &json!({"name": "thickness", "expression": "8 mm"})).unwrap();
        let restored = task.revalidate(&mut session).unwrap();
        assert!(restored.passed, "{:?}", restored.report);
        assert_eq!(restored.report.profile.profile_id, BETA_PROFILE_ID);
        assert_eq!(restored.report.profile.version, BETA_PROFILE_VERSION);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn live_task_budget_exhaustion_is_quality_failed_after_the_last_allowed_repair() {
        let _guard = SNAPSHOT_LOCK.lock().unwrap();
        let mut session = bracket("5 mm");
        let (mut task, root) = open(&mut session, "repair-budget", 8.0, 1);
        // The first wrong write uses the only allowed repair round.
        let action = write(&mut task, &mut session, "6 mm");
        let task::TurnAction::Repair { .. } = action else { panic!("expected repair, got {action:?}") };
        // The repair turn is real (a new plan, a new pre-apply snapshot) but still wrong.
        let action = write(&mut task, &mut session, "6 mm");
        let task::TurnAction::Terminal(result) = action else { panic!("expected terminal, got {action:?}") };
        assert_eq!(result.terminal, task::TaskTerminal::QualityFailed, "{result:?}");
        assert!(result.reason.contains("repair budget"), "{}", result.reason);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn the_l_bracket_profile_matches_the_beta_fixture() {
        let profile = l_bracket_profile(8.0).unwrap();
        assert_eq!(profile.profile_id, BETA_PROFILE_ID);
        assert_eq!(profile.version, BETA_PROFILE_VERSION);
        assert_eq!(
            profile.checks.iter().map(|check| check.check_id.as_str()).collect::<Vec<_>>(),
            ["geometry.exact", "input.integrity", "native.rebuild", "parameters.required"]
        );
        assert!(profile.checks.iter().all(|check| check.required));
    }
}
