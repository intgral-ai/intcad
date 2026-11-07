//! `document.snapshot.capture` (#11): freeze the live design, its manifest and every supported
//! dependency into a host-owned immutable candidate.
//!
//! Capture reads the document only: it never evaluates the model, marks the design saved, adds an
//! undo step or changes what is shown. The guarded executor checks `expectedDocument` before this
//! command runs, so a stale stamp fails before anything is pinned or written.
//!
//! Dependencies are captured, not referenced: each linked design file is frozen into the
//! container and the captured design's link is rewritten to the frozen relative path. A link that
//! has changed since this design last read it, cannot be read, or itself links to another design
//! is refused with `DEPENDENCY_CHANGED`; the live path is never kept as a mutable input.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use solidcraft_doc::Document;
use solidcraft_io::candidate::{self, CandidateFile, StoredCandidate};

use super::{CommandSpec, design_link};
use crate::schema::snapshot::{
    CandidateArtifact, CandidateRef, InputKind, ManifestInput, SnapshotCaptureParams, SnapshotCaptureResult, SnapshotManifest, SnapshotSource,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{EngineError, Result, Session};

const CAPTURE: TypedContract<SnapshotCaptureParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] =
    &[CommandSpec::new("document.snapshot.capture", "Capture Snapshot", crate::checked!("document.snapshot.capture", capture))
        .icon("capture")
        .noundo()
        .doc_only()
        .contract(&CAPTURE)
        .output(output_schema_for::<SnapshotCaptureResult>)
        .params(
            "→ candidate: {candidateId, source: {documentId, revision}, candidateDigest, manifestDigest, artifact: {artifactId, digest, \
     sizeBytes, mediaType}} (one immutable native design snapshot; no parameters)",
        )];

static NEXT_CANDIDATE: AtomicU64 = AtomicU64::new(0);

/// The host store: `SOLIDCRAFT_CANDIDATE_DIR`, the per-user data folder in a build, or a
/// per-process scratch folder under the test recovery folder. The browser has no such store.
fn candidate_root() -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(dir) = std::env::var_os("SOLIDCRAFT_CANDIDATE_DIR").filter(|dir| !dir.is_empty()) {
            return Some(PathBuf::from(dir));
        }
        if cfg!(test) {
            return crate::recovery::default_dir().map(|dir| dir.join("candidates"));
        }
        crate::recovery::data_dir().map(|dir| dir.join("validation-candidates"))
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)).unwrap_or(0)
}

/// A durable host identity. The process-local document ID is deliberately not used.
fn new_candidate_id(captured_ms: u64) -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let serial = NEXT_CANDIDATE.fetch_add(1, Ordering::Relaxed);
    format!("cand-{captured_ms}-{nanos:x}-{:x}-{serial:x}", std::process::id())
}

/// RFC3339 UTC with milliseconds, from Unix milliseconds.
fn captured_at(ms: u64) -> String {
    let millis = ms % 1000;
    let seconds = ms / 1000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX / 2);
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (day_seconds / 3600, (day_seconds / 60) % 60, day_seconds % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm, no wall clock or locale).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn dependency_changed(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("DEPENDENCY_CHANGED: document.snapshot.capture: {message}"))
}

fn invalid_candidate(message: impl std::fmt::Display) -> EngineError {
    EngineError::Other(format!("INVALID_CANDIDATE: document.snapshot.capture: {message}"))
}

/// Freeze every supported external design link into `design`, rewriting the captured links to
/// the container's relative paths. Returns the frozen files and their manifest rows.
fn capture_dependencies(s: &Session, design: &mut Document) -> Result<(Vec<CandidateFile>, Vec<ManifestInput>)> {
    let mut components: Vec<u64> = design.links.keys().copied().collect();
    components.sort_unstable();
    let mut files = Vec::with_capacity(components.len());
    let mut inputs = Vec::with_capacity(components.len());
    for component in components {
        let Some(link) = design.links.get(&component).cloned() else { continue };
        let file = design_link::resolve(s.path.as_deref(), &link.path);
        let shown = file.display().to_string();
        let bytes = solidcraft_io::vfs::read_design_path(&shown, s.path.as_deref(), solidcraft_io::limits::max_design_bytes())
            .map_err(|error| dependency_changed(format!("{shown}: {error}")))?;
        let actual = candidate::digest_hex(&bytes);
        if let Some(expected) = link.file_sha256.as_deref()
            && expected != actual
        {
            return Err(dependency_changed(format!("{shown}: the linked design changed since this design last read it")));
        }
        let linked = solidcraft_io::read_design(&bytes).map_err(|error| dependency_changed(format!("{shown}: {error}")))?;
        if !linked.links.is_empty() {
            return Err(dependency_changed(format!("{shown}: nested design links are not supported")));
        }
        let dependency_id = format!("design-link-{component}");
        let path = format!("inputs/{dependency_id}.sc");
        let digest = candidate::digest(&bytes);
        let size_bytes = bytes.len() as u64;
        if let Some(link) = design.links.get_mut(&component) {
            link.path = path.clone();
        }
        files.push(CandidateFile { path: path.clone(), bytes });
        inputs.push(ManifestInput { path, kind: InputKind::Component, dependency_id, dependency_version: None, digest, size_bytes });
    }
    Ok((files, inputs))
}

fn capture(s: &mut Session, _: &Value) -> Result<Value> {
    let Some(root) = candidate_root() else {
        return Err(EngineError::Other("document.snapshot.capture: this build has no candidate store".into()));
    };
    // Pin the coherent state first: a copy of the design (with what is shown), and the bytes of
    // every dependency. Serialization below works only from that copy.
    let source = s.document_stamp();
    let captured_ms = now_ms();
    let candidate_id = new_candidate_id(captured_ms);
    let mut design = s.design_at(None).into_owned();
    let (dependency_files, dependency_inputs) = capture_dependencies(s, &mut design)?;
    let design_bytes = solidcraft_io::write_design_container_with_compression(&design, solidcraft_io::DesignCompression::Fast)
        .map_err(|error| invalid_candidate(format!("serializing the design: {error}")))?;
    let design_digest = candidate::digest(&design_bytes);
    let design_size = design_bytes.len() as u64;
    let mut files = Vec::with_capacity(1 + dependency_files.len());
    files.push(CandidateFile { path: "design.sc".into(), bytes: design_bytes });
    files.extend(dependency_files);
    let mut inputs = Vec::with_capacity(1 + dependency_inputs.len());
    inputs.push(ManifestInput {
        path: "design.sc".into(),
        kind: InputKind::Design,
        dependency_id: "design-root".into(),
        dependency_version: None,
        digest: design_digest,
        size_bytes: design_size,
    });
    inputs.extend(dependency_inputs);
    // The contract orders inputs by bytewise UTF-8 path.
    files.sort_by(|a, b| a.path.cmp(&b.path));
    inputs.sort_by(|a, b| a.path.cmp(&b.path));
    let manifest = SnapshotManifest {
        schema_version: "1.0".into(),
        candidate_id: candidate_id.clone(),
        source: SnapshotSource { document_id: source.document_id.clone(), revision: source.revision.to_string() },
        captured_at: captured_at(captured_ms),
        format_version: if design.format.is_empty() { "solidcraft/unknown".into() } else { design.format.clone() },
        engine_version: env!("CARGO_PKG_VERSION").into(),
        inputs,
    };
    let manifest_value = serde_json::to_value(&manifest).map_err(|error| invalid_candidate(format!("manifest: {error}")))?;
    let artifact_id = format!("artifact-{candidate_id}");
    let stored: StoredCandidate =
        candidate::write(&root, &candidate_id, &files, &manifest_value, &artifact_id).map_err(|error| invalid_candidate(error.to_string()))?;
    let candidate = CandidateRef {
        candidate_id,
        source: manifest.source,
        candidate_digest: stored.candidate_digest,
        manifest_digest: stored.manifest_digest,
        artifact: CandidateArtifact {
            artifact_id: stored.artifact.artifact_id,
            digest: stored.artifact.digest,
            size_bytes: stored.artifact.size_bytes,
            media_type: stored.artifact.media_type,
        },
    };
    serde_json::to_value(SnapshotCaptureResult { candidate }).map_err(|error| invalid_candidate(format!("reply: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentStamp;
    use serde_json::json;

    fn run(s: &mut Session, id: &str, params: Value) -> Value {
        match s.execute(id, &params) {
            Ok(value) => value,
            Err(error) => panic!("{id} {params}: {error}"),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("solidcraft-snapshot-{name}-{}-{nanos}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn open_captured(candidate: &Value) -> (Document, candidate::VerifiedCandidate) {
        let root = candidate_root().expect("test candidate store");
        let id = candidate["candidateId"].as_str().expect("candidateId");
        let verified = candidate::lookup(&root, id).expect("verified candidate");
        let design = verified.files.iter().find(|file| file.path == "design.sc").expect("design input");
        (solidcraft_io::read_design(&design.bytes).expect("captured design opens"), verified)
    }

    #[test]
    fn capture_pins_the_design_and_does_not_mark_it_saved_or_add_history() {
        let mut s = Session::default();
        run(&mut s, "parameters.add", json!({"name": "width", "expression": "10 mm"}));
        run(&mut s, "solid.box", json!({"length": "width", "width": 10, "height": 5}));
        s.mark_saved();
        let stamp = s.document_stamp();
        let undo = s.undo.len();
        let visibility = s.visibility.clone();
        let pending = s.pending_moves.clone();
        let path = s.path.clone();
        assert!(!s.is_dirty());
        let (reply, after) = s.execute_if_current("document.snapshot.capture", &json!({}), &stamp).expect("capture");
        assert_eq!(after, stamp, "the guarded executor reports the captured source stamp");
        assert_eq!(reply["candidate"]["source"]["revision"], json!(stamp.revision.to_string()));
        assert_eq!(reply["candidate"]["source"]["documentId"], json!(stamp.document_id));
        assert_eq!(reply["candidate"]["artifact"]["mediaType"], "application/zip");
        assert!(reply["candidate"]["candidateDigest"].as_str().is_some_and(|d| d.starts_with("sha256:")));
        assert_eq!(s.undo.len(), undo, "capture adds no undo step");
        assert!(!s.is_dirty(), "capture does not mark the live design saved");
        assert_eq!(s.visibility, visibility);
        assert_eq!(s.pending_moves, pending);
        assert_eq!(s.path, path);

        let (captured, verified) = open_captured(&reply["candidate"]);
        assert_eq!(captured.param("width").map(|p| p.expr.as_str()), Some("10 mm"), "parameters survive");
        assert_eq!(captured.features, s.doc.features, "feature history survives");
        assert_eq!(verified.manifest["inputs"][0]["kind"], "design");
        assert_eq!(verified.manifest["inputs"][0]["path"], "design.sc");
        assert_eq!(verified.files.len(), 1);

        // A live edit after capture changes neither the captured bytes nor their digest.
        run(&mut s, "parameters.change", json!({"name": "width", "expression": "20 mm"}));
        let (still, again) = open_captured(&reply["candidate"]);
        assert_eq!(still.param("width").map(|p| p.expr.as_str()), Some("10 mm"));
        assert_eq!(again.candidate, verified.candidate);
    }

    #[test]
    fn a_stale_stamp_fails_before_pinning_and_leaves_history_alone() {
        let mut s = Session::default();
        run(&mut s, "parameters.add", json!({"name": "width", "expression": "10 mm"}));
        let base = s.document_stamp();
        let root = candidate_root().expect("test candidate store");
        let before = std::fs::read_dir(&root).map(|entries| entries.flatten().count()).unwrap_or(0);
        run(&mut s, "parameters.change", json!({"name": "width", "expression": "20 mm"}));
        let undo = s.undo.len();
        let doc = s.doc.clone();
        let dirty = s.is_dirty();
        assert!(matches!(
            s.execute_if_current("document.snapshot.capture", &json!({}), &DocumentStamp { revision: base.revision, ..base.clone() }),
            Err(crate::ConditionedError::Changed { .. })
        ));
        assert_eq!(s.undo.len(), undo);
        assert!(std::sync::Arc::ptr_eq(&s.doc, &doc));
        assert_eq!(s.is_dirty(), dirty);
        let after = std::fs::read_dir(&root).map(|entries| entries.flatten().count()).unwrap_or(0);
        assert_eq!(before, after, "a stale capture writes nothing");
    }

    #[test]
    fn a_changed_linked_design_is_rejected_explicitly_but_a_fresh_one_is_frozen() {
        let dir = scratch("dependency");
        let mut part = Session::default();
        run(&mut part, "solid.box", json!({"length": 20, "width": 10, "height": 5}));
        let part_path = dir.join("Part.sc");
        run(&mut part, "file.save", json!({"path": part_path}));
        let mut assembly = Session::default();
        let inserted = run(&mut assembly, "component.insert_design", json!({"path": part_path}));
        assert!(inserted["component"].is_u64());
        let first = run(&mut assembly, "document.snapshot.capture", json!({}));
        let (captured, verified) = open_captured(&first["candidate"]);
        assert!(!captured.links.is_empty(), "the captured design keeps its link, now to the frozen input");
        assert!(
            verified.files.iter().any(|file| file.path.starts_with("inputs/design-link-")),
            "{:?}",
            verified.files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
        assert!(verified.manifest["inputs"].as_array().is_some_and(|inputs| inputs.iter().any(|input| input["kind"] == "component")));

        // The mutable live file changes: capture must not keep the live path as a fixed input.
        let mut changed = std::fs::read(&part_path).unwrap();
        changed.extend_from_slice(b"\n");
        std::fs::write(&part_path, changed).unwrap();
        let refused = assembly.execute("document.snapshot.capture", &json!({})).unwrap_err().to_string();
        assert!(refused.contains("DEPENDENCY_CHANGED"), "{refused}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
