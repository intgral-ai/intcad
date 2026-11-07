//! Contract and result types for `document.snapshot.capture` (#11).
//!
//! The wire shapes follow the Mastra contract §8: `CandidateRef` identifies an immutable
//! candidate, and the manifest records the exact persisted input bytes. Revisions are canonical
//! decimal strings, never JSON numbers; digests are `sha256:<64 lowercase hex>`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Capture a snapshot of the live design (no parameters).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct SnapshotCaptureParams {}

/// What `document.snapshot.capture` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SnapshotCaptureResult {
    /// The captured candidate: its source stamp, digests and immutable artifact.
    pub candidate: CandidateRef,
}

/// The document identity and revision the snapshot was captured from. `documentId` is
/// process-local; `candidateId` (not this stamp) is the durable artifact identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotSource {
    /// The live document's identity at capture.
    pub document_id: String,
    /// The design revision as a canonical decimal u64 string (never a JSON number).
    pub revision: String,
}

/// A host-owned immutable artifact reference.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateArtifact {
    /// The artifact's stable identity in the host store.
    pub artifact_id: String,
    /// SHA-256 of the exact persisted container bytes.
    pub digest: String,
    /// Container size in bytes.
    pub size_bytes: u64,
    /// `application/zip`.
    pub media_type: String,
}

/// A captured candidate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateRef {
    /// The candidate's durable identity (host-allocated).
    pub candidate_id: String,
    /// The live state it was captured from.
    pub source: SnapshotSource,
    /// SHA-256 of JCS `{formatVersion, engineVersion, inputs}`.
    pub candidate_digest: String,
    /// SHA-256 of JCS of the complete manifest.
    pub manifest_digest: String,
    /// The immutable container holding the design, manifest and inputs.
    pub artifact: CandidateArtifact,
}

/// The kind of one manifest input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    /// The one native design input.
    Design,
    /// A linked design dependency.
    Component,
    /// An imported file frozen into the candidate.
    Import,
    /// A requirements or protected-dimension baseline.
    Requirements,
}

/// One exact persisted input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestInput {
    /// Normalized relative POSIX path inside the container.
    pub path: String,
    /// What the input is.
    pub kind: InputKind,
    /// Stable logical dependency identity (unique in the manifest).
    pub dependency_id: String,
    /// The upstream version, or null when it has none.
    pub dependency_version: Option<String>,
    /// SHA-256 of the exact persisted bytes.
    pub digest: String,
    /// Persisted size in bytes.
    pub size_bytes: u64,
}

/// The complete snapshot manifest. Neither digest is a field here (no recursive hashing).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnapshotManifest {
    /// Always `"1.0"`.
    pub schema_version: String,
    /// The candidate this manifest describes.
    pub candidate_id: String,
    /// The live state captured.
    pub source: SnapshotSource,
    /// RFC3339 UTC with milliseconds.
    pub captured_at: String,
    /// The native design format of the captured document.
    pub format_version: String,
    /// The build that captured it.
    pub engine_version: String,
    /// Exactly the container's inputs, sorted by path.
    pub inputs: Vec<ManifestInput>,
}
