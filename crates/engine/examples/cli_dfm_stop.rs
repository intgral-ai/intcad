//! Measure the CLI DFM submit/claim/run path below billing, using the actual validation port.

mod controlled_stop_support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use controlled_stop_support::{CUTS, Result, header, plate, sample};
use serde_json::json;
use solidcraft_engine::schema::snapshot::CandidateRef;
use solidcraft_engine::validation::{
    CheckRequirement, Limits, LocalRunner, RunControl, RunJob, Runner, RunnerIdentity, RunnerOutcome, SystemClock, ValidationPort, ValidationState,
    generate_run_id, generate_submission_key, native_basic_profile, now_ms,
};

#[derive(Default)]
struct TimedRunner(Mutex<Duration>);

impl Runner for TimedRunner {
    fn identity(&self) -> RunnerIdentity {
        LocalRunner::identity()
    }

    fn run(&self, job: &RunJob, control: &RunControl) -> RunnerOutcome {
        let start = Instant::now();
        let outcome = LocalRunner.run(job, control);
        if let Ok(mut wall) = self.0.lock() {
            *wall = start.elapsed();
        }
        outcome
    }
}

fn main() -> Result<()> {
    let root = std::env::temp_dir().join(format!("intcad-cli-dfm-stop-{}-{}", std::process::id(), now_ms()));
    std::fs::create_dir(&root)?;
    let result = measure(&root);
    std::fs::remove_dir_all(&root)?;
    result
}

fn measure(root: &std::path::Path) -> Result<()> {
    let candidates = root.join("candidates");
    std::fs::create_dir(&candidates)?;
    solidcraft_engine::cmd::set_candidate_store(Some(candidates.clone()));

    header("cut features / phase");
    for cuts in CUTS {
        let mut session = plate(cuts)?;
        let captured = session.execute("document.snapshot.capture", &json!({}))?;
        let candidate: CandidateRef = serde_json::from_value(captured.get("candidate").cloned().ok_or("snapshot missing candidate")?)?;
        let state = session.model.state();
        let sheet = state.sheets.first().ok_or("fixture missing sheet")?;
        let mut profile = native_basic_profile();
        profile.profile_id = "dfm-stop-bench".into();
        profile.checks.insert(
            0,
            CheckRequirement {
                check_id: "dfm.sheet".into(),
                kind: "dfm.sheet".into(),
                required: false,
                parameters: json!({
                    "sheet":0, "body":sheet.body,
                    "declared":{"thicknessMm":2,"process":"press brake","material":"5052-H32"},
                    "generic":{"format":1,"name":"generic","version":"BENCH","layer":"generic","requirements":[]},
                    "overrides":[], "model":{"modelVersion":"doc-1","body":sheet.body,"source":"native"}, "source":"native",
                }),
            },
        );
        let runner = Arc::new(TimedRunner::default());
        let mut overhead = Vec::new();
        let mut pass = 0;
        sample(format!("{cuts} / total"), || {
            pass += 1;
            let store = root.join(format!("validation-{cuts}-{pass}"));
            let start = Instant::now();
            let port = ValidationPort::open_with(store, candidates.clone(), runner.clone(), Limits::default(), Arc::new(SystemClock))?;
            let reference = port.register_profile(profile.clone())?;
            let now = now_ms();
            let run = generate_run_id(now);
            let op = format!("op_bench_{cuts}_{pass}");
            // Harness watchdog only; the validation runner still has its normal ceiling.
            let stop = now.saturating_add(600_000);
            let submitted = port.submit_controlled_for(&candidate, &reference, &generate_submission_key(now), "local", &run, &op, stop)?;
            if submitted.run_id != run || submitted.state != ValidationState::Queued {
                return Err("validation did not queue the reserved run".into());
            }
            let job = port.start_controlled_run(&run, &op)?.ok_or("validation did not claim the run")?;
            port.run_claimed_with_stop(&job, Some(stop))?;
            let status = port.status(&run)?;
            if status.state != ValidationState::Passed {
                return Err(format!("validation did not pass: {:?}", status.state).into());
            }
            let _json_reply = serde_json::to_vec_pretty(&status)?;
            let total = start.elapsed();
            let inside = *runner.0.lock().map_err(|_| "runner timing lock poisoned")?;
            overhead.push(total.checked_sub(inside).ok_or("runner exceeded total time")?);
            Ok(total)
        })?;
        let mut overhead = overhead.into_iter();
        sample(format!("{cuts} / outside LocalRunner"), || overhead.next().ok_or_else(|| "missing paired sample".into()))?;
    }
    Ok(())
}
