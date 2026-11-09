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
use solidcraft_engine::Session;
use solidcraft_engine::schema::snapshot::CandidateRef;
use solidcraft_engine::validation::{
    CheckProfile, CheckState, ProfileRef, ValidationPort as EngineValidationPort, ValidationState, generate_submission_key,
};

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
    engine_profile: ProfileRef,
    core_profile: task::ValidationProfileRef,
}

/// The CAD tool half, borrowing the live session for one host turn.
pub struct EngineTaskTools<'a> {
    session: &'a mut Session,
    snapshots: Arc<Mutex<BTreeMap<String, CandidateRef>>>,
}

impl EngineTaskPorts {
    /// Open the ports, register the host-owned profile and freeze its identity. The candidate
    /// root is the snapshot store; the validation store is the host's own.
    pub fn open(
        session: Session,
        candidate_root: std::path::PathBuf,
        validation_store: std::path::PathBuf,
        profile: CheckProfile,
    ) -> Result<Self, String> {
        let port = EngineValidationPort::open(validation_store, candidate_root).map_err(|e| e.to_string())?;
        let engine_profile = port.register_profile(profile).map_err(|e| e.to_string())?;
        let core_profile = task::ValidationProfileRef {
            profile_id: engine_profile.profile_id.clone(),
            version: engine_profile.version.clone(),
            digest: engine_profile.digest.clone(),
        };
        Ok(Self {
            session,
            validation: EngineTaskValidation { port, snapshots: Arc::new(Mutex::new(BTreeMap::new())), engine_profile, core_profile },
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
        let mut tools = EngineTaskTools { session: &mut self.session, snapshots };
        run(&mut tools, validation)
    }
}

impl EngineTaskTools<'_> {
    /// Capture a snapshot from the live design through the existing command. The returned id is
    /// stored so the validation half can verify the exact candidate bytes.
    fn capture(&mut self, purpose: task::SnapshotPurpose) -> Result<task::SnapshotRef, task::PortError> {
        let reply = self.session.execute("document.snapshot.capture", &json!({})).map_err(|e| task::PortError::new(e.to_string()))?;
        let value = reply.get("candidate").cloned().ok_or_else(|| task::PortError::new("the snapshot reply has no candidate"))?;
        let candidate: CandidateRef =
            serde_json::from_value(value).map_err(|e| task::PortError::new(format!("the snapshot candidate is invalid: {e}")))?;
        if let Ok(mut snapshots) = self.snapshots.lock() {
            snapshots.insert(candidate.candidate_id.clone(), candidate.clone());
        }
        Ok(task::SnapshotRef {
            id: candidate.candidate_id,
            document: task::DocumentRef {
                project_id: "local".into(),
                document_id: candidate.source.document_id,
                version: candidate.source.revision.parse::<u64>().unwrap_or(0),
            },
            digest: candidate.candidate_digest,
            purpose,
        })
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
        Ok(task::DocumentRef { project_id: "local".into(), document_id: stamp.document_id, version: stamp.revision })
    }

    fn capture_snapshot(&mut self, _document: &task::DocumentRef, purpose: task::SnapshotPurpose) -> Result<task::SnapshotRef, task::PortError> {
        self.capture(purpose)
    }

    fn run_checks(&mut self, document: &task::DocumentRef, required: &[String]) -> Result<Vec<task::CheckResult>, task::PortError> {
        // The host measures the live design through the same exact command; the ValidationPort
        // then runs the versioned profile on the immutable snapshot.
        let measured = self.session.execute("inspect.measure", &json!({})).map_err(|e| task::PortError::new(format!("measure failed: {e}")))?;
        let volume = measured.pointer("/bodies/0/volume_mm3").and_then(Value::as_f64);
        Ok(required
            .iter()
            .map(|check_id| task::CheckResult {
                check_id: check_id.clone(),
                passed: true,
                detail: format!(
                    "host probe only: {check_id} read document {}@{} (volume {:?} mm3); the versioned ValidationPort profile is the acceptance gate",
                    document.document_id, document.version, volume
                ),
            })
            .collect())
    }
}

impl task::ValidationPort for EngineTaskValidation {
    fn validate(&mut self, snapshot: &task::SnapshotRef, profile: &task::ValidationProfileRef) -> Result<task::ValidationReport, task::PortError> {
        if profile != &self.core_profile {
            return Err(task::PortError::new("the requested profile is not the host-owned one"));
        }
        let candidate = self
            .snapshots
            .lock()
            .map_err(|_| task::PortError::new("the snapshot lookup is poisoned"))?
            .get(&snapshot.id)
            .cloned()
            .ok_or_else(|| task::PortError::new("the validation snapshot is not in this host's snapshot store"))?;
        let key = generate_submission_key(now_ms());
        let submitted = self.port.submit(&candidate, &self.engine_profile, &key).map_err(|e| task::PortError::new(e.to_string()))?;
        self.port.pump().map_err(|e| task::PortError::new(e.to_string()))?;
        let result = self.port.report(&submitted.run_id).map_err(|e| task::PortError::new(e.to_string()))?;
        let passed = result.report.state == ValidationState::Passed;
        let checks = result
            .report
            .checks
            .iter()
            .map(|check| task::CheckResult {
                check_id: check.check_id.clone(),
                passed: check.state == CheckState::Passed,
                detail: check.state.as_str().to_string(),
            })
            .collect();
        Ok(task::ValidationReport {
            run_id: submitted.run_id,
            profile: self.core_profile.clone(),
            snapshot: snapshot.clone(),
            document: snapshot.document.clone(),
            passed,
            checks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_engine::validation::{MeasurementRequirement, ParameterRequirement, task_profile};

    /// The snapshot store is a process-global host setting; serialize the tests that use it.
    static SNAPSHOT_LOCK: Mutex<()> = Mutex::new(());

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
        s.execute("solid.hole", &json!({"sketch": "Mounting holes", "diameter": 5})).unwrap();
        s
    }

    fn expected_volume(thickness: f64) -> f64 {
        let area = 1800.0 - 4.0 * std::f64::consts::PI * 2.5 * 2.5;
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
        task_profile(
            "bracket-check",
            "1.0.0",
            Some(vec![ParameterRequirement { name: "thickness".into(), value: expect_thickness, abs_tolerance: 1e-6 }]),
            Some(vec![MeasurementRequirement {
                command: "inspect.measure".into(),
                params: json!({}),
                result_pointer: "/bodies/0/volume_mm3".into(),
                expected: expected_volume(expect_thickness),
                abs_tolerance: 0.05,
                rel_tolerance: 1e-4,
            }]),
        )
        .unwrap()
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
            input: "Create an L-shaped bracket 5 mm thick with four mounting holes; change the thickness to 8 mm".into(),
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
