//! `intcad-cli validate`: drive the engine's local `ValidationPort` from a scriptable CLI.
//!
//! The request is the contract's `submit` payload (`candidate`, `profile`, `requestId`); the
//! actual `CheckProfile` for a task profile comes from `--profile FILE`. `submit` runs the local
//! headless worker to a terminal state and prints `status`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use solidcraft_engine::entitlement::{
    BillingPort, BillingResult, BudgetTable, Capability, EntryCategory, Grant, OperationId, Refusal, RefusalCode, UpperBound,
};
use solidcraft_engine::schema::snapshot::CandidateRef;
use solidcraft_engine::validation::{
    CheckProfile, Limits, LiveDocument, LocalRunner, ProfileRef, RunControl, RunJob, Runner, RunnerIdentity, RunnerOutcome, SkillValidationRequest,
    SystemClock, ValidationPort, ValidationState, generate_run_id, native_basic_profile, now_ms, profile_ref, text_to_cad_profile_ref,
    validate_skill_snapshot,
};

/// `validate`'s lines in the top-level usage.
pub const USAGE: &str = "  intcad-cli validate submit <request.json> [--profile FILE] [--candidate-dir DIR] [--validation-dir DIR] [--json-out FILE]\n  intcad-cli validate skill <request.json> --profile FILE [--candidate-dir DIR] [--validation-dir DIR] [--json-out FILE]\n  intcad-cli validate status <runId> [--candidate-dir DIR] [--validation-dir DIR] [--json-out FILE]\n  intcad-cli validate report <runId> [--candidate-dir DIR] [--validation-dir DIR] [--out FILE]\n  intcad-cli validate cancel <runId> [--candidate-dir DIR] [--validation-dir DIR] [--json-out FILE]\n      (submit admits the candidate/profile job, runs the local headless worker and prints its status;\n       skill uses the same port through the UI-free skill path, requires live {documentId, revision}\n       and prints the bound SkillValidationReport. A quality failure is a terminal report, not a CLI\n       error. --profile is the complete task profile; native-basic needs none. The request is\n       {candidate, profile, requestId[, live]} and requestId is '<unix-ms>:<uuid-v4>'. --json-out/--out\n       write the same JSON to a file. A resolved profile containing a dfm.* check is one controlled\n       check: it needs an installed authority and a verified safe-stop bound, produces one report\n       (violations or not) and settles once; when unauthorized it is refused whole with a stable\n       code (AUTH_UNAVAILABLE/UNSAFE_TO_START) and the DFM part is never silently skipped. Plain\n       validation stays free and offline. Nothing retries automatically.)\n";

struct SubmitRequest {
    candidate: CandidateRef,
    profile: ProfileRequest,
    request_id: String,
    owner: Option<String>,
    live: Option<LiveDocument>,
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
    let live = object.get("live").map(|value| serde_json::from_value(value.clone()).map_err(|error| format!("live: {error}"))).transpose()?;
    Ok(SubmitRequest { candidate, profile: ProfileRequest { profile_id, version, digest }, request_id, owner, live })
}

fn read_request(path: &str) -> Result<SubmitRequest, String> {
    let value: Value = serde_json::from_str(
        &solidcraft_engine::io::vfs::read_to_string(path, solidcraft_engine::io::limits::max_design_bytes())
            .map_err(|error| format!("{path}: {error}"))?,
    )
    .map_err(|error| format!("{path}: {error}"))?;
    parse_submit_request(&value).map_err(|error| format!("{path}: {error}"))
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

/// What a resolved profile contains. The class comes from the resolved profile, never from the
/// request's profile name, profile id or request id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileScope {
    /// No DFM check: ordinary validation stays free, offline and anonymous.
    Free,
    /// At least one DFM check: the whole mixed request is one controlled check.
    Dfm,
}

/// Classify a resolved profile conservatively.
///
/// A check whose kind or id starts with `dfm.` is a DFM check. Unknown profile text is not trusted
/// to hide a DFM requirement: a host profile that names one is classified controlled even when
/// the runner would later mark that kind unsupported.
pub fn profile_scope(profile: &CheckProfile) -> ProfileScope {
    if profile.checks.iter().any(|check| check.kind.starts_with("dfm.") || check.check_id.starts_with("dfm.")) {
        ProfileScope::Dfm
    } else {
        ProfileScope::Free
    }
}

/// A host-minted operation for one CLI check. `None` when this build has no host entropy.
#[cfg(not(target_arch = "wasm32"))]
pub fn mint_operation() -> Option<OperationId> {
    let mut entropy = [0_u8; 16];
    getrandom::fill(&mut entropy).ok()?;
    OperationId::mint_from_entropy(&entropy).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn mint_operation() -> Option<OperationId> {
    None
}

/// The only billing authority until a host composition supplies the real one. It never grants;
/// a mixed DFM check therefore fails closed with a stable refusal instead of running unbilled.
pub struct NoAuthorityBilling;

impl BillingPort for NoAuthorityBilling {
    fn admit(&mut self, _operation: &OperationId, _capability: Capability, _entry: EntryCategory) -> Result<Grant, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "no host billing authority is installed for a controlled CLI check"))
    }

    fn commit(&mut self, _operation: &OperationId, _result: BillingResult) -> Result<BillingResult, Refusal> {
        Err(Refusal::new(RefusalCode::AuthUnavailable, "no host billing authority is installed for a controlled CLI check"))
    }

    fn release(&mut self, _operation: &OperationId, _reason: RefusalCode) -> Result<(), Refusal> {
        Ok(())
    }
}

/// What a controlled CLI check completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlledCheck {
    pub run_id: String,
    pub state: ValidationState,
    /// True when the run produced its quality report (passed or quality_failed). Violations do
    /// not change the charge; an incomplete or failed run does.
    pub report_produced: bool,
}

fn coded_refusal(code: RefusalCode, message: impl std::fmt::Display) -> String {
    format!("{}: {message}", refusal_name(code))
}

fn refusal_name(code: RefusalCode) -> &'static str {
    match code {
        RefusalCode::AuthRequired => "AUTH_REQUIRED",
        RefusalCode::EmailUnverified => "EMAIL_UNVERIFIED",
        RefusalCode::AccountDisabled => "ACCOUNT_DISABLED",
        RefusalCode::CapabilityDenied => "CAPABILITY_DENIED",
        RefusalCode::VersionUnsupported => "VERSION_UNSUPPORTED",
        RefusalCode::ProtocolUnsupported => "PROTOCOL_UNSUPPORTED",
        RefusalCode::UnsafeToStart => "UNSAFE_TO_START",
        RefusalCode::CreditsExhausted => "CREDITS_EXHAUSTED",
        RefusalCode::CreditsHeld => "CREDITS_HELD",
        RefusalCode::OperationConflict => "OPERATION_CONFLICT",
        RefusalCode::OperationFinalized => "OPERATION_FINALIZED",
        RefusalCode::OperationOwnedElsewhere => "OPERATION_OWNED_ELSEWHERE",
        RefusalCode::AuthUnavailable => "AUTH_UNAVAILABLE",
    }
}

fn settle_check(billing: &mut dyn BillingPort, operation: &OperationId, produced: bool) -> Result<(), String> {
    if produced {
        match billing.commit(operation, BillingResult::Succeeded) {
            Ok(_) => Ok(()),
            Err(error) => Err(coded_refusal(
                error.code,
                format!("the report was produced but settlement was not confirmed; keep operation {} and settle it again", operation.as_str()),
            )),
        }
    } else {
        match billing.release(operation, RefusalCode::UnsafeToStart) {
            Ok(()) => Ok(()),
            Err(error) => Err(coded_refusal(
                error.code,
                format!("the check produced no report and its release was not confirmed; keep operation {} for recovery", operation.as_str()),
            )),
        }
    }
}

/// Run one mixed profile as exactly one controlled check.
///
/// The resolved profile must contain a DFM check. A verified safe-stop bound and a server-issued
/// admission are required before the profile is registered or a job is submitted. Only the exact
/// admitted run is claimed; the earlier of the measured bound and the execution deadline stops it.
/// A produced quality report (passed or quality_failed, violations or not) settles once; an
/// incomplete, cancelled, timed-out or failed run releases once. A replay of an already terminal
/// run releases this invocation's admission and does not charge again.
pub fn run_controlled_check(
    port: &ValidationPort,
    candidate: &CandidateRef,
    profile: CheckProfile,
    request_id: &str,
    owner: &str,
    operation: &OperationId,
    budget: &BudgetTable,
    billing: &mut dyn BillingPort,
) -> Result<ControlledCheck, String> {
    if profile_scope(&profile) != ProfileScope::Dfm {
        return Err("CAPABILITY_DENIED: the controlled path only accepts a profile that contains a DFM check".to_string());
    }
    let bound = match budget.bound(Capability::Dfm, EntryCategory::CliDfm) {
        UpperBound::Verified(bound) => bound,
        UpperBound::Unverified => {
            return Err(coded_refusal(RefusalCode::UnsafeToStart, "this controlled CLI DFM path has no verified safe-stop bound"));
        }
    };
    let grant = billing.admit(operation, Capability::Dfm, EntryCategory::CliDfm).map_err(|error| coded_refusal(error.code, error.message))?;
    drop(grant); // A grant is execution authority, not a value the CLI stores or logs.
    let now = now_ms();
    let stop_at_ms = now.saturating_add(duration_ms(bound));
    let reserved_run = generate_run_id(now);

    enum Claimed {
        Ran(String),
        Replayed(String, ValidationState),
    }

    let mut executed = false;
    let mut uncertain = false;
    let run = (|| -> Result<Claimed, String> {
        let reference = port.register_profile(profile).map_err(|error| format!("PROFILE_UNSUPPORTED: {error}"))?;
        let submitted = port
            .submit_controlled_for(candidate, &reference, request_id, owner, &reserved_run, operation.as_str(), stop_at_ms)
            .map_err(|error| error.to_string())?;
        if submitted.run_id != reserved_run {
            return Err(coded_refusal(RefusalCode::OperationConflict, "the controlled run identity does not match the reservation"));
        }
        if submitted.state.is_terminal() {
            // The durable run already exists. This invocation produced nothing and must not charge.
            return Ok(Claimed::Replayed(submitted.run_id, submitted.state));
        }
        let claimed = port.start_controlled_run(&submitted.run_id, operation.as_str()).map_err(|error| error.to_string())?;
        let Some(job) = claimed else {
            let state = port.status(&submitted.run_id).map_err(|error| error.to_string())?.state;
            return match state {
                ValidationState::Queued => {
                    Err(coded_refusal(RefusalCode::UnsafeToStart, "the admitted run could not be claimed by this host; it was not executed"))
                }
                ValidationState::Running => {
                    uncertain = true;
                    Err(coded_refusal(
                        RefusalCode::AuthUnavailable,
                        "another worker owns the admitted run; its outcome is not proven and its hold is kept",
                    ))
                }
                _ => Ok(Claimed::Replayed(submitted.run_id, state)),
            };
        };
        executed = true;
        port.run_claimed_with_stop(&job, Some(stop_at_ms)).map_err(|error| error.to_string())?;
        Ok(Claimed::Ran(submitted.run_id))
    })();

    match run {
        Ok(Claimed::Ran(run_id)) => {
            let status = port.status(&run_id).map_err(|error| {
                format!("the check ran but its status could not be read; operation {} stays for recovery: {error}", operation.as_str())
            })?;
            let produced = matches!(status.state, ValidationState::Passed | ValidationState::QualityFailed);
            settle_check(billing, operation, produced)?;
            Ok(ControlledCheck { run_id, state: status.state, report_produced: produced })
        }
        Ok(Claimed::Replayed(run_id, state)) => {
            settle_check(billing, operation, false)?;
            Ok(ControlledCheck { run_id, state, report_produced: false })
        }
        Err(error) if executed || uncertain => Err(coded_refusal(
            RefusalCode::AuthUnavailable,
            format!(
                "the controlled check may have started but its result is not proven; keep operation {} and settle it, never rerun it: {error}",
                operation.as_str()
            ),
        )),
        Err(error) => {
            settle_check(billing, operation, false)?;
            Err(error)
        }
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// A worker wrapper that sleeps before running checks. Tests use it to hold a submit open while
/// a second process calls status or cancel. It changes no production behaviour unless
/// `SOLIDCRAFT_VALIDATION_RUNNER_DELAY_MS` is set.
struct SlowRunner {
    delay: Duration,
}

impl Runner for SlowRunner {
    fn identity(&self) -> RunnerIdentity {
        LocalRunner::identity()
    }

    fn run(&self, job: &RunJob, control: &RunControl) -> RunnerOutcome {
        // This is a capped test hook: it must never outlive the execution deadline or delay a
        // cancellation, even if the environment asks for a huge delay.
        let delay = self.delay.min(Duration::from_secs(5));
        let deadline = std::time::Instant::now() + delay;
        while std::time::Instant::now() < deadline {
            if control.cancelled() || control.timed_out() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        LocalRunner.run(job, control)
    }
}

fn open_worker(store: PathBuf, candidates: PathBuf) -> Result<ValidationPort, String> {
    // Test-only hook, capped by SlowRunner and bounded by the execution deadline.
    let delay = std::env::var("SOLIDCRAFT_VALIDATION_RUNNER_DELAY_MS").ok().and_then(|value| value.parse::<u64>().ok()).unwrap_or(0).min(5_000);
    if delay > 0 {
        ValidationPort::open_with(
            store,
            candidates,
            Arc::new(SlowRunner { delay: Duration::from_millis(delay) }),
            Limits::default(),
            Arc::new(SystemClock),
        )
        .map_err(|error| error.to_string())
    } else {
        ValidationPort::open(store, candidates).map_err(|error| error.to_string())
    }
}

/// Run one `validate` mode.
pub fn run(args: &[String]) -> Result<(), String> {
    let positionals = positional(args);
    let mode = positionals.first().map(|arg| arg.as_str()).ok_or_else(|| format!("usage:\n{USAGE}"))?;
    let store = validation_dir(args);
    let candidates = candidate_dir(args);
    // Only the worker host reconciles. Status/report/cancel are read-only observers: opening
    // them must not turn another process's live job into a restart.
    let port = if mode == "submit" || mode == "skill" {
        open_worker(store, candidates)
    } else {
        ValidationPort::open_read_only(store, candidates).map_err(|error| error.to_string())
    }?;
    match mode {
        "submit" => {
            let request_path = positionals.get(1).ok_or_else(|| "validate submit needs <request.json>".to_string())?;
            let request = read_request(request_path)?;
            let profile = resolve_profile(args, &request.profile)?;
            let reference = profile_ref(&profile).map_err(|error| error.to_string())?;
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
            if profile_scope(&profile) == ProfileScope::Dfm {
                // One mixed request is one controlled check. Without a verified bound and an
                // installed authority it is refused whole, never run with the DFM part skipped.
                let operation = mint_operation()
                    .ok_or_else(|| coded_refusal(RefusalCode::AuthUnavailable, "the host could not mint an operation for this controlled check"))?;
                let budget = BudgetTable::unverified_default();
                let mut billing = NoAuthorityBilling;
                let controlled =
                    run_controlled_check(&port, &request.candidate, profile, &request.request_id, owner, &operation, &budget, &mut billing)?;
                let status = port.status(&controlled.run_id).map_err(|error| error.to_string())?;
                return print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?, output(args));
            }
            let reference = port.register_profile(profile).map_err(|error| error.to_string())?;
            let submitted = port.submit_for(&request.candidate, &reference, &request.request_id, owner).map_err(|error| error.to_string())?;
            port.pump_free().map_err(|error| error.to_string())?;
            let status = port.status(&submitted.run_id).map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(status).map_err(|error| error.to_string())?, output(args))
        }
        "skill" => {
            let request_path = positionals.get(1).ok_or_else(|| "validate skill needs <request.json>".to_string())?;
            let request = read_request(request_path)?;
            let live = request.live.ok_or_else(|| "validate skill needs live: {documentId, revision}".to_string())?;
            let profile = resolve_profile(args, &request.profile)?;
            let reference = text_to_cad_profile_ref(&profile).map_err(|error| error.to_string())?;
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
            if profile_scope(&profile) == ProfileScope::Dfm {
                // The skill path submits and pumps internally, so it cannot yet offer the exact
                // claimed run a controlled check must bill. Refuse the whole mixed request rather
                // than run the DFM part unbilled or skip it silently.
                return Err(coded_refusal(
                    RefusalCode::UnsafeToStart,
                    "a mixed skill profile cannot be billed on this path; run it as validate submit or wait for the controlled skill entry",
                ));
            }
            let report = validate_skill_snapshot(&port, SkillValidationRequest::new(&request.candidate, profile, request.request_id).with_live(live))
                .map_err(|error| error.to_string())?;
            print_json(&serde_json::to_value(report).map_err(|error| error.to_string())?, output(args))
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
