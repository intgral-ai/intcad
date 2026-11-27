//! Opt-in measurement of the real Unix bridge lane against a windowless control recipient.

#![cfg(unix)]

#[path = "../../engine/examples/controlled_stop_support/mod.rs"]
mod controlled_stop_support;

// Compile the private lane unchanged. This stand-in for the private request constructors
// keeps the real lane/forwarding/queue/reply code runnable without billing or opening a GUI.
#[allow(dead_code)]
#[path = "../src/agent_chat/host_bridge.rs"]
mod host_bridge;

mod backend {
    pub use solidcraft_ui_egui::agent_chat::backend::{Event, ToolStatus};
}

mod control {
    use serde_json::Value;
    use solidcraft_engine::entitlement::{ConservativeDeadline, MonotonicInstant, TrustedContext};
    pub use solidcraft_ui_egui::control::entry_capability;
    use std::sync::{
        Arc,
        atomic::AtomicBool,
        mpsc::{Receiver, Sender, channel},
    };

    pub struct ControlRequest {
        pub method: String,
        pub params: Value,
        pub reply: Sender<Value>,
        pub chat_turn: Option<Arc<AtomicBool>>,
    }

    impl ControlRequest {
        pub fn trusted_bridge(method: &str, params: Value) -> (Self, Receiver<Value>) {
            let (reply, rx) = channel();
            (Self { method: method.into(), params, reply, chat_turn: None }, rx)
        }

        pub fn trusted_bridge_helper(method: &str, params: Value, _helper: &str) -> (Self, Receiver<Value>) {
            Self::trusted_bridge(method, params)
        }

        pub fn trusted_bridge_forwarded(
            method: &str,
            params: Value,
            _context: TrustedContext,
            _deadline: Option<ConservativeDeadline>,
            _now: Option<MonotonicInstant>,
        ) -> (Self, Receiver<Value>) {
            Self::trusted_bridge(method, params)
        }
    }
}

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use controlled_stop_support::{CUTS, Result, elapsed, header, plate, sample};
use host_bridge::{HostBridge, HostProvenance};
use serde_json::{Value, json};
use solidcraft_engine::entitlement::{AccountId, AuthenticatedHostScope, HostSessionBinding, InstallationId};
use solidcraft_ui_egui::{Services, SolveApp};

fn read_reply(reader: &mut BufReader<UnixStream>) -> Result<Value> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(serde_json::from_str(&line)?)
}

#[test]
#[ignore = "measurement harness; not part of CI"]
fn gui_bridge_mcp_stop() -> Result<()> {
    header("cut features / phase");
    let scope =
        AuthenticatedHostScope::new(AccountId::new("bench-account"), InstallationId::new("bench-install"), HostSessionBinding::new("bench-session"));
    let wire = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{
        "command":"inspect.measure","params":{"names_only":true}
    }}});
    for cuts in CUTS {
        let (tools, rx) = channel();
        let mut bridge = HostBridge::open(tools, Some(HostProvenance::new(scope.clone())))?;
        let child = bridge.take_child().ok_or("no bridge peer")?;
        child.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut writer = child.try_clone()?;
        let mut reader = BufReader::new(child);
        bridge.begin_turn(Arc::new(AtomicBool::new(true)));
        writeln!(writer, "{}", json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":solidcraft_mcp::PROTOCOL_VERSION}}))?;
        if read_reply(&mut reader)?.pointer("/result/serverInfo").is_none() {
            return Err("bridge initialization failed".into());
        }
        writeln!(writer, "{}", json!({"jsonrpc":"2.0","method":"notifications/initialized"}))?;
        let mut app = SolveApp::new(plate(cuts)?, Services::default());
        let ctx = egui::Context::default();
        let mut overhead = Vec::new();
        sample(format!("{cuts} / total"), || {
            let start = Instant::now();
            writeln!(writer, "{wire}")?;
            let envelope = rx.recv_timeout(Duration::from_secs(5))?;
            let inside = elapsed(|| {
                let (mut req, _) = solidcraft_ui_egui::ControlRequest::trusted_gui(envelope.method, envelope.params);
                let solidcraft_ui_egui::control::Outcome::Done(reply) = solidcraft_ui_egui::control::handle(&mut app, &ctx, &mut req) else {
                    return Err("expected synchronous control dispatch".into());
                };
                if reply["ok"] != true || reply["result"]["body_count"] != 1 {
                    return Err(format!("unexpected control response: {reply}").into());
                }
                envelope.reply.send(reply)?;
                Ok(())
            })?;
            let reply = read_reply(&mut reader)?;
            if reply.pointer("/result/content/0/text").is_none() || reply["result"]["isError"] == true {
                return Err(format!("unexpected MCP response: {reply}").into());
            }
            let total = start.elapsed();
            overhead.push(total.checked_sub(inside).ok_or("dispatch exceeded total time")?);
            Ok(total)
        })?;
        let mut overhead = overhead.into_iter();
        sample(format!("{cuts} / outside control dispatch"), || overhead.next().ok_or_else(|| "missing paired sample".into()))?;
    }
    Ok(())
}
