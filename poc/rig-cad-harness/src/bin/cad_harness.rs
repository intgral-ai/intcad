//! Headless invocation of the same CAD Task Controller.
//!
//! This binary has no UI. It builds the real engine host by default and runs
//! the L-bracket task through Rig with the deterministic fake provider
//! (`MockCompletionModel`, explicitly labelled). `--host mock` selects the
//! genuinely mock state-machine host for fault injection only.

use std::sync::Arc;
use std::time::Duration;

use cad_rig_harness::controller::TaskController;
use cad_rig_harness::engine_host::EngineCadHost;
use cad_rig_harness::mock_host::{MockHost, PermissionMode, ValidationMode, WriteMode};
use cad_rig_harness::rig_runner::{build_agent, run_plain, run_task};
use cad_rig_harness::scenario;
use cad_rig_harness::types::{Budget, CadHost, TaskSpec, TaskTerminal};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostKind {
    Engine,
    Mock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Happy,
    EarlyStop,
    MissingValidation,
    Repair,
    QualityFailed,
    WriteWithoutInspect,
    Stale,
    Unknown,
    Partial,
    Denied,
    Cancel,
    Timeout,
    PlainChat,
}

impl Case {
    fn name(self) -> &'static str {
        match self {
            Self::Happy => "happy",
            Self::EarlyStop => "early_stop",
            Self::MissingValidation => "missing_validation",
            Self::Repair => "repair",
            Self::QualityFailed => "quality_failed",
            Self::WriteWithoutInspect => "write_without_inspect",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
            Self::Partial => "partial",
            Self::Denied => "denied",
            Self::Cancel => "cancel",
            Self::Timeout => "timeout",
            Self::PlainChat => "plain_chat",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "happy" => Self::Happy,
            "early_stop" => Self::EarlyStop,
            "missing_validation" => Self::MissingValidation,
            "repair" => Self::Repair,
            "quality_failed" => Self::QualityFailed,
            "write_without_inspect" => Self::WriteWithoutInspect,
            "stale" => Self::Stale,
            "unknown" => Self::Unknown,
            "partial" => Self::Partial,
            "denied" => Self::Denied,
            "cancel" => Self::Cancel,
            "timeout" => Self::Timeout,
            "plain_chat" => Self::PlainChat,
            _ => return None,
        })
    }

    fn requires_mock(self) -> bool {
        matches!(self, Self::Stale | Self::Unknown | Self::Partial | Self::Denied | Self::Cancel | Self::Timeout)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut case = Case::Happy;
    let mut host = HostKind::Engine;
    let mut root = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scenario" => {
                let value = args.next().unwrap_or_default();
                if value == "all" {
                    run_all().await;
                    return;
                }
                match Case::parse(&value) {
                    Some(parsed) => case = parsed,
                    None => {
                        eprintln!("unknown scenario `{value}`");
                        std::process::exit(2);
                    }
                }
            }
            "--host" => {
                let value = args.next().unwrap_or_default();
                host = match value.as_str() {
                    "engine" => HostKind::Engine,
                    "mock" => HostKind::Mock,
                    _ => {
                        eprintln!("--host wants engine or mock");
                        std::process::exit(2);
                    }
                };
            }
            "--root" => root = args.next(),
            "--help" | "-h" => {
                println!(
                    "usage: cad-harness [--scenario happy|early_stop|missing_validation|repair|quality_failed|write_without_inspect|stale|unknown|partial|denied|cancel|timeout|plain_chat|all] [--host engine|mock] [--root DIR]"
                );
                return;
            }
            other => {
                eprintln!("unknown argument `{other}`");
                std::process::exit(2);
            }
        }
    }
    if case.requires_mock() && host == HostKind::Engine {
        eprintln!("scenario `{}` uses fault injection and needs --host mock", case.name());
        std::process::exit(2);
    }
    match run_case(case, host, root.as_deref()).await {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap_or_default()),
        Err(error) => {
            eprintln!("cad-harness: {error}");
            std::process::exit(1);
        }
    }
}

async fn run_all() {
    let cases = [
        (Case::Happy, HostKind::Engine),
        (Case::EarlyStop, HostKind::Engine),
        (Case::MissingValidation, HostKind::Engine),
        (Case::Repair, HostKind::Engine),
        (Case::QualityFailed, HostKind::Engine),
        (Case::WriteWithoutInspect, HostKind::Engine),
        (Case::PlainChat, HostKind::Engine),
        (Case::Stale, HostKind::Mock),
        (Case::Unknown, HostKind::Mock),
        (Case::Partial, HostKind::Mock),
        (Case::Denied, HostKind::Mock),
        (Case::Cancel, HostKind::Mock),
        (Case::Timeout, HostKind::Mock),
    ];
    let mut results = Vec::new();
    for (case, host) in cases {
        match run_case(case, host, None).await {
            Ok(value) => results.push(value),
            Err(error) => results.push(json!({"scenario": case.name(), "error": error})),
        }
    }
    println!("{}", serde_json::to_string_pretty(&json!({"spike": "track-b-b", "results": results})).unwrap_or_default());
}

async fn run_case(case: Case, host_kind: HostKind, root: Option<&str>) -> Result<Value, String> {
    let root = root.map(str::to_string).unwrap_or_else(|| {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        std::env::temp_dir().join(format!("cad-rig-harness-{}-{nanos}", std::process::id())).to_string_lossy().into_owned()
    });
    let (host, mock): (Arc<dyn CadHost>, Option<Arc<MockHost>>) = match host_kind {
        HostKind::Engine => (EngineCadHost::l_bracket(&root)?, None),
        HostKind::Mock => {
            let host = MockHost::l_bracket();
            (host.clone(), Some(host))
        }
    };

    if let Some(mock) = &mock {
        match case {
            Case::Stale => mock.set_write_mode(WriteMode::StaleOnce),
            Case::Unknown => mock.set_write_mode(WriteMode::UnknownOnce),
            Case::Partial => mock.set_write_mode(WriteMode::PartialOnce),
            Case::Denied => mock.set_permission_mode(PermissionMode::DenyAll),
            Case::Cancel => mock.set_write_mode(WriteMode::CancelDuringWrite),
            Case::Repair => mock.set_validation_mode(ValidationMode::FailOnce),
            Case::QualityFailed => mock.set_validation_mode(ValidationMode::FailAlways),
            _ => {}
        }
    }

    let mut spec = TaskSpec::l_bracket();
    if case == Case::QualityFailed {
        spec.budget = Budget { repair_rounds: 0, ..spec.budget };
    }
    if case == Case::Timeout {
        spec.budget = Budget { wall_time_ms: 0, ..spec.budget };
    }
    let controller = TaskController::start(spec, Arc::clone(&host)).map_err(|error| error.to_string())?;

    if case == Case::Cancel {
        let controller = controller.clone();
        let mock = mock.clone();
        std::thread::spawn(move || {
            if let Some(mock) = mock {
                let _ = mock.wait_until_write_started(Duration::from_secs(5));
            }
            controller.cancel("user pressed Stop");
        });
    }

    let model = match case {
        Case::Happy | Case::Cancel => scenario::happy_model(),
        Case::Stale => scenario::stale_model(),
        Case::Unknown => scenario::unknown_model(),
        Case::Partial => scenario::partial_model(),
        Case::Denied => scenario::denied_model(),
        Case::Timeout => scenario::happy_model(),
        Case::EarlyStop => scenario::early_stop_model(),
        Case::MissingValidation => scenario::missing_validation_model(),
        Case::Repair => scenario::repair_model(),
        Case::QualityFailed => scenario::quality_failed_model(),
        Case::WriteWithoutInspect => scenario::write_without_inspect_model(),
        Case::PlainChat => scenario::plain_chat_model(),
    };
    let agent = build_agent(model, &controller);

    let (result, plain) = if case == Case::PlainChat {
        let text = run_plain(&agent, "Hello, just chat.").await?;
        (controller.result(), Some(text))
    } else {
        (run_task(&agent, &controller, scenario::PROMPT, 24).await, None)
    };

    let engine_evidence = if host_kind == HostKind::Engine && case != Case::PlainChat {
        match host.inspect() {
            Ok(view) => Some(view),
            Err(error) => {
                return Ok(json!({
                    "scenario": case.name(),
                    "host": "real-engine",
                    "provider": "rig 0.46.0 MockCompletionModel (fake provider, labelled)",
                    "error": error.to_string(),
                    "task": result
                }))
            }
        }
    } else {
        None
    };

    Ok(json!({
        "scenario": case.name(),
        "host": if host_kind == HostKind::Engine { "real-engine" } else { "mock (state machine/fault injection only)" },
        "provider": "rig 0.46.0 MockCompletionModel (fake provider, labelled; no network)",
        "validated_success": result.terminal == TaskTerminal::ValidatedSuccess,
        "plain_chat": plain,
        "plain_chat_isolated": case == Case::PlainChat,
        "engine_evidence": engine_evidence,
        "task": if case == Case::PlainChat { Value::Null } else { serde_json::to_value(&result).unwrap_or(Value::Null) }
    }))
}
