//! A design's command script (`timeline.export_script`, #886) replays into the same design:
//! every corpus design that the script supports comes back with the same timeline, parameters,
//! settings and bodies.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use solidcraft_engine::Session;

fn corpus() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut v: Vec<PathBuf> =
        std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "solidcraft")).collect();
    v.sort();
    assert!(v.len() >= 7, "corpus files in {}", dir.display());
    v
}

/// Designs with components or joints, which scripts do not support yet.
const ASSEMBLIES: [&str; 2] = ["components-arm", "joints-hinge"];

fn bodies(s: &mut Session) -> Vec<(String, f64, f64, Value)> {
    let m = s.execute("inspect.measure", &json!({})).unwrap();
    let mut v: Vec<_> = m["bodies"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|b| {
            (
                b["name"].as_str().unwrap_or_default().to_string(),
                b["volume_mm3"].as_f64().unwrap(),
                b["area_mm2"].as_f64().unwrap(),
                b["faces"].clone(),
            )
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn errors(s: &Session) -> Vec<(u64, Option<String>)> {
    s.model.results.iter().map(|r| (r.id, r.error.clone())).collect()
}

fn params(s: &Session) -> Vec<solidcraft_engine::doc::Param> {
    let mut v = s.doc.params.clone();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

#[test]
fn corpus_designs_replay_from_their_scripts() {
    let mut replayed = 0;
    for p in corpus() {
        let name = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let mut s = Session::default();
        s.execute("doc.open", &json!({"path": p.to_string_lossy()})).unwrap();
        let r = match s.execute("timeline.export_script", &json!({})) {
            Ok(r) => r,
            Err(e) => {
                assert!(ASSEMBLIES.contains(&name.as_str()), "{name}: {e}");
                assert!(e.to_string().contains("not supported yet"), "{name}: {e}");
                continue;
            }
        };
        assert!(!ASSEMBLIES.contains(&name.as_str()), "{name} has components: the script should refuse it");
        // Through text, as a script file would carry it.
        let script: Value = serde_json::from_str(&r["script"].to_string()).unwrap();
        let mut t = Session::default();
        t.run_script(&script).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(t.doc.features, s.doc.features, "{name}: timeline");
        assert_eq!(params(&t), params(&s), "{name}: parameters");
        assert_eq!(t.doc.marker, s.doc.marker, "{name}: marker");
        assert_eq!(t.doc.units, s.doc.units, "{name}: units");
        assert_eq!(t.doc.sheet, s.doc.sheet, "{name}: sheet metal rules");
        assert_eq!(t.doc.plastic, s.doc.plastic, "{name}: plastic rules");
        assert_eq!(t.doc.materials, s.doc.materials, "{name}: materials");
        assert_eq!(t.doc.user_materials, s.doc.user_materials, "{name}: user materials");
        assert_eq!(t.doc.favorites, s.doc.favorites, "{name}: favourites");
        assert_eq!(t.doc.param_comments, s.doc.param_comments, "{name}: comments");
        assert_eq!(errors(&t), errors(&s), "{name}: feature results");
        assert_eq!(bodies(&mut t), bodies(&mut s), "{name}: bodies");
        replayed += 1;
    }
    assert!(replayed >= 7, "replayed {replayed} corpus designs");
}

/// #1990: a sketch and its extrude may share a name, and the design's script still replays,
/// keeping the names: the replayed design's own script has the same steps.
#[test]
fn a_design_naming_a_sketch_and_its_extrude_alike_replays_from_its_script() {
    let dir = std::env::temp_dir().join(format!("solidcraft-shared-names-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cli = |args: &[&std::ffi::OsStr]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli")).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{}: {}",
            args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let script = dir.join("mini.json");
    let saved = dir.join("mini.solidcraft");
    let exported = dir.join("rt.json");
    let replayed = dir.join("rt.solidcraft");
    let again = dir.join("rt2.json");
    let commands = json!({"commands": [
        {"command": "sketch.create", "params": {"plane": "XY", "name": "Block"}},
        {"command": "sketch.rectangle.two_point", "params": {"p0": [0, 0], "p1": [20, 10]}},
        {"command": "sketch.finish", "params": {}},
        {"command": "solid.extrude", "params": {"distance": "5", "name": "Block"}},
    ]});
    std::fs::write(&script, commands.to_string()).unwrap();
    cli(&["run".as_ref(), script.as_ref(), "--save".as_ref(), saved.as_ref()]);
    cli(&["script".as_ref(), saved.as_ref(), "--out".as_ref(), exported.as_ref()]);
    cli(&["run".as_ref(), exported.as_ref(), "--save".as_ref(), replayed.as_ref()]);
    cli(&["script".as_ref(), replayed.as_ref(), "--out".as_ref(), again.as_ref()]);
    let first: Value = serde_json::from_str(&std::fs::read_to_string(&exported).unwrap()).unwrap();
    let second: Value = serde_json::from_str(&std::fs::read_to_string(&again).unwrap()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let names: Vec<&str> = first["commands"].as_array().unwrap().iter().filter_map(|c| c["params"]["feature"]["name"].as_str()).collect();
    assert_eq!(names, ["Block", "Block"], "{first:#}");
    assert_eq!(second["commands"], first["commands"], "the replayed design's script has the same steps");
}

/// #10230: script export must disclose the print-face state that replay leaves out.
#[test]
fn script_export_warns_about_omitted_print_faces_on_stderr() {
    let dir = std::env::temp_dir().join(format!("solidcraft-print-face-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let saved = dir.join("block.sc");
    let exported = dir.join("block.json");
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 20, "width": 10, "height": 5, "body_name": "Block"})).unwrap();
    s.execute("body.print_face", &json!({"body": "Block", "point": [10, 5, 0]})).unwrap();
    s.execute("file.save", &json!({"path": saved})).unwrap();
    let native = std::fs::read(&saved).unwrap();

    for to_file in [false, true] {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"));
        cmd.arg("script").arg(&saved);
        if to_file {
            cmd.arg("--out").arg(&exported);
        }
        let out = cmd.output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        assert!(stderr.contains("note: the script leaves out print-face assignments (body.print_face)"), "{stderr}");
        let script = if to_file {
            assert!(out.stdout.is_empty(), "--out must keep stdout empty");
            std::fs::read(&exported).unwrap()
        } else {
            out.stdout
        };
        let script: Value = serde_json::from_slice(&script).unwrap();
        let mut replayed = Session::default();
        replayed.run_script(&script).unwrap();
        assert!(replayed.doc.print_faces.is_empty());
        assert_eq!(replayed.doc.features, s.doc.features);
        assert_eq!(std::fs::read(&saved).unwrap(), native, "export must not change the source design");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// #11: the CLI captures a native snapshot through the command channel, and the captured design
/// reopens independently of the live session and its file.
#[test]
fn cli_captures_a_native_snapshot_that_reopens_after_the_live_design_is_gone() {
    let dir = std::env::temp_dir().join(format!("solidcraft-cli-snapshot-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = dir.join("candidates");
    let script = dir.join("capture.json");
    let results = dir.join("results.json");
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "parameters.add", "params": {"name": "width", "expression": "12 mm"}},
            {"command": "solid.box", "params": {"length": "width", "width": 20, "height": 6}},
            {"command": "document.revision", "params": {}},
            {"command": "document.snapshot.capture", "params": {}},
        ]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .arg("run")
        .arg(&script)
        .args(["--results", "--json-out"])
        .arg(&results)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &store)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let replies: Value = serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    let revision = &replies.as_array().unwrap()[2];
    let candidate = &replies.as_array().unwrap()[3]["candidate"];
    assert_eq!(candidate["source"]["revision"], revision["revision"].to_string(), "a canonical decimal revision");
    assert_eq!(candidate["source"]["documentId"], revision["document_id"]);
    assert_eq!(candidate["artifact"]["mediaType"], "application/zip");
    let id = candidate["candidateId"].as_str().unwrap();

    let verified = solidcraft_engine::io::candidate::lookup(&store, id).unwrap();
    assert_eq!(verified.candidate.manifest_digest, candidate["manifestDigest"]);
    assert_eq!(verified.candidate.candidate_digest, candidate["candidateDigest"]);
    let design = verified.files.iter().find(|file| file.path == "design.sc").unwrap();
    let reopened_path = dir.join("reopened.sc");
    std::fs::write(&reopened_path, &design.bytes).unwrap();
    let mut reopened = Session::default();
    reopened.execute("doc.open", &json!({"path": reopened_path.to_string_lossy()})).unwrap();
    assert_eq!(reopened.doc.param("width").map(|p| p.expr.as_str()), Some("12 mm"));
    assert_eq!(reopened.doc.features.len(), 1, "the native feature history reopens");
    std::fs::remove_dir_all(dir).unwrap();
}

/// #12: the real CLI runs the local validation port over a captured candidate. The analytic
/// profile passes; a deliberate parameter/measurement mismatch is a quality failure; the same
/// submission key returns the same run and a different payload conflicts.
#[test]
fn cli_validates_a_captured_candidate_and_distinguishes_quality_from_infrastructure() {
    use solidcraft_engine::validation::{MeasurementRequirement, ParameterRequirement, generate_submission_key, profile_ref, task_profile};

    let dir = std::env::temp_dir().join(format!("solidcraft-cli-validate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let candidates = dir.join("candidates");
    let validation = dir.join("validation");
    let script = dir.join("capture.json");
    let results = dir.join("results.json");
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "parameters.add", "params": {"name": "width", "expression": "20 mm"}},
            {"command": "solid.box", "params": {"length": "width", "width": 10, "height": 5}},
            {"command": "document.snapshot.capture", "params": {}},
        ]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .arg("run")
        .arg(&script)
        .args(["--results", "--json-out"])
        .arg(&results)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let replies: Value = serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    let candidate = replies.as_array().unwrap().last().unwrap()["candidate"].clone();

    let profile = task_profile(
        "task-cli",
        "1.0.0",
        Some(vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }]),
        Some(vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 1000.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }]),
    )
    .unwrap();
    let reference = profile_ref(&profile).unwrap();
    let profile_path = dir.join("profile.json");
    std::fs::write(&profile_path, serde_json::to_string_pretty(&profile).unwrap()).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let key = generate_submission_key(now);
    let request = json!({"candidate": candidate, "profile": reference, "requestId": key});
    let request_path = dir.join("request.json");
    std::fs::write(&request_path, serde_json::to_string_pretty(&request).unwrap()).unwrap();
    let status_path = dir.join("status.json");

    let validate = |mode: &str, run: Option<&str>, profile: Option<&Path>, out: Option<&Path>| {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"));
        cmd.arg("validate").arg(mode);
        if let Some(run) = run {
            cmd.arg(run);
        } else {
            cmd.arg(&request_path);
        }
        if let Some(profile) = profile {
            cmd.arg("--profile").arg(profile);
        }
        if let Some(out) = out {
            cmd.arg("--json-out").arg(out);
        }
        cmd.args(["--candidate-dir"]).arg(&candidates).args(["--validation-dir"]).arg(&validation);
        cmd.env("SOLIDCRAFT_CANDIDATE_DIR", &candidates);
        cmd.env("SOLIDCRAFT_VALIDATION_DIR", &validation);
        cmd.output().unwrap()
    };

    let out = validate("submit", None, Some(&profile_path), Some(&status_path));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let status: Value = serde_json::from_slice(&std::fs::read(&status_path).unwrap()).unwrap();
    assert_eq!(status["state"], "passed", "{status:#}");
    let run_id = status["runId"].as_str().unwrap().to_string();

    let report_path = dir.join("report.json");
    let out = validate("report", Some(&run_id), None, Some(&report_path));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report["report"]["state"], "passed");
    assert_eq!(report["report"]["checks"].as_array().unwrap().len(), 4);
    let persisted = std::fs::read(validation.join("runs").join(format!("{run_id}.report.json"))).unwrap();
    assert_eq!(report["reference"]["digest"], solidcraft_engine::io::candidate::digest(&persisted));

    // The same key and payload is the same durable job.
    let out = validate("submit", None, Some(&profile_path), Some(&status_path));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let again: Value = serde_json::from_slice(&std::fs::read(&status_path).unwrap()).unwrap();
    assert_eq!(again["runId"], run_id);

    // A deliberate measurement mismatch is a quality failure, not a CLI error.
    let wrong = task_profile(
        "task-cli-wrong",
        "1.0.0",
        Some(vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }]),
        Some(vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 999.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }]),
    )
    .unwrap();
    let wrong_reference = profile_ref(&wrong).unwrap();
    let wrong_path = dir.join("profile-wrong.json");
    std::fs::write(&wrong_path, serde_json::to_string_pretty(&wrong).unwrap()).unwrap();
    let wrong_request = dir.join("request-wrong.json");
    std::fs::write(
        &wrong_request,
        serde_json::to_string_pretty(&json!({
            "candidate": candidate,
            "profile": wrong_reference,
            "requestId": generate_submission_key(now + 1),
        }))
        .unwrap(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .args(["validate", "submit"])
        .arg(&wrong_request)
        .args(["--profile"])
        .arg(&wrong_path)
        .args(["--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let wrong_status: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(wrong_status["state"], "quality_failed", "{wrong_status:#}");

    // The same key with another profile payload is a conflict.
    let conflict_request = dir.join("request-conflict.json");
    std::fs::write(
        &conflict_request,
        serde_json::to_string_pretty(&json!({"candidate": candidate, "profile": wrong_reference, "requestId": key})).unwrap(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .args(["validate", "submit"])
        .arg(&conflict_request)
        .args(["--profile"])
        .arg(&wrong_path)
        .args(["--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("IDEMPOTENCY_CONFLICT"), "{}", String::from_utf8_lossy(&out.stderr));

    // Cancelling a terminal run preserves it.
    let out = validate("cancel", Some(&run_id), None, None);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let cancelled: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cancelled["state"], "passed");
    let _ = std::fs::remove_dir_all(dir);
}

/// #65: the headless CLI consumer prints the same bound skill report a panel displays, requires
/// live, and keeps the snapshot/report identity fields.
#[test]
fn cli_skill_validation_prints_a_bound_report_and_requires_live() {
    use solidcraft_engine::validation::{
        MeasurementRequirement, ParameterRequirement, generate_submission_key, text_to_cad_profile, text_to_cad_profile_ref,
    };

    let dir = std::env::temp_dir().join(format!("solidcraft-cli-skill-validate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let candidates = dir.join("candidates");
    let validation = dir.join("validation");
    let script = dir.join("capture.json");
    let results = dir.join("results.json");
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "parameters.add", "params": {"name": "width", "expression": "20 mm"}},
            {"command": "solid.box", "params": {"length": "width", "width": 10, "height": 5}},
            {"command": "document.snapshot.capture", "params": {}},
        ]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .arg("run")
        .arg(&script)
        .args(["--results", "--json-out"])
        .arg(&results)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let replies: Value = serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    let candidate = replies.as_array().unwrap().last().unwrap()["candidate"].clone();
    let source = candidate["source"].clone();
    let revision = source["revision"].as_str().unwrap().parse::<u64>().unwrap();

    let profile = text_to_cad_profile(
        vec![ParameterRequirement { name: "width".into(), value: 20.0, abs_tolerance: 1e-6 }],
        vec![MeasurementRequirement {
            command: "inspect.measure".into(),
            params: json!({}),
            result_pointer: "/total/volume_mm3".into(),
            expected: 1000.0,
            abs_tolerance: 1e-6,
            rel_tolerance: 0.0,
        }],
    )
    .unwrap();
    let reference = text_to_cad_profile_ref(&profile).unwrap();
    let profile_path = dir.join("skill-profile.json");
    std::fs::write(&profile_path, serde_json::to_string_pretty(&profile).unwrap()).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let request = json!({
        "candidate": candidate.clone(),
        "profile": reference.clone(),
        "requestId": generate_submission_key(now),
        "live": {"documentId": source["documentId"], "revision": revision},
    });
    let request_path = dir.join("skill-request.json");
    std::fs::write(&request_path, serde_json::to_string_pretty(&request).unwrap()).unwrap();
    let report_path = dir.join("skill-report.json");

    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"));
    cmd.args(["validate", "skill"]).arg(&request_path).args(["--profile"]).arg(&profile_path);
    cmd.args(["--candidate-dir"]).arg(&candidates).args(["--validation-dir"]).arg(&validation);
    cmd.args(["--json-out"]).arg(&report_path);
    let out = cmd.output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let report: Value = serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    let run_id = report["runId"].as_str().unwrap();
    assert_eq!(report["state"], "passed", "{report:#}");
    assert_eq!(report["snapshotPassed"], true);
    assert_eq!(report["approved"], true);
    assert!(report["failure"].is_null());
    assert_eq!(report["snapshotId"], candidate["candidateId"]);
    assert_eq!(report["candidateDigest"], candidate["candidateDigest"]);
    assert_eq!(report["manifestDigest"], candidate["manifestDigest"]);
    assert_eq!(report["documentId"], source["documentId"]);
    assert_eq!(report["documentRevision"], source["revision"]);
    assert_eq!(report["profile"]["profileId"], "text-to-cad");
    assert_eq!(report["profile"]["version"], "1.0.0");
    assert_eq!(report["profile"]["digest"], reference.digest.as_str());
    assert_eq!(report["requiredChecks"], json!(["geometry.exact", "input.integrity", "native.rebuild", "parameters.required"]));
    assert_eq!(report["checks"].as_array().unwrap().len(), 4);
    assert!(report["checks"].as_array().unwrap().iter().all(|check| check["required"] == true && check["passed"] == true));
    let persisted = std::fs::read(validation.join("runs").join(format!("{run_id}.report.json"))).unwrap();
    assert_eq!(report["report"]["digest"], solidcraft_engine::io::candidate::digest(&persisted));
    assert_eq!(report["report"]["sizeBytes"], persisted.len());

    // Omitting live never claims approval; the CLI refuses the request before admission.
    let no_live_request = dir.join("skill-request-no-live.json");
    let mut request_without_live = request.clone();
    request_without_live.as_object_mut().unwrap().remove("live");
    std::fs::write(&no_live_request, serde_json::to_string_pretty(&request_without_live).unwrap()).unwrap();
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"));
    cmd.args(["validate", "skill"]).arg(&no_live_request).args(["--profile"]).arg(&profile_path);
    cmd.args(["--candidate-dir"]).arg(&candidates).args(["--validation-dir"]).arg(&validation);
    let out = cmd.output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("live"), "{}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(dir);
}

/// #12: a changed captured input byte is rejected before the report can be accepted.
#[test]
fn cli_validation_rejects_a_changed_candidate_byte() {
    let dir = std::env::temp_dir().join(format!("solidcraft-cli-validate-changed-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let candidates = dir.join("candidates");
    let validation = dir.join("validation");
    let script = dir.join("capture.json");
    let results = dir.join("results.json");
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "solid.box", "params": {"length": 20, "width": 10, "height": 5}},
            {"command": "document.snapshot.capture", "params": {}},
        ]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .arg("run")
        .arg(&script)
        .args(["--results", "--json-out"])
        .arg(&results)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let replies: Value = serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    let candidate = replies.as_array().unwrap().last().unwrap()["candidate"].clone();
    let id = candidate["candidateId"].as_str().unwrap();
    let container = candidates.join(format!("{id}.zip"));
    let mut bytes = std::fs::read(&container).unwrap();
    *bytes.last_mut().unwrap() ^= 0xff;
    std::fs::write(&container, bytes).unwrap();

    let request = json!({
        "candidate": candidate,
        "profile": solidcraft_engine::validation::profile_ref(&solidcraft_engine::validation::native_basic_profile()).unwrap(),
        "requestId": solidcraft_engine::validation::generate_submission_key(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
        ),
    });
    let request_path = dir.join("request.json");
    std::fs::write(&request_path, serde_json::to_string_pretty(&request).unwrap()).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .args(["validate", "submit"])
        .arg(&request_path)
        .args(["--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("INVALID_CANDIDATE") || stderr.contains("DEPENDENCY_CHANGED"), "{stderr}");
    let _ = std::fs::remove_dir_all(dir);
}

/// #12: a second process's status/cancel is a read-only observer. It must not reconcile the
/// first process's running job into `unknown`, and cancel must persist a marker and win over the
/// worker's late result.
#[test]
fn cli_status_and_cancel_from_a_second_process_do_not_reconcile_a_live_submit() {
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let dir = std::env::temp_dir().join(format!("solidcraft-cli-validate-two-process-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let candidates = dir.join("candidates");
    let validation = dir.join("validation");
    let script = dir.join("capture.json");
    let results = dir.join("results.json");
    std::fs::write(
        &script,
        json!({"commands": [
            {"command": "solid.box", "params": {"length": 20, "width": 10, "height": 5}},
            {"command": "document.snapshot.capture", "params": {}},
        ]})
        .to_string(),
    )
    .unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .arg("run")
        .arg(&script)
        .args(["--results", "--json-out"])
        .arg(&results)
        .env("SOLIDCRAFT_CANDIDATE_DIR", &candidates)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let replies: Value = serde_json::from_slice(&std::fs::read(&results).unwrap()).unwrap();
    let candidate = replies.as_array().unwrap().last().unwrap()["candidate"].clone();
    let request = json!({
        "candidate": candidate,
        "profile": solidcraft_engine::validation::profile_ref(&solidcraft_engine::validation::native_basic_profile()).unwrap(),
        "requestId": solidcraft_engine::validation::generate_submission_key(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
        ),
    });
    let request_path = dir.join("request.json");
    std::fs::write(&request_path, serde_json::to_string_pretty(&request).unwrap()).unwrap();

    let submit = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .args(["validate", "submit"])
        .arg(&request_path)
        .args(["--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .env("SOLIDCRAFT_VALIDATION_RUNNER_DELAY_MS", "10000")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let run_id = {
        let deadline = Instant::now() + Duration::from_secs(15);
        'poll: loop {
            if let Ok(entries) = std::fs::read_dir(validation.join("runs")) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".json")
                        && !name.contains(".state.")
                        && !name.contains(".profile.")
                        && !name.contains(".worker.")
                        && !name.contains(".report.")
                        && let Some(id) = name.strip_suffix(".json")
                    {
                        let status = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
                            .args(["validate", "status", id, "--candidate-dir"])
                            .arg(&candidates)
                            .args(["--validation-dir"])
                            .arg(&validation)
                            .output()
                            .unwrap();
                        let value: Value = serde_json::from_slice(&status.stdout).unwrap_or(Value::Null);
                        if value["state"] == "running" {
                            break 'poll id.to_string();
                        }
                    }
                }
            }
            assert!(Instant::now() < deadline, "the submit never reached running");
            std::thread::sleep(Duration::from_millis(50));
        }
    };

    let cancel = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .args(["validate", "cancel", &run_id, "--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .output()
        .unwrap();
    assert!(cancel.status.success(), "{}", String::from_utf8_lossy(&cancel.stderr));
    let cancelled: Value = serde_json::from_slice(&cancel.stdout).unwrap();
    assert_eq!(cancelled["state"], "cancelled", "{cancelled:#}");
    assert!(validation.join("runs").join(format!("{run_id}.cancel")).is_file(), "the cancel marker was not persisted");

    let submit_out = submit.wait_with_output().unwrap();
    assert!(submit_out.status.success(), "{}", String::from_utf8_lossy(&submit_out.stderr));
    let submitted: Value = serde_json::from_slice(&submit_out.stdout).unwrap();
    assert_eq!(submitted["state"], "cancelled", "{submitted:#}");

    let status = std::process::Command::new(env!("CARGO_BIN_EXE_solidcraft-cli"))
        .args(["validate", "status", &run_id, "--candidate-dir"])
        .arg(&candidates)
        .args(["--validation-dir"])
        .arg(&validation)
        .output()
        .unwrap();
    let final_status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(final_status["state"], "cancelled", "{final_status:#}");
    let _ = std::fs::remove_dir_all(dir);
}
