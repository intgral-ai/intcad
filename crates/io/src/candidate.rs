//! Host-owned immutable validation candidates (#11): a native design snapshot, its manifest and
//! every frozen input in one container.
//!
//! A candidate is a zip file whose entries are exactly the manifest's inputs plus
//! `manifest.json`. The zip is published with [`crate::safe_write::create_atomic`], so an
//! incomplete capture is never visible as a valid candidate, and a candidate is never replaced.
//! Beside it, `<candidateId>.json` records the artifact, candidate and manifest digests: that
//! record is the durable lookup, and verification re-reads the exact persisted bytes rather
//! than trusting it. `<candidateId>.pin` marks an active reference; cleanup removes only
//! unpinned, expired candidates.
//!
//! Hash rules follow the Mastra contract (§8): every input digest is SHA-256 of its exact
//! persisted bytes, `candidateDigest` is SHA-256 of the RFC 8785 (JCS) UTF-8 encoding of
//! `{formatVersion, engineVersion, inputs}`, and `manifestDigest` is SHA-256 of the JCS encoding
//! of the complete manifest. The artifact digest hashes the exact container bytes.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::{IoError, Result};

/// The artifact's media type: one zip container per candidate.
pub const MEDIA_TYPE: &str = "application/zip";
/// The manifest entry inside the container.
pub const MANIFEST_ENTRY: &str = "manifest.json";
/// The contract's snapshot ceilings.
pub const MAX_INPUTS: usize = 256;
/// 1 GiB of persisted input.
pub const MAX_INPUT_BYTES: u64 = 1 << 30;
/// 1 MiB manifest.
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// A zip adds headers and the central directory; leave room above the input ceiling.
const MAX_CONTAINER_BYTES: u64 = MAX_INPUT_BYTES + (64 << 20);
/// The artifact store's ceiling (contract §4).
pub const MAX_STORE_BYTES: u64 = 8 << 30;
/// Completed captures retained seven days (contract §4).
pub const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// One persisted input: its normalized relative POSIX path inside the container and its exact
/// bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// An immutable artifact reference, wire-shaped (`sha256:` digests, camelCase).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredArtifact {
    pub artifact_id: String,
    pub digest: String,
    pub size_bytes: u64,
    pub media_type: String,
}

/// What `<candidateId>.json` records, and what [`lookup`] returns after verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredCandidate {
    pub artifact: StoredArtifact,
    pub candidate_digest: String,
    pub manifest_digest: String,
}

/// A verified candidate: its record, manifest and exact persisted inputs.
#[derive(Clone, Debug)]
pub struct VerifiedCandidate {
    pub candidate: StoredCandidate,
    pub manifest: Value,
    pub files: Vec<CandidateFile>,
}

fn invalid(message: impl std::fmt::Display) -> IoError {
    IoError::Invalid(format!("candidate: {message}"))
}

/// `sha256:<lowercase hex>` of `bytes`.
pub fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// The bare lowercase hex SHA-256 of `bytes` (what design links keep).
pub fn digest_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// RFC 8785 JSON Canonicalization Scheme encoding for the values a manifest uses: null, booleans,
/// strings, arrays, objects and integers. Floats are refused: a candidate manifest never needs
/// one, and ECMAScript number formatting is the part of JCS this host must not guess at.
pub fn canonical_json(value: &Value) -> Result<Vec<u8>> {
    let mut out = String::new();
    write_canonical(&mut out, value)?;
    Ok(out.into_bytes())
}

fn write_canonical(out: &mut String, value: &Value) -> Result<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                out.push_str(&integer.to_string());
            } else if let Some(integer) = number.as_u64() {
                out.push_str(&integer.to_string());
            } else {
                return Err(invalid("JCS: only integer numbers are canonicalizable here"));
            }
        }
        // serde_json's string escaping is exactly JCS's for Rust UTF-8 text: control characters
        // use the short escapes or \u00XX, and quote and backslash are escaped.
        Value::String(text) => {
            let escaped = serde_json::to_string(text).map_err(|e| invalid(format!("JCS string: {e}")))?;
            out.push_str(&escaped);
        }
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(out, item)?;
            }
            out.push(']');
        }
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            // JCS orders by UTF-16 code units, not UTF-8 bytes.
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(out, &Value::String(key.clone()))?;
                out.push(':');
                if let Some(item) = object.get(key) {
                    write_canonical(out, item)?;
                }
            }
            out.push('}');
        }
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}

fn valid_digest(text: &str) -> bool {
    text.strip_prefix("sha256:").is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// A normalized relative POSIX path: no absolute path, `..`, empty component, backslash, NUL or
/// symlink escape (container entries are never extracted, so an entry name cannot leave it).
fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && !path.contains(['\\', '\0'])
        && !path.starts_with('/')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn valid_revision(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 20
        && (text.len() == 1 || !text.starts_with('0'))
        && text.bytes().all(|b| b.is_ascii_digit())
        && text.parse::<u64>().is_ok()
}

fn exact_keys(object: &serde_json::Map<String, Value>, allowed: &[&str]) -> bool {
    object.len() == allowed.len() && object.keys().all(|key| allowed.contains(&key.as_str()))
}

/// Validate a manifest and return its inputs as `(path, digest, sizeBytes)`.
fn manifest_inputs(candidate_id: &str, manifest: &Value) -> Result<Vec<(String, String, u64)>> {
    let object = manifest.as_object().ok_or_else(|| invalid("manifest must be an object"))?;
    if !exact_keys(object, &["schemaVersion", "candidateId", "source", "capturedAt", "formatVersion", "engineVersion", "inputs"]) {
        return Err(invalid("manifest has missing or unknown fields"));
    }
    if object.get("schemaVersion").and_then(Value::as_str) != Some("1.0") {
        return Err(invalid("manifest schemaVersion must be \"1.0\""));
    }
    if object.get("candidateId").and_then(Value::as_str) != Some(candidate_id) {
        return Err(invalid("manifest candidateId does not name this candidate"));
    }
    let source = object.get("source").and_then(Value::as_object).ok_or_else(|| invalid("manifest source must be an object"))?;
    if !exact_keys(source, &["documentId", "revision"]) {
        return Err(invalid("manifest source has missing or unknown fields"));
    }
    if !source.get("documentId").and_then(Value::as_str).is_some_and(valid_id) {
        return Err(invalid("manifest source.documentId is not a valid Id"));
    }
    if !source.get("revision").and_then(Value::as_str).is_some_and(valid_revision) {
        return Err(invalid("manifest source.revision must be a canonical decimal string"));
    }
    for key in ["capturedAt", "formatVersion", "engineVersion"] {
        if !object.get(key).and_then(Value::as_str).is_some_and(|text| !text.is_empty() && text.len() <= 240) {
            return Err(invalid(format!("manifest {key} must be non-empty text")));
        }
    }
    let inputs = object.get("inputs").and_then(Value::as_array).ok_or_else(|| invalid("manifest inputs must be an array"))?;
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(invalid(format!("manifest must have 1..={MAX_INPUTS} inputs")));
    }
    let mut previous: Option<&str> = None;
    let mut dependency_ids = BTreeSet::new();
    let mut designs = 0;
    let mut total = 0_u64;
    let mut rows = Vec::with_capacity(inputs.len());
    for input in inputs {
        let input = input.as_object().ok_or_else(|| invalid("manifest input must be an object"))?;
        if !exact_keys(input, &["path", "kind", "dependencyId", "dependencyVersion", "digest", "sizeBytes"]) {
            return Err(invalid("manifest input has missing or unknown fields"));
        }
        let path = input.get("path").and_then(Value::as_str).ok_or_else(|| invalid("input path must be text"))?;
        if !valid_path(path) || previous.is_some_and(|previous| previous >= path) {
            return Err(invalid(format!("input paths must be unique, sorted relative POSIX paths: {path:?}")));
        }
        previous = Some(path);
        match input.get("kind").and_then(Value::as_str) {
            Some("design") => designs += 1,
            Some("component" | "import" | "requirements") => {}
            _ => return Err(invalid(format!("input {path} has an unknown kind"))),
        }
        let dependency_id = input.get("dependencyId").and_then(Value::as_str).ok_or_else(|| invalid("input dependencyId must be text"))?;
        if !valid_id(dependency_id) || !dependency_ids.insert(dependency_id.to_string()) {
            return Err(invalid(format!("input dependencyIds must be unique Ids: {dependency_id:?}")));
        }
        match input.get("dependencyVersion") {
            Some(Value::Null) | Some(Value::String(_)) => {}
            _ => return Err(invalid(format!("input {path} dependencyVersion must be text or null"))),
        }
        let digest = input.get("digest").and_then(Value::as_str).ok_or_else(|| invalid("input digest must be text"))?;
        if !valid_digest(digest) {
            return Err(invalid(format!("input {path} digest must be sha256:<64 lowercase hex>")));
        }
        let size = input.get("sizeBytes").and_then(Value::as_u64).ok_or_else(|| invalid("input sizeBytes must be an integer"))?;
        total = total.checked_add(size).filter(|total| *total <= MAX_INPUT_BYTES).ok_or_else(|| invalid("inputs exceed 1 GiB"))?;
        rows.push((path.to_string(), digest.to_string(), size));
    }
    if designs != 1 {
        return Err(invalid("manifest must have exactly one design input"));
    }
    Ok(rows)
}

fn candidate_digest(manifest: &Value) -> Result<String> {
    let object = manifest.as_object().ok_or_else(|| invalid("manifest must be an object"))?;
    let subset = json!({
        "formatVersion": object.get("formatVersion").cloned().unwrap_or(Value::Null),
        "engineVersion": object.get("engineVersion").cloned().unwrap_or(Value::Null),
        "inputs": object.get("inputs").cloned().unwrap_or(Value::Null),
    });
    Ok(digest(&canonical_json(&subset)?))
}

fn manifest_digest(manifest: &Value) -> Result<String> {
    Ok(digest(&canonical_json(manifest)?))
}

fn container_path(root: &Path, candidate_id: &str) -> PathBuf {
    root.join(format!("{candidate_id}.zip"))
}

/// Bytes already in the store (containers only; records and pins are tiny).
fn store_bytes(root: &Path) -> u64 {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("zip"))
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .fold(0_u64, u64::saturating_add)
}

fn record_path(root: &Path, candidate_id: &str) -> PathBuf {
    root.join(format!("{candidate_id}.json"))
}

fn pin_path(root: &Path, candidate_id: &str) -> PathBuf {
    root.join(format!("{candidate_id}.pin"))
}

fn checked_candidate_id(candidate_id: &str) -> Result<()> {
    if valid_id(candidate_id) { Ok(()) } else { Err(invalid("candidateId must be a valid Id")) }
}

fn zip_bytes(manifest: &[u8], files: &[&CandidateFile]) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let write_entry = |zip: &mut zip::ZipWriter<Cursor<Vec<u8>>>, name: &str, bytes: &[u8]| -> Result<()> {
        zip.start_file(name, options).map_err(|e| invalid(format!("container {name}: {e}")))?;
        zip.write_all(bytes).map_err(|e| invalid(format!("container {name}: {e}")))
    };
    write_entry(&mut zip, MANIFEST_ENTRY, manifest)?;
    for file in files {
        write_entry(&mut zip, &file.path, &file.bytes)?;
    }
    let cursor = zip.finish().map_err(|e| invalid(format!("container: {e}")))?;
    Ok(cursor.into_inner())
}

/// Publish a candidate atomically. `root` is created if needed; the container and its record are
/// never replaced. The returned record is what a later [`lookup`] verifies.
pub fn write(root: &Path, candidate_id: &str, files: &[CandidateFile], manifest: &Value, artifact_id: &str) -> Result<StoredCandidate> {
    checked_candidate_id(candidate_id)?;
    if !valid_id(artifact_id) {
        return Err(invalid("artifactId must be a valid Id"));
    }
    let inputs = manifest_inputs(candidate_id, manifest)?;
    let mut sorted: Vec<&CandidateFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    for pair in sorted.windows(2) {
        if pair[0].path == pair[1].path {
            return Err(invalid(format!("duplicate persisted input {}", pair[0].path)));
        }
    }
    if sorted.len() != inputs.len() {
        return Err(invalid("persisted inputs do not match the manifest"));
    }
    let mut total = 0_u64;
    for (file, (path, expected_digest, expected_size)) in sorted.iter().zip(&inputs) {
        if &file.path != path {
            return Err(invalid(format!("persisted input {} is not manifest input {path}", file.path)));
        }
        let actual = digest(&file.bytes);
        let size = file.bytes.len() as u64;
        if &actual != expected_digest || size != *expected_size {
            return Err(invalid(format!("input {path} digest or size does not match the manifest")));
        }
        total = total.checked_add(size).filter(|total| *total <= MAX_INPUT_BYTES).ok_or_else(|| invalid("inputs exceed 1 GiB"))?;
    }
    let manifest_bytes = serde_json::to_vec(manifest).map_err(|e| invalid(format!("manifest: {e}")))?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(invalid("manifest exceeds 1 MiB"));
    }
    let container = zip_bytes(&manifest_bytes, &sorted)?;
    if container.len() as u64 > MAX_CONTAINER_BYTES {
        return Err(invalid("container exceeds the snapshot ceiling"));
    }
    // Store pressure refuses new work; it never deletes active evidence.
    if store_bytes(root).saturating_add(container.len() as u64) > MAX_STORE_BYTES {
        return Err(invalid("the artifact store is full: remove expired candidates first"));
    }
    fs::create_dir_all(root).map_err(|e| invalid(format!("store {}: {e}", root.display())))?;
    let path = container_path(root, candidate_id);
    crate::safe_write::create_atomic(&path, &container).map_err(|e| invalid(format!("publishing {}: {e}", path.display())))?;
    let record = StoredCandidate {
        artifact: StoredArtifact {
            artifact_id: artifact_id.to_string(),
            digest: digest(&container),
            size_bytes: container.len() as u64,
            media_type: MEDIA_TYPE.into(),
        },
        candidate_digest: candidate_digest(manifest)?,
        manifest_digest: manifest_digest(manifest)?,
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(|e| invalid(format!("record: {e}")))?;
    if let Err(error) = crate::safe_write::create_atomic(&record_path(root, candidate_id), &bytes) {
        // Never leave a container without its lookup record.
        let _ = fs::remove_file(&path);
        return Err(invalid(format!("publishing record: {error}")));
    }
    Ok(record)
}

/// Reopen and verify a candidate by ID after a restart. A missing record or container is
/// `NOT_FOUND`; a changed container, manifest or input fails integrity verification.
pub fn lookup(root: &Path, candidate_id: &str) -> Result<VerifiedCandidate> {
    checked_candidate_id(candidate_id)?;
    let record_path = record_path(root, candidate_id);
    let record_bytes = read_bounded(&record_path, 1 << 20).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { IoError::Invalid(format!("NOT_FOUND: candidate {candidate_id}")) } else { invalid(e) }
    })?;
    let record: StoredCandidate = serde_json::from_slice(&record_bytes).map_err(|_| invalid("candidate record is not valid JSON"))?;
    if record.artifact.media_type != MEDIA_TYPE {
        return Err(invalid(format!("candidate {candidate_id} is not a {MEDIA_TYPE} artifact")));
    }
    let path = container_path(root, candidate_id);
    let container = read_bounded(&path, MAX_CONTAINER_BYTES).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { IoError::Invalid(format!("NOT_FOUND: candidate {candidate_id}")) } else { invalid(e) }
    })?;
    if container.len() as u64 != record.artifact.size_bytes || digest(&container) != record.artifact.digest {
        return Err(invalid(format!("candidate {candidate_id} container was modified")));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(container)).map_err(|e| invalid(format!("candidate container: {e}")))?;
    let manifest_bytes = {
        let mut entry = archive.by_name(MANIFEST_ENTRY).map_err(|_| invalid("candidate container has no manifest.json"))?;
        if entry.size() > MAX_MANIFEST_BYTES as u64 {
            return Err(invalid("manifest exceeds 1 MiB"));
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| invalid(format!("manifest: {e}")))?;
        bytes
    };
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|_| invalid("manifest is not valid JSON"))?;
    let inputs = manifest_inputs(candidate_id, &manifest)?;
    if candidate_digest(&manifest)? != record.candidate_digest || manifest_digest(&manifest)? != record.manifest_digest {
        return Err(invalid(format!("candidate {candidate_id} manifest digests do not match its record")));
    }
    let mut files = Vec::with_capacity(inputs.len());
    let mut total = 0_u64;
    for (path, expected_digest, expected_size) in inputs {
        let mut entry = archive.by_name(&path).map_err(|_| invalid(format!("candidate container has no input {path}")))?;
        if entry.size() != expected_size {
            return Err(invalid(format!("input {path} size does not match the manifest")));
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes).map_err(|e| invalid(format!("input {path}: {e}")))?;
        if digest(&bytes) != expected_digest {
            return Err(invalid(format!("input {path} was modified")));
        }
        total = total.checked_add(bytes.len() as u64).filter(|total| *total <= MAX_INPUT_BYTES).ok_or_else(|| invalid("inputs exceed 1 GiB"))?;
        files.push(CandidateFile { path, bytes });
    }
    Ok(VerifiedCandidate { candidate: record, manifest, files })
}

fn read_bounded(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    if metadata.len() > limit {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
    }
    Ok(bytes)
}

/// Pin a candidate while a validation run refers to it. Pinning is idempotent.
pub fn pin(root: &Path, candidate_id: &str) -> Result<()> {
    checked_candidate_id(candidate_id)?;
    fs::create_dir_all(root).map_err(|e| invalid(format!("store {}: {e}", root.display())))?;
    match crate::safe_write::create_atomic(&pin_path(root, candidate_id), b"active\n") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(invalid(format!("pinning {candidate_id}: {error}"))),
    }
}

/// Release a pin. Missing pins are already released.
pub fn unpin(root: &Path, candidate_id: &str) -> Result<()> {
    checked_candidate_id(candidate_id)?;
    match fs::remove_file(pin_path(root, candidate_id)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(invalid(format!("unpinning {candidate_id}: {error}"))),
    }
}

/// Is the candidate pinned by an active reference?
pub fn is_pinned(root: &Path, candidate_id: &str) -> bool {
    valid_id(candidate_id) && pin_path(root, candidate_id).is_file()
}

/// Remove unpinned candidates older than `max_age`. An active (pinned) candidate is never
/// removed, so cleanup cannot invalidate a running job. Returns how many candidates went.
pub fn cleanup(root: &Path, now: SystemTime, max_age: Duration) -> usize {
    let Ok(entries) = fs::read_dir(root) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("zip") {
            continue;
        }
        let Some(candidate_id) = path.file_stem().and_then(|s| s.to_str()).filter(|id| valid_id(id)) else { continue };
        if is_pinned(root, candidate_id) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| now.duration_since(modified).unwrap_or(Duration::ZERO) >= max_age);
        if !old {
            continue;
        }
        let container = fs::remove_file(&path).is_ok();
        let record = fs::remove_file(record_path(root, candidate_id)).is_ok();
        if container || record {
            removed += 1;
        }
    }
    // A record without a container is not a candidate; clear the remains of an interrupted write.
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(candidate_id) = path.file_stem().and_then(|s| s.to_str()).filter(|id| valid_id(id)) else { continue };
            if !is_pinned(root, candidate_id) && !container_path(root, candidate_id).exists() {
                let _ = fs::remove_file(path);
            }
        }
    }
    removed
}

/// The bytes and record of a candidate, for a host that needs the container itself.
pub fn container_bytes(root: &Path, candidate_id: &str) -> Result<Vec<u8>> {
    checked_candidate_id(candidate_id)?;
    read_bounded(&container_path(root, candidate_id), MAX_CONTAINER_BYTES).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound { IoError::Invalid(format!("NOT_FOUND: candidate {candidate_id}")) } else { invalid(e) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("solidcraft-candidate-{name}-{}-{nanos}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manifest(candidate_id: &str, design: &[u8]) -> Value {
        json!({
            "schemaVersion": "1.0",
            "candidateId": candidate_id,
            "source": {"documentId": "doc-demo", "revision": "42"},
            "capturedAt": "2026-11-07T12:00:00.000Z",
            "formatVersion": "solidcraft/50",
            "engineVersion": "0.40.0",
            "inputs": [{
                "path": "design.sc",
                "kind": "design",
                "dependencyId": "design-root",
                "dependencyVersion": null,
                "digest": digest(design),
                "sizeBytes": design.len() as u64,
            }],
        })
    }

    #[test]
    fn jcs_known_vectors_sort_keys_and_keep_exact_integers() {
        let value = json!({"b": 2, "a": "x", "nested": {"z": null, "y": ["é", 1, true]}});
        assert_eq!(String::from_utf8(canonical_json(&value).unwrap()).unwrap(), "{\"a\":\"x\",\"b\":2,\"nested\":{\"y\":[\"é\",1,true],\"z\":null}}");
        assert_eq!(
            digest(&canonical_json(&json!({"a": "x", "b": 1})).unwrap()),
            "sha256:cdab067e9f3beb32d1252cfd63e492592fecbf591b0d08cadb24bb17f3864246"
        );
        assert!(canonical_json(&json!({"f": 1.5})).is_err());
    }

    #[test]
    fn write_lookup_and_verify_round_trip_the_exact_bytes() {
        let root = scratch("roundtrip");
        let design = b"native design bytes";
        let candidate_id = "cand-roundtrip";
        let record = write(
            &root,
            candidate_id,
            &[CandidateFile { path: "design.sc".into(), bytes: design.to_vec() }],
            &manifest(candidate_id, design),
            "artifact-roundtrip",
        )
        .unwrap();
        assert_eq!(record.manifest_digest, manifest_digest(&manifest(candidate_id, design)).unwrap());
        let verified = lookup(&root, candidate_id).unwrap();
        assert_eq!(verified.candidate, record);
        assert_eq!(verified.files, [CandidateFile { path: "design.sc".into(), bytes: design.to_vec() }]);
        assert_eq!(verified.manifest["source"]["revision"], "42");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_modified_container_fails_integrity_verification() {
        let root = scratch("modified");
        let design = b"native design bytes";
        let candidate_id = "cand-modified";
        write(
            &root,
            candidate_id,
            &[CandidateFile { path: "design.sc".into(), bytes: design.to_vec() }],
            &manifest(candidate_id, design),
            "artifact-modified",
        )
        .unwrap();
        let path = container_path(&root, candidate_id);
        let mut bytes = fs::read(&path).unwrap();
        *bytes.last_mut().unwrap() ^= 0xff;
        fs::write(&path, bytes).unwrap();
        let error = lookup(&root, candidate_id).unwrap_err().to_string();
        assert!(error.contains("modified"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_pinned_candidate_survives_cleanup_and_an_unpinned_one_does_not() {
        let root = scratch("retention");
        let design = b"native design bytes";
        for candidate_id in ["cand-pinned", "cand-expired"] {
            write(
                &root,
                candidate_id,
                &[CandidateFile { path: "design.sc".into(), bytes: design.to_vec() }],
                &manifest(candidate_id, design),
                &format!("artifact-{candidate_id}"),
            )
            .unwrap();
        }
        pin(&root, "cand-pinned").unwrap();
        assert_eq!(cleanup(&root, SystemTime::now(), Duration::ZERO), 1);
        assert!(lookup(&root, "cand-pinned").is_ok());
        assert!(lookup(&root, "cand-expired").is_err());
        unpin(&root, "cand-pinned").unwrap();
        assert_eq!(cleanup(&root, SystemTime::now(), Duration::ZERO), 1);
        assert!(lookup(&root, "cand-pinned").is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hostile_manifest_paths_and_digests_are_refused() {
        let root = scratch("hostile");
        let design = b"x";
        let mut bad = manifest("cand-hostile", design);
        bad["inputs"][0]["path"] = json!("../escape.sc");
        assert!(
            write(&root, "cand-hostile", &[CandidateFile { path: "../escape.sc".into(), bytes: design.to_vec() }], &bad, "artifact-hostile").is_err()
        );
        let mut bad = manifest("cand-hostile", design);
        bad["inputs"][0]["digest"] = json!("sha256:XYZ");
        assert!(
            write(&root, "cand-hostile", &[CandidateFile { path: "design.sc".into(), bytes: design.to_vec() }], &bad, "artifact-hostile").is_err()
        );
        let mut bad = manifest("cand-hostile", design);
        bad["source"]["revision"] = json!("01");
        assert!(
            write(&root, "cand-hostile", &[CandidateFile { path: "design.sc".into(), bytes: design.to_vec() }], &bad, "artifact-hostile").is_err()
        );
        let _ = fs::remove_dir_all(root);
    }
}
