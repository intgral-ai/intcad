//! `solidcraft-cli validate`: drive the engine's local `ValidationPort` from a scriptable CLI.
//!
//! The request is the contract's `submit` payload (`candidate`, `profile`, `requestId`); the
//! actual `CheckProfile` for a task profile comes from `--profile FILE`. `submit` runs the local
//! headless worker to a terminal state and prints `status`.

use std::path::PathBuf;

use serde_json::Value;

use solidcraft_engine::schema::snapshot::CandidateRef;
use solidcraft_engine::validation::{CheckProfile, ProfileRef, ValidationPort, native_basic_profile};

/// `validate`'s lines in the top-level usage.
pub const USAGE: &str = "  solidcraft-cli validate submit <request.json> [--profile FILE] [--candidate-dir DIR] [--validation-dir DIR] [--json-out FILE]\n  solidcraft-cli validate status <runId> [--candidate-dir DIR] [--validation-dir DIR] [--json-out FILE]\n  solidcraft-cli validate report <runId> [--candidate-dir DIR] [--validation-dir DIR] [--out FILE]\n  solidcraft-cli validate cancel <runId> [--candidate-dir DIR] [--validation-dir DIR] [--json-out FILE]\n      (submit admits the candidate/profile job, runs the local headless worker and prints its status;\n       a quality failure is a terminal report, not a CLI error. --profile is the complete task profile;\n       native-basic needs none. The request is {candidate, profile, requestId} and requestId is\n       '<unix-ms>:<uuid-v4>'. --json-out/--out write the same JSON to a file.)\n";

struct SubmitRequest {
    candidate: CandidateRef,
    profile: ProfileRequest,
    request_id: String,
    owner: Option<String>,
}

struct ProfileRequest {
    profile_id: String,
    version: String,
    digest: Option<String>,
}

fn parse_submit_request(value: &Value) -> Result<SubmitRequest, String> {
    let object = value.as_object().ok_or_else(|| "the request must be a JSON object".to_string())?;
    let candidate: CandidateRef = serde_json::from_value(object.get("candidate").cloned().ok_or_else(|| "the request needs candidate".to_string())?)
        .map_err(|error| format!("candidate: {error}"))?;
    let profile = object.get("profile").and_then(Value::as_object).ok_or_else(|| "the request needs profile".to_string())?;
    let text = |key: &str| profile.get(key).and_then(Value::as_str).map(str::to_string);
    let profile_id = text("profileId").ok_or_else(|| "the request needs profile.profileId".to_string())?;
    let version = text("version").ok_or_else(|| "the request needs profile.version".to_string())?;
    let digest = text("digest").filter(|digest| !digest.is_empty());
    let request_id = object.get("requestId").and_then(Value::as_str).ok_or_else(|| "the request needs requestId".to_string())?.to_string();
    let owner = object.get("owner").and_then(Value::as_str).map(str::to_string);
    Ok(SubmitRequest { candidate, profile: ProfileRequest { profile_id, version, digest }, request_id, owner })
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|arg| arg == name).and_then(|index| args.get(index + 1)).map(String::as_str)
}

fn positional(args: &[String]) -> Vec<&String> {
    let mut out = Vec::new();
    let mut index = 0;
    while let Some(arg) = args.get(index) {
        if arg.starts_with("--") {
            index += if matches!(arg.as_str(), "--json-out" | "--out" | "--profile" | "--candidate-dir" | "--validation-dir") { 2 } else { 1 };
            continue;
        }
        out.push(arg);
        index += 1;
    }
    out
}

fn candidate_dir(args: &[String]) -> PathBuf {
    if let Some(dir) = flag(args, "--candidate-dir") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("SOLIDCRAFT_CANDIDATE_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    solidcraft_engine::recovery::data_dir().map(|dir| dir.join("validation-candidates")).unwrap_or_else(|| PathBuf::from("validation-candidates"))
}

fn validation_dir(args: &[String]) -> PathBuf {
    if let Some(dir) = flag(args, "--validation-dir") {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("SOLIDCRAFT_VALIDATION_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    solidcraft_engine::recovery::data_dir().map(|dir| dir.join("validation")).unwrap_or_else(|| PathBuf::from("validation"))
}

fn output(args: &[String]) -> Option<&str> {
    flag(args, "--json-out").or_else(|| flag(args, "--out"))
}

fn requested_profile(request: &ProfileRequest) -> ProfileRef {
    ProfileRef { profile_id: request.profile_id.clone(), version: request.version.clone(), digest: request.digest.clone().unwrap_or_default() }
}

/// Run one `validate` mode.
pub fn run(args: &[String]) -> Result<(), String> {
    let positionals = positional(args);
    let mode = positionals.first().map(|arg| arg.as_str()).ok_or_else(|| format!("usage:\n{USAGE}"))?;
    let store = validation_dir(args);
    let candidates = candidate_dir(args);
    let port = ValidationPort::open(store, candidates).map_err(|error| error.to_string())?;
    match mode {
        "submit" => {
            let request_path = positionals.get(1).ok_or_else(|| "validate submit needs <request.json>".to_string())?;
            let value: Value = serde_json::from_str(
                &solidcraft_engine::io::vfs::read_to_string(request_path, solidcraft_engine::io::limits::max_design_bytes())
                    .map_err(|error| format!("{request_path}: {error}"))?,
            )
            .map_err(|error| format!("{request_path}: {error}"))?;
            let request = parse_submit_request(&value).map_err(|error| format!("{request_path}: {error}"))?;
            let profile = resolve_profile(args, &request.profile)?;
            let reference = port.register_profile(profile).map_err(|error| error.to_string())?;
            let supplied = requested_profile(&request.profile);
            if !supplied.digest.is_empty() && supplied.digest != reference.digest {
                return Err(format!(
                    "PROFILE_UNSUPPORTED: request profile digest {} does not match the resolved profile {}",
                    supplied.digest, reference.digest
                ));
            }
            if supplied.profile_id != reference.profile_id || supplied.version != reference.version {
                return Err("PROFILE_UNSUPPORTED: request profile identity does not match --profile".to_string());
            }
            let owner = request.owner.as_deref().unwrap_or("local");
            let submitted = port.submit_for(&request.candidate, &reference, &request.request_id, owner).map_err(|error| error.to_string())?;
            port.pump().map_err(|error| error.to_string())?;
            let status = port.status(&submitted.run_id).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?, output(args))
        }
        "status" => {
            let run_id = positionals.get(1).ok_or_else(|| "validate status needs <runId>".to_string())?;
            let status = port.status(run_id).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?, output(args))
        }
        "report" => {
            let run_id = positionals.get(1).ok_or_else(|| "validate report needs <runId>".to_string())?;
            let report = port.report(run_id).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(report).map_err(|error| error.to_string())?, output(args))
        }
        "cancel" => {
            let run_id = positionals.get(1).ok_or_else(|| "validate cancel needs <runId>".to_string())?;
            let cancelled = port.cancel(run_id).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(cancelled).map_err(|error| error.to_string())?, output(args))
        }
        other => Err(format!("validate: unknown mode `{other}`\nusage:\n{USAGE}")),
    }
}

fn resolve_profile(args: &[String], requested: &ProfileRequest) -> Result<CheckProfile, String> {
    let file = flag(args, "--profile");
    match file {
        Some(path) => {
            let bytes = solidcraft_engine::io::vfs::read_to_string(path, solidcraft_engine::io::limits::max_design_bytes())
                .map_err(|error| format!("{path}: {error}"))?;
            let profile: CheckProfile = serde_json::from_str(&bytes).map_err(|error| format!("{path}: {error}"))?;
            if profile.profile_id != requested.profile_id || profile.version != requested.version {
                return Err(format!(
                    "PROFILE_UNSUPPORTED: --profile is {} {}, the request asks for {} {}",
                    profile.profile_id, profile.version, requested.profile_id, requested.version
                ));
            }
            Ok(profile)
        }
        None => {
            if requested.profile_id == solidcraft_engine::validation::NATIVE_BASIC_ID
                && requested.version == solidcraft_engine::validation::NATIVE_BASIC_VERSION
            {
                return Ok(native_basic_profile());
            }
            // A task profile must be supplied; the host never invents requirements.
            Err(format!(
                "PROFILE_UNSUPPORTED: {} {} is not native-basic; pass its complete profile with --profile",
                requested.profile_id, requested.version
            ))
        }
    }
}

fn print_json(value: &Value, file: Option<&str>) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    text.push('\n');
    match file {
        Some(path) => std::fs::write(path, text).map_err(|error| format!("{path}: {error}")),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}
