//! Measure physical drain after cancellation of an offline local task at three in-flight points.

#[path = "../../engine/examples/controlled_stop_support/mod.rs"]
mod controlled_stop_support;

use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use controlled_stop_support::{CUTS, Result, header, plate, sample};
use solidcraft_agent_task as task;
use solidcraft_agent_task_engine::{LiveTask, l_bracket_profile};
use solidcraft_engine::Session;
use solidcraft_ui_egui::agent_chat::{agents, host_ai::*};

// A control callback that has already started can keep its reply sender alive after cancel.
const FAKE_WAIT: Duration = Duration::from_millis(100);
const READY_LIMIT: Duration = Duration::from_secs(5);

type Worker = thread::JoinHandle<std::result::Result<(), String>>;

fn model_wait(entered: mpsc::Sender<()>, drained: mpsc::Sender<()>, turn: &TurnControl) -> Result<Worker> {
    let shared = Arc::new(AiShared::default());
    shared
        .connections()
        .replace(&[agents::AgentConfig {
            id: "offline".into(),
            kind: agents::Kind::Api,
            base_url: "http://unused.invalid/v1".into(),
            request_timeout_s: 90,
            ..Default::default()
        }])
        .map_err(|error| error.message())?;
    let binding = shared.connections().bind("offline", "local").map_err(|error| error.message())?;
    let epoch = Instant::now();
    let host = HostAi::new(HostAiDependencies {
        shared,
        http: Arc::new(move |_| {
            // The last clone of this closure is owned by the physical HTTP worker.
            let _lifetime = &drained;
            entered.send(()).map_err(|error| error.to_string())?;
            thread::sleep(FAKE_WAIT);
            Ok((200, br#"{"choices":[{"message":{"role":"assistant","content":"late answer"},"finish_reason":"stop"}]}"#.to_vec()))
        }),
        secrets: None,
        artifacts: Arc::new(|_, _| Ok(Arc::from([]))),
        clock: Arc::new(move || epoch.elapsed()),
        ids: Arc::new(|| Ok("0123456789abcdef0123456789abcdef".into())),
    });
    let session = SessionKey { runtime_instance_id: "bench".into(), session_id: "local".into() };
    host.open_session(session.clone(), AiLimits::contract_defaults()).map_err(|error| error.message())?;
    let context = CallContext {
        scope: CallScope { session, turn_id: "turn".into(), workflow_run_id: "workflow".into() },
        control: turn.new_call().map_err(|error| error.message())?,
        deadline: Duration::from_secs(10),
        limits: AiLimits::contract_defaults(),
        model_budget: Arc::new(ModelCallBudget::new(4).map_err(|error| error.message())?),
    };
    let request = GenerateRequest {
        connection_id: "offline".into(),
        model_id: "local".into(),
        messages: vec![ModelMessage {
            role: MessageRole::User,
            content: vec![ContentPart::Text("local bench".into())],
            tool_calls: vec![],
            tool_call_id: None,
        }],
        tools: vec![],
        options: GenerateOptions {
            delivery: Delivery::Buffered,
            timeout_seconds: 10,
            max_output_tokens: None,
            temperature: None,
            reasoning_effort: None,
            output_schema: None,
        },
        continuation: None,
    };
    Ok(thread::spawn(move || match host.generate(&binding, &context, request) {
        Err(error) if error.code() == AiErrorCode::Cancelled => Ok(()),
        other => Err(format!("expected cancellation, got {other:?}")),
    }))
}

fn cancelled_sample(
    session: &mut Session,
    root: &std::path::Path,
    work: impl FnOnce(mpsc::Sender<()>, mpsc::Sender<()>, &TurnControl) -> Result<Worker>,
    after_entered: Duration,
) -> Result<(Duration, Duration)> {
    let document = task::DocumentRef { project_id: "local".into(), document_id: session.document_id.clone(), version: session.design_revision() };
    let mut live = LiveTask::start(
        session,
        root.join("candidates"),
        root.join("validation"),
        l_bracket_profile(5.0)?,
        task::TaskSpec {
            skill_id: task::TEXT_TO_CAD_ID.into(),
            input: "local bench".into(),
            document,
            selection: Vec::new(),
            requirements: None,
            budget: task::Budget::finite(4, 16, 1, 120_000),
        },
        task::PermissionContext { principal: "agent-chat".into(), approval_mode: task::ApprovalMode::Ask, allow_list: Vec::new(), turn_valid: true },
    )?;
    live.on_turn_start()?;
    let turn = TurnControl::new();
    let (entered, rx) = mpsc::channel();
    let (drained, drain_rx) = mpsc::channel();
    let worker = work(entered, drained, &turn)?;
    rx.recv_timeout(READY_LIMIT)?;
    thread::sleep(after_entered);
    if worker.is_finished() {
        return Err("the worker already stopped before the cancellation point".into());
    }
    let start = Instant::now();
    // The same LiveTask call made by task_runtime::cancel, followed by host model cancellation.
    live.on_cancel("measurement cancellation");
    turn.cancel();
    worker.join().map_err(|_| "in-flight worker did not join cleanly")??;
    let joined = start.elapsed();
    if !matches!(drain_rx.recv_timeout(READY_LIMIT), Err(mpsc::RecvTimeoutError::Disconnected)) {
        return Err("the worker still holds the physical callback".into());
    }
    if !matches!(live.on_turn_end(session), task::TurnAction::Terminal(result) if result.terminal == task::TaskTerminal::Cancelled) {
        return Err("the task did not report Cancelled".into());
    }
    Ok((start.elapsed(), joined))
}

fn measure(root: &std::path::Path) -> Result<()> {
    std::fs::create_dir(root.join("candidates"))?;
    solidcraft_engine::cmd::set_candidate_store(Some(root.join("candidates")));
    header("cancel point");
    let mut session = Session::default();
    let mut wait_only = Vec::new();
    sample("HostAi buffered model request (100 ms fake HTTP) / drained", || {
        let (drain, joined) = cancelled_sample(&mut session, root, model_wait, Duration::ZERO)?;
        wait_only.push(joined);
        Ok(drain)
    })?;
    let mut wait_only = wait_only.into_iter();
    sample("HostAi buffered model request / waiter returned", || wait_only.next().ok_or_else(|| "missing paired sample".into()))?;
    sample("in-flight tool callback (100 ms local fake)", || {
        cancelled_sample(
            &mut session,
            root,
            |entered, _drained, _| {
                Ok(thread::spawn(move || {
                    entered.send(()).map_err(|error| error.to_string())?;
                    thread::sleep(FAKE_WAIT);
                    Ok(())
                }))
            },
            Duration::ZERO,
        )
        .map(|(drain, _)| drain)
    })?;
    // The 8-cut prefix is enough to prove the worker is still busy after cancelling. The
    // separate 32-cut native rebuild measurements characterize the slower shared term.
    let fixture = plate(CUTS[1])?;
    let design = fixture.design().into_owned();
    sample("synchronous native rebuild (8 cuts; cancel after 20 ms)", || {
        let design = design.clone();
        cancelled_sample(
            &mut session,
            root,
            |entered, _drained, _| {
                Ok(thread::spawn(move || {
                    entered.send(()).map_err(|error| error.to_string())?;
                    let rebuilt = Session::opened_at(design, None, None);
                    let state = rebuilt.world_state().map_err(|error| error.to_string())?;
                    if state.sheets.len() != 1 {
                        return Err("rebuild did not return the expected sheet".into());
                    }
                    Ok(())
                }))
            },
            Duration::from_millis(20),
        )
        .map(|(drain, _)| drain)
    })?;
    Ok(())
}

fn main() -> Result<()> {
    let root = std::env::temp_dir().join(format!("intcad-mastra-cancel-{}-{}", std::process::id(), solidcraft_engine::validation::now_ms()));
    std::fs::create_dir(&root)?;
    let result = measure(&root);
    std::fs::remove_dir_all(&root)?;
    result
}
