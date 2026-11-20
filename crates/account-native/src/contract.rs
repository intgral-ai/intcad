//! Frozen rev3 contract pin and vector consumption.
//!
//! The pinned artifacts live in the contract-owning repository, not here. This module ships the
//! pin only. Release validation points `INTCAD_CONTRACT_VECTORS` at the extracted rev3 artifacts
//! and runs `contract_vectors` with `--include-ignored`.
//!
//! A missing artifact never counts as a pass: the vector test is `#[ignore]`, so a normal unit run
//! skips it, and the release gate must explicitly include it. A present-but-mismatched artifact
//! fails.

use std::fmt;
use std::path::Path;

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The reviewed pin, embedded so a normal test always checks it.
pub const PIN_JSON: &str = include_str!("../../../contracts/pin.json");
pub const EXPECTED_ARTIFACT_SLUG: &str = "3c7c6dfefe52";
pub const EXPECTED_PROTOCOL_VERSION: &str = "intcad-auth/1.0";

/// The pin fields the client and the release gate agree on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractPin {
    pub artifact_slug: String,
    pub protocol_version: String,
    pub openapi_version: String,
    pub verification_state: String,
    pub manifest_embedded_sha256: String,
    pub manifest_embedded_bytes: u64,
    pub openapi_embedded_sha256: String,
    pub openapi_embedded_bytes: u64,
    pub openapi_canonical_sha256: String,
    pub fixtures_embedded_sha256: String,
    pub fixtures_embedded_bytes: u64,
    pub fixtures_canonical_sha256: String,
    pub fixture_bundle_version: String,
    pub vector_count: u64,
    pub primary_vector_count: u64,
    pub support_vector_count: u64,
    pub permission_count: u64,
    pub race_matrix_count: u64,
    pub rollback_matrix_count: u64,
    pub operation_id_pattern: String,
}

impl ContractPin {
    /// The only accepted pin. `verification_state` must be `FROZEN`.
    pub fn frozen() -> Result<Self, ContractError> {
        Self::from_json(PIN_JSON)
    }

    pub fn from_json(raw: &str) -> Result<Self, ContractError> {
        let value: Value = serde_json::from_str(raw).map_err(|_| ContractError::MalformedPin("pin.json is not JSON"))?;
        let pin = Self {
            artifact_slug: field_str(&value, "/artifact_slug")?,
            protocol_version: field_str(&value, "/protocol_version")?,
            openapi_version: field_str(&value, "/openapi_version")?,
            verification_state: field_str(&value, "/verification_state")?,
            manifest_embedded_sha256: field_str(&value, "/manifest_embedded_sha256")?,
            manifest_embedded_bytes: field_u64(&value, "/manifest_embedded_bytes")?,
            openapi_embedded_sha256: field_str(&value, "/openapi_embedded_sha256")?,
            openapi_embedded_bytes: field_u64(&value, "/openapi_embedded_bytes")?,
            openapi_canonical_sha256: field_str(&value, "/openapi_canonical_sha256")?,
            fixtures_embedded_sha256: field_str(&value, "/fixtures_embedded_sha256")?,
            fixtures_embedded_bytes: field_u64(&value, "/fixtures_embedded_bytes")?,
            fixtures_canonical_sha256: field_str(&value, "/fixtures_canonical_sha256")?,
            fixture_bundle_version: field_str(&value, "/fixture_bundle_version")?,
            vector_count: field_u64(&value, "/vector_count")?,
            primary_vector_count: field_u64(&value, "/primary_vector_count")?,
            support_vector_count: field_u64(&value, "/support_vector_count")?,
            permission_count: field_u64(&value, "/permission_count")?,
            race_matrix_count: field_u64(&value, "/race_matrix_count")?,
            rollback_matrix_count: field_u64(&value, "/rollback_matrix_count")?,
            operation_id_pattern: field_str(&value, "/operation_id_pattern")?,
        };
        pin.validate()?;
        Ok(pin)
    }

    fn validate(&self) -> Result<(), ContractError> {
        if self.artifact_slug != EXPECTED_ARTIFACT_SLUG {
            return Err(ContractError::PinMismatch("artifact slug is not the reviewed rev3 slug"));
        }
        if self.protocol_version != EXPECTED_PROTOCOL_VERSION {
            return Err(ContractError::PinMismatch("protocol version is not intcad-auth/1.0"));
        }
        if self.verification_state != "FROZEN" {
            return Err(ContractError::NotFrozen);
        }
        for (label, hash) in [
            ("manifest_embedded_sha256", &self.manifest_embedded_sha256),
            ("openapi_embedded_sha256", &self.openapi_embedded_sha256),
            ("openapi_canonical_sha256", &self.openapi_canonical_sha256),
            ("fixtures_embedded_sha256", &self.fixtures_embedded_sha256),
            ("fixtures_canonical_sha256", &self.fixtures_canonical_sha256),
        ] {
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(ContractError::MalformedPin(label));
            }
        }
        Ok(())
    }
}

/// Raw artifact bytes. They are never copied into this crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContractArtifacts {
    manifest: Vec<u8>,
    openapi: Vec<u8>,
    fixtures: Vec<u8>,
}

impl ContractArtifacts {
    /// Load from a directory that contains either:
    /// - the original `doc_content.md` (fenced JSON bodies are extracted byte-for-byte), or
    /// - `embedded_manifest.json`, `embedded_openapi.json`, `embedded_fixtures.json`.
    pub fn load(dir: &Path) -> Result<Self, ContractError> {
        if dir.join("doc_content.md").is_file() {
            let text = std::fs::read_to_string(dir.join("doc_content.md")).map_err(|_| ContractError::MissingArtifacts)?;
            let blocks = extract_fenced_json(&text)?;
            if blocks.len() != 3 {
                return Err(ContractError::MalformedArtifact("doc_content.md must contain exactly three fenced JSON bodies"));
            }
            return Ok(Self { manifest: blocks[0].clone(), openapi: blocks[1].clone(), fixtures: blocks[2].clone() });
        }
        let read = |name: &str| std::fs::read(dir.join(name)).map_err(|_| ContractError::MissingArtifacts);
        let manifest = read("embedded_manifest.json")?;
        let openapi = read("embedded_openapi.json")?;
        let fixtures = read("embedded_fixtures.json")?;
        Ok(Self { manifest, openapi, fixtures })
    }

    /// Verify exact embedded bytes, canonical bytes, versions and counts before any vector runs.
    pub fn verify(self, pin: &ContractPin) -> Result<VerifiedContract, ContractError> {
        require_bytes("manifest", &self.manifest, &pin.manifest_embedded_sha256, pin.manifest_embedded_bytes)?;
        require_bytes("openapi", &self.openapi, &pin.openapi_embedded_sha256, pin.openapi_embedded_bytes)?;
        require_bytes("fixtures", &self.fixtures, &pin.fixtures_embedded_sha256, pin.fixtures_embedded_bytes)?;

        let manifest: Value = parse(&self.manifest, "manifest")?;
        let openapi: Value = parse(&self.openapi, "openapi")?;
        let fixtures: Value = parse(&self.fixtures, "fixtures")?;

        if manifest.get("protocol_version").and_then(Value::as_str) != Some(pin.protocol_version.as_str()) {
            return Err(ContractError::PinMismatch("manifest protocol version differs from the pin"));
        }
        if openapi.pointer("/info/version").and_then(Value::as_str) != Some(pin.openapi_version.as_str()) {
            return Err(ContractError::PinMismatch("OpenAPI info.version differs from the pin"));
        }
        if fixtures.get("bundle_version").and_then(Value::as_str) != Some(pin.fixture_bundle_version.as_str())
            || fixtures.get("protocol_version").and_then(Value::as_str) != Some(pin.protocol_version.as_str())
            || fixtures.get("openapi_canonical_sha256").and_then(Value::as_str) != Some(pin.openapi_canonical_sha256.as_str())
        {
            return Err(ContractError::PinMismatch("fixtures metadata differs from the pin"));
        }
        require_canonical("openapi", &openapi, &pin.openapi_canonical_sha256)?;
        require_canonical("fixtures", &fixtures, &pin.fixtures_canonical_sha256)?;

        let manifest_fixtures = manifest.pointer("/fixtures");
        let fixture_count = fixtures.get("vectors").and_then(Value::as_array).map_or(0, Vec::len) as u64;
        if fixture_count != pin.vector_count {
            return Err(ContractError::CoverageMismatch("vector count differs from the pin"));
        }
        if manifest_fixtures.and_then(|value| value.get("vector_count")).and_then(Value::as_u64) != Some(pin.vector_count)
            || manifest_fixtures.and_then(|value| value.get("primary_vector_count")).and_then(Value::as_u64) != Some(pin.primary_vector_count)
            || manifest_fixtures.and_then(|value| value.get("support_vector_count")).and_then(Value::as_u64) != Some(pin.support_vector_count)
        {
            return Err(ContractError::CoverageMismatch("manifest vector counts differ from the pin"));
        }
        for (name, expected) in [("permissions", pin.permission_count), ("races", pin.race_matrix_count), ("rollbacks", pin.rollback_matrix_count)] {
            let actual = fixtures.get(name).and_then(Value::as_array).map_or(0, Vec::len) as u64;
            if actual != expected {
                return Err(ContractError::CoverageMismatch("a support matrix count differs from the pin"));
            }
        }
        Ok(VerifiedContract { pin: pin.clone(), manifest, openapi, fixtures })
    }
}

/// Artifacts that passed byte, canonical, version and count verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedContract {
    pub pin: ContractPin,
    pub manifest: Value,
    pub openapi: Value,
    pub fixtures: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContractError {
    MissingArtifacts,
    MalformedPin(&'static str),
    MalformedArtifact(&'static str),
    HashMismatch(&'static str),
    CanonicalHashMismatch(&'static str),
    PinMismatch(&'static str),
    NotFrozen,
    CoverageMismatch(&'static str),
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingArtifacts => "the frozen rev3 artifacts are missing",
            Self::MalformedPin(field) => field,
            Self::MalformedArtifact(name) => name,
            Self::HashMismatch(name) => name,
            Self::CanonicalHashMismatch(name) => name,
            Self::PinMismatch(reason) => reason,
            Self::NotFrozen => "the contract pin is not FROZEN",
            Self::CoverageMismatch(reason) => reason,
        })
    }
}

impl std::error::Error for ContractError {}

fn field_str(value: &Value, pointer: &str) -> Result<String, ContractError> {
    value.pointer(pointer).and_then(Value::as_str).map(str::to_owned).ok_or(ContractError::MalformedPin("a pin field is missing"))
}

fn field_u64(value: &Value, pointer: &str) -> Result<u64, ContractError> {
    value.pointer(pointer).and_then(Value::as_u64).ok_or(ContractError::MalformedPin("a numeric pin field is missing"))
}

fn parse(bytes: &[u8], name: &'static str) -> Result<Value, ContractError> {
    serde_json::from_slice(bytes).map_err(|_| ContractError::MalformedArtifact(name))
}

fn require_bytes(name: &'static str, bytes: &[u8], expected: &str, expected_len: u64) -> Result<(), ContractError> {
    if bytes.len() as u64 != expected_len || sha256_hex(bytes) != expected {
        return Err(ContractError::HashMismatch(name));
    }
    Ok(())
}

fn require_canonical(name: &'static str, value: &Value, expected: &str) -> Result<(), ContractError> {
    if sha256_hex(&canonical_json(value)?) != expected {
        return Err(ContractError::CanonicalHashMismatch(name));
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Python `json.dumps(sort_keys=True, separators=(',', ':'))` equivalent for the frozen artifacts.
fn canonical_json(value: &Value) -> Result<Vec<u8>, ContractError> {
    let mut out = Vec::new();
    write_canonical(value, &mut out)?;
    Ok(out)
}

fn write_canonical(value: &Value, out: &mut Vec<u8>) -> Result<(), ContractError> {
    match value {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(flag) => out.extend_from_slice(if *flag { b"true" } else { b"false" }),
        Value::Number(number) => out.extend_from_slice(number.to_string().as_bytes()),
        Value::String(text) => {
            out.extend_from_slice(serde_json::to_string(text).map_err(|_| ContractError::MalformedArtifact("canonical string"))?.as_bytes())
        }
        Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                write_canonical(item, out)?;
            }
            out.push(b']');
        }
        Value::Object(object) => {
            out.push(b'{');
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(b',');
                }
                out.extend_from_slice(serde_json::to_string(key).map_err(|_| ContractError::MalformedArtifact("canonical key"))?.as_bytes());
                out.push(b':');
                write_canonical(&object[key], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

/// Extract the exact bytes between ```` ```json ```` and the following ```` ``` ````. The closing
/// newline is not part of the body, matching the manifest's `embedded_bytes`.
fn extract_fenced_json(text: &str) -> Result<Vec<Vec<u8>>, ContractError> {
    let mut blocks = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```json\n") {
        let after = &rest[start + "```json\n".len()..];
        let Some(end) = after.find("\n```") else {
            return Err(ContractError::MalformedArtifact("an unterminated JSON fence"));
        };
        blocks.push(after.as_bytes()[..end].to_vec());
        rest = &after[end + "\n```".len()..];
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::BTreeSet;
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde_json::{Value, json};
    use solidcraft_engine::entitlement::{
        AccountId, AccountService, Admission, AuthenticatedHostScope, BillingResult, Capability, EntryCategory, Grant, HostSessionBinding,
        InstallationId, MonotonicClock, MonotonicInstant, OperationId, OperationStatus, Refusal, RefusalCode,
    };

    use super::{ContractArtifacts, ContractError, ContractPin, canonical_json};
    use crate::http::ScriptedTransport;
    use crate::http::{BootstrapPlatform, HttpAccountService, InstallationBootstrap, Secret, WireConnection, WireCredentials, WireResponse};

    const T0: u64 = 1_767_225_600_000;
    const BASE: &str = "https://auth.intcad.invalid";

    struct Clock {
        millis: u64,
    }

    impl MonotonicClock for Clock {
        fn now(&self) -> MonotonicInstant {
            MonotonicInstant::from_millis(self.millis)
        }

        fn wall_now(&self) -> SystemTime {
            UNIX_EPOCH + Duration::from_millis(self.millis)
        }
    }

    struct VectorCredentials {
        product: Option<Secret>,
        installation: Option<Secret>,
        settlement: Option<Secret>,
        proof: Option<Secret>,
    }

    impl WireCredentials for VectorCredentials {
        fn product_session(&self) -> Option<Secret> {
            self.product.clone()
        }

        fn installation_credential(&self) -> Option<Secret> {
            self.installation.clone()
        }

        fn settlement_credential(&self, _operation: &OperationId) -> Option<Secret> {
            self.settlement.clone()
        }

        fn execution_session_proof(&self, _operation: &OperationId) -> Option<Secret> {
            self.proof.clone()
        }
    }

    #[test]
    fn contract_pin_is_frozen_and_complete() {
        let pin = ContractPin::frozen().expect("the committed pin must parse");
        assert_eq!(pin.artifact_slug, super::EXPECTED_ARTIFACT_SLUG);
        assert_eq!(pin.protocol_version, super::EXPECTED_PROTOCOL_VERSION);
        assert_eq!(pin.verification_state, "FROZEN");
        assert_eq!(pin.openapi_embedded_bytes, 47_822);
        assert_eq!(pin.fixtures_embedded_bytes, 176_410);
        assert_eq!(pin.vector_count, 68);
        assert_eq!(pin.primary_vector_count, 64);
        assert_eq!(pin.support_vector_count, 4);
        assert_eq!(pin.permission_count, 16);
        assert_eq!(pin.race_matrix_count, 10);
        assert_eq!(pin.rollback_matrix_count, 6);
    }

    #[test]
    fn canonical_hash_matches_the_python_fixture_definition() {
        let value: Value = json!({"b": 2, "a": [1, {"d": 4, "c": 3}], "s": "ü"});
        assert_eq!(canonical_json(&value).unwrap(), "{\"a\":[1,{\"c\":3,\"d\":4}],\"b\":2,\"s\":\"ü\"}".as_bytes());
    }

    #[test]
    fn missing_artifacts_are_an_error_not_a_pass() {
        assert_eq!(ContractArtifacts::load(Path::new("/nonexistent/intcad-contract-rev3")), Err(ContractError::MissingArtifacts));
    }

    #[test]
    #[ignore = "requires INTCAD_CONTRACT_VECTORS pointing at the frozen rev3 artifacts; release gate runs it with --include-ignored"]
    fn contract_vectors() {
        let dir = std::env::var("INTCAD_CONTRACT_VECTORS").expect("INTCAD_CONTRACT_VECTORS must point at the frozen rev3 artifacts");
        let pin = ContractPin::frozen().expect("the committed pin must parse");
        let verified =
            ContractArtifacts::load(Path::new(&dir)).and_then(|artifacts| artifacts.verify(&pin)).expect("frozen rev3 artifacts must match the pin");
        run_vectors(&verified).expect("every applicable frozen vector must pass");
    }

    fn run_vectors(verified: &super::VerifiedContract) -> Result<(), ContractError> {
        let vectors = verified.fixtures.get("vectors").and_then(Value::as_array).ok_or(ContractError::MalformedArtifact("fixtures.vectors"))?;
        let expected_unsupported = expected_unsupported_ids();
        let mut supported = BTreeSet::new();
        let mut unsupported = BTreeSet::new();
        let mut cases = 0usize;
        for vector in vectors {
            let id = vector.get("id").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("vector id"))?;
            if expected_unsupported.contains(id) {
                unsupported.insert(id.to_owned());
                continue;
            }
            supported.insert(id.to_owned());
            for outcome in outcomes(vector)? {
                run_case(vector, &outcome)?;
                cases += 1;
            }
        }
        // 52 applicable vectors, 16 explicitly unsupported responsibilities, and four extra
        // possible-outcome cases (AUTH-004, START-013, EXP-002 has three) = 56 executed cases.
        if supported.len() != 52 || unsupported.len() != 16 || cases != 56 {
            return Err(ContractError::CoverageMismatch("vector coverage differs from the reviewed Task 3 matrix"));
        }
        if unsupported != expected_unsupported {
            return Err(ContractError::CoverageMismatch("the unsupported vector set changed"));
        }
        if verified.fixtures.get("permissions").and_then(Value::as_array).map_or(0, Vec::len) != 16
            || verified.fixtures.get("races").and_then(Value::as_array).map_or(0, Vec::len) != 10
            || verified.fixtures.get("rollbacks").and_then(Value::as_array).map_or(0, Vec::len) != 6
        {
            return Err(ContractError::CoverageMismatch("a support matrix count changed"));
        }
        Ok(())
    }

    fn expected_unsupported_ids() -> BTreeSet<String> {
        [
            "AUTH-007",
            "COMMIT-001",
            "COMMIT-002",
            "COMMIT-003",
            "COMMIT-004",
            "DEL-001",
            "DEL-002",
            "DEL-003",
            "DEL-004",
            "DEL-005",
            "DEL-006",
            "DEL-008",
            "FIN-014",
            "FIN-015",
            "FIN-016",
            "START-005",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    fn outcomes(vector: &Value) -> Result<Vec<Value>, ContractError> {
        if let Some(outcomes) = vector.pointer("/then/possible_outcomes").and_then(Value::as_array) {
            return Ok(outcomes.clone());
        }
        let then = vector.get("then").cloned().ok_or(ContractError::MalformedArtifact("vector then"))?;
        if then.get("http_status").is_some() && then.get("body").is_some() {
            Ok(vec![then])
        } else {
            Err(ContractError::MalformedArtifact("vector outcome"))
        }
    }

    fn run_case(vector: &Value, outcome: &Value) -> Result<(), ContractError> {
        let request = vector.pointer("/when/request").ok_or(ContractError::MalformedArtifact("vector request"))?;
        let given = vector.get("given").cloned().unwrap_or(Value::Null);
        let operation = operation_id(vector, request, outcome)?;
        let scope = scope(&given);
        let clock = clock_ms(vector, outcome);
        let status = outcome.get("http_status").and_then(Value::as_u64).ok_or(ContractError::MalformedArtifact("http status"))? as u16;
        let body = outcome.get("body").ok_or(ContractError::MalformedArtifact("response body"))?;
        let schema = outcome.get("response_schema").and_then(Value::as_str).unwrap_or("");
        let mut credentials = credentials_from(request);
        if outcome.get("request_name").and_then(Value::as_str) == Some("lookup_with_existing_settlement_credential") {
            credentials.settlement = credentials.settlement.or_else(|| Some(Secret::new("settlement_credential_example_0000000000000000")));
        }
        let response = WireResponse::new(status, body.to_string().into_bytes());
        let mut service = HttpAccountService::with_transport(
            ScriptedTransport::new(vec![Ok(response)]),
            WireConnection::new(BASE, "1.0.0").map_err(|_| ContractError::MalformedArtifact("base url"))?,
            Box::new(credentials),
            Box::new(Clock { millis: clock }),
        );
        let observed = observe(&mut service, request, outcome, &scope, &operation)?;
        if (200..300).contains(&status) { verify_success(schema, body, observed, clock) } else { verify_error(body, observed) }
    }

    fn observe(
        service: &mut HttpAccountService<ScriptedTransport>,
        request: &Value,
        outcome: &Value,
        scope: &AuthenticatedHostScope,
        operation: &OperationId,
    ) -> Result<Observed, ContractError> {
        let method = request.get("method").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("method"))?;
        let path = request.get("path").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("path"))?;
        let body = request.get("body").cloned().unwrap_or(Value::Null);
        let request_name = outcome.get("request_name").and_then(Value::as_str);
        if request_name == Some("lookup_with_existing_settlement_credential") {
            return Ok(normalize_status(service.operation(scope, operation)));
        }
        if method == "GET" && path == "/v1/status" {
            return Ok(match service.status() {
                Ok(availability) => Observed::Availability(availability),
                Err(error) => Observed::Refusal(error),
            });
        }
        if method == "POST" && path == "/v1/installations" {
            let platform = match body.get("platform").and_then(Value::as_str) {
                Some("windows") => BootstrapPlatform::Windows,
                Some("macos") => BootstrapPlatform::Macos,
                Some("linux") => BootstrapPlatform::Linux,
                _ => BootstrapPlatform::Unknown,
            };
            let label = body.get("label").and_then(Value::as_str);
            return Ok(match service.bootstrap_installation(scope, platform, label) {
                Ok(bootstrap) => Observed::Installation(bootstrap),
                Err(error) => Observed::Refusal(error),
            });
        }
        if method == "POST" && path == "/v1/authorizations" {
            let capability = capability_of(&body)?;
            return Ok(match body.get("renewal") {
                Some(renewal) => {
                    let authorization_id =
                        renewal.get("authorization_id").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("authorization id"))?;
                    match service.renew(scope, operation, &solidcraft_engine::entitlement::GrantId::new(authorization_id), capability) {
                        Ok(grant) => Observed::Grant(grant),
                        Err(error) => Observed::Refusal(error),
                    }
                }
                None => match service.authorize(scope, capability) {
                    Ok(grant) => Observed::Grant(grant),
                    Err(error) => Observed::Refusal(error),
                },
            });
        }
        if method == "POST" && path == "/v1/operations" {
            let capability = capability_of(&body)?;
            let entry = entry_of(&body)?;
            return Ok(match service.begin(scope, operation, capability, entry) {
                Ok(Admission::Refused(error)) | Err(error) => Observed::Refusal(error),
                Ok(admission) => Observed::Admission(admission),
            });
        }
        if method == "POST" && path.ends_with("/finalize") {
            let result = match body.get("result").and_then(Value::as_str) {
                Some("SUCCEEDED") => BillingResult::Succeeded,
                Some("PARTIAL_RETAINED") => BillingResult::PartialRetained,
                Some("FAILED_NO_EFFECT") => BillingResult::FailedNoEffect,
                Some("UNKNOWN") => BillingResult::Unknown,
                _ => return Err(ContractError::MalformedArtifact("finalize result")),
            };
            return Ok(normalize_status(service.finalize(scope, operation, result)));
        }
        if method == "GET" && path.starts_with("/v1/operations/") {
            return Ok(normalize_status(service.operation(scope, operation)));
        }
        Err(ContractError::MalformedArtifact("unsupported vector endpoint"))
    }

    enum Observed {
        Availability(solidcraft_engine::entitlement::Availability),
        Grant(Grant),
        Admission(Admission),
        Status(OperationStatus),
        Installation(InstallationBootstrap),
        Refusal(Refusal),
    }

    fn normalize_status(result: Result<OperationStatus, Refusal>) -> Observed {
        match result {
            Ok(status) => Observed::Status(status),
            Err(error) => Observed::Refusal(error),
        }
    }

    fn verify_success(schema: &str, body: &Value, observed: Observed, clock: u64) -> Result<(), ContractError> {
        match (schema, observed) {
            ("StatusResponse", Observed::Availability(availability)) => {
                let balance = body.get("balance").and_then(Value::as_u64).unwrap_or(u64::MAX);
                let held = body.get("held").and_then(Value::as_u64).unwrap_or(u64::MAX);
                let available = body.get("available").and_then(Value::as_u64).unwrap_or(u64::MAX);
                if availability.balance != balance || availability.held != held || availability.available() != available {
                    return Err(ContractError::CoverageMismatch("STAT snapshot mismatch"));
                }
                Ok(())
            }
            ("AuthorizationResponse", Observed::Grant(grant)) => {
                let capability = capability_of(body)?;
                let remaining = body.get("remaining_seconds").and_then(Value::as_u64).ok_or(ContractError::MalformedArtifact("remaining seconds"))?;
                if grant.capability() != capability
                    || grant.expires_at() != MonotonicInstant::from_millis(clock.saturating_add(remaining.saturating_mul(1000)))
                {
                    return Err(ContractError::CoverageMismatch("authorization grant mismatch"));
                }
                Ok(())
            }
            ("StartOperationSuccess", Observed::Admission(admission)) => match admission {
                Admission::NewlyAdmitted { operation, grant } => {
                    if body.get("outcome").and_then(Value::as_str) != Some("NEWLY_ADMITTED")
                        || body.get("operation_id").and_then(Value::as_str) != Some(operation.as_str())
                        || grant.capability()
                            != capability_of(body.pointer("/execution_grant").ok_or(ContractError::MalformedArtifact("execution grant"))?)?
                    {
                        return Err(ContractError::CoverageMismatch("NEWLY_ADMITTED mismatch"));
                    }
                    Ok(())
                }
                Admission::AlreadyExists { status } => {
                    let expected = body.get("state").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("state"))?;
                    if body.get("outcome").and_then(Value::as_str) != Some("ALREADY_EXISTS") || status != status_of(expected)? {
                        return Err(ContractError::CoverageMismatch("ALREADY_EXISTS mismatch"));
                    }
                    Ok(())
                }
                other => Err(ContractError::CoverageMismatch(if matches!(other, Admission::Refused(_)) {
                    "unexpected refusal"
                } else {
                    "unexpected admission"
                })),
            },
            ("FinalizeReceipt", Observed::Status(status)) | ("OperationStatusResponse", Observed::Status(status)) => {
                let expected = body.get("state").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("state"))?;
                if status != status_of(expected)? {
                    return Err(ContractError::CoverageMismatch("operation state mismatch"));
                }
                Ok(())
            }
            ("InstallationBootstrapResponse", Observed::Installation(bootstrap)) => {
                let id = body.get("installation_id").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("installation id"))?;
                let secret =
                    body.get("installation_credential").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("installation credential"))?;
                if bootstrap.installation_id() != id || bootstrap.into_credential().expose_secret() != secret {
                    return Err(ContractError::CoverageMismatch("installation bootstrap mismatch"));
                }
                Ok(())
            }
            ("StartOperationConflict", Observed::Refusal(refusal)) => {
                let expected = body.pointer("/error/code").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("error code"))?;
                if Some(refusal.code) != expected_refusal(expected) {
                    return Err(ContractError::CoverageMismatch("start conflict code mismatch"));
                }
                Ok(())
            }
            _ => Err(ContractError::CoverageMismatch("unexpected success shape")),
        }
    }

    fn verify_error(body: &Value, observed: Observed) -> Result<(), ContractError> {
        let Observed::Refusal(refusal) = observed else {
            return Err(ContractError::CoverageMismatch("a failure response produced authority or state"));
        };
        let raw = body.pointer("/error/code").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("error code"))?;
        let expected = match raw {
            // INVALID_REQUEST and INTERNAL_ERROR are transport errors in the engine. This adapter
            // fails closed with AUTH_UNAVAILABLE and never grants.
            "INVALID_REQUEST" | "INTERNAL_ERROR" => RefusalCode::AuthUnavailable,
            other => expected_refusal(other).ok_or(ContractError::MalformedArtifact("unknown error code"))?,
        };
        if refusal.code != expected {
            return Err(ContractError::CoverageMismatch("failure code mismatch"));
        }
        Ok(())
    }

    fn expected_refusal(raw: &str) -> Option<RefusalCode> {
        Some(match raw {
            "AUTH_REQUIRED" => RefusalCode::AuthRequired,
            "ACCOUNT_DISABLED" => RefusalCode::AccountDisabled,
            "EMAIL_UNVERIFIED" => RefusalCode::EmailUnverified,
            "CAPABILITY_DENIED" => RefusalCode::CapabilityDenied,
            "VERSION_UNSUPPORTED" => RefusalCode::VersionUnsupported,
            "PROTOCOL_UNSUPPORTED" => RefusalCode::ProtocolUnsupported,
            "UNSAFE_TO_START" => RefusalCode::UnsafeToStart,
            "CREDITS_EXHAUSTED" => RefusalCode::CreditsExhausted,
            "CREDITS_HELD" => RefusalCode::CreditsHeld,
            "OPERATION_CONFLICT" => RefusalCode::OperationConflict,
            "OPERATION_FINALIZED" => RefusalCode::OperationFinalized,
            "OPERATION_OWNED_ELSEWHERE" => RefusalCode::OperationOwnedElsewhere,
            "AUTH_UNAVAILABLE" => RefusalCode::AuthUnavailable,
            _ => return None,
        })
    }

    fn status_of(raw: &str) -> Result<OperationStatus, ContractError> {
        match raw {
            "HELD" => Ok(OperationStatus::Held),
            "PENDING" => Ok(OperationStatus::Pending),
            "CHARGED" => Ok(OperationStatus::Charged),
            "RELEASED" => Ok(OperationStatus::Released),
            "EXPIRED" => Ok(OperationStatus::Expired),
            _ => Err(ContractError::MalformedArtifact("operation state")),
        }
    }

    fn capability_of(body: &Value) -> Result<Capability, ContractError> {
        let raw = body.get("capability").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("capability"))?;
        Capability::from_wire(raw).ok_or(ContractError::MalformedArtifact("capability value"))
    }

    fn entry_of(body: &Value) -> Result<EntryCategory, ContractError> {
        let raw = body.get("entry_category").and_then(Value::as_str).ok_or(ContractError::MalformedArtifact("entry category"))?;
        Ok(match raw {
            "mastra" => EntryCategory::Mastra,
            "external_acp_mcp" => EntryCategory::ExternalAcpMcp,
            "headless_mcp" => EntryCategory::HeadlessMcp,
            "gui_bridge_mcp" => EntryCategory::GuiBridgeMcp,
            "direct_control" => EntryCategory::DirectControl,
            "cli_dfm" => EntryCategory::CliDfm,
            "review_panel" => EntryCategory::ReviewPanel,
            "external_ui_automation" => EntryCategory::ExternalUiAutomation,
            "unknown" => EntryCategory::Unknown,
            _ => return Err(ContractError::MalformedArtifact("entry category value")),
        })
    }

    fn operation_id(vector: &Value, request: &Value, outcome: &Value) -> Result<OperationId, ContractError> {
        let raw = request
            .get("body")
            .and_then(|body| body.get("operation_id"))
            .and_then(Value::as_str)
            .or_else(|| request.get("path").and_then(Value::as_str).and_then(|path| path.split('/').nth(3)))
            .or_else(|| outcome.get("body").and_then(|body| body.get("operation_id")).and_then(Value::as_str))
            .or_else(|| vector.pointer("/given/operation/id").and_then(Value::as_str))
            .unwrap_or("op_example_0001");
        OperationId::parse(raw).map_err(|_| ContractError::MalformedArtifact("operation id"))
    }

    fn scope(given: &Value) -> AuthenticatedHostScope {
        let account = given.pointer("/account/id").and_then(Value::as_str).unwrap_or("acct_example_0001");
        let installation = given.pointer("/installation/id").and_then(Value::as_str).unwrap_or("inst_example_0001");
        AuthenticatedHostScope::new(AccountId::new(account), InstallationId::new(installation), HostSessionBinding::new("vector_session_binding"))
    }

    fn clock_ms(vector: &Value, outcome: &Value) -> u64 {
        outcome
            .get("virtual_clock_ms")
            .or_else(|| vector.pointer("/when/client_clock/send_anchor_ms"))
            .or_else(|| vector.pointer("/when/client_clock/monotonic_ms"))
            .or_else(|| vector.pointer("/given/virtual_clock_ms"))
            .and_then(Value::as_u64)
            .unwrap_or(T0)
    }

    fn credentials_from(request: &Value) -> VectorCredentials {
        let mut credentials = VectorCredentials { product: None, installation: None, settlement: None, proof: None };
        if let Some(headers) = request.get("headers").and_then(Value::as_object) {
            for (name, value) in headers {
                let Some(value) = value.as_str() else { continue };
                match name.to_ascii_lowercase().as_str() {
                    "authorization" => {
                        credentials.product = value.strip_prefix("Bearer ").map(Secret::new);
                    }
                    "x-installation-credential" => credentials.installation = Some(Secret::new(value)),
                    "x-settlement-credential" => credentials.settlement = Some(Secret::new(value)),
                    "x-execution-session-proof" => credentials.proof = Some(Secret::new(value)),
                    _ => {}
                }
            }
        }
        credentials
    }
}
