//! Measure MCP envelope and real Headless execution, separately from paid admission.

#[path = "../../engine/examples/controlled_stop_support/mod.rs"]
mod controlled_stop_support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use controlled_stop_support::{CUTS, Result, elapsed, header, plate, sample};
use serde_json::{Value, json};
use solidcraft_engine::entitlement::{
    AccountId, AuthenticatedHostScope, Capability, EntryCategory, HostSessionBinding, InstallationId, TrustedContext,
};
use solidcraft_mcp::{Backend, Headless, Server, ToolCallContext};

struct TimedHeadless {
    inner: Headless,
    wall: Arc<Mutex<Duration>>,
    context: ToolCallContext,
}

impl Backend for TimedHeadless {
    fn call(&mut self, method: &str, args: Value) -> std::result::Result<Value, String> {
        let start = Instant::now();
        let result = self.inner.call(method, args);
        *self.wall.lock().map_err(|_| "backend timing lock poisoned")? = start.elapsed();
        result
    }

    fn has_ui(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        self.inner.describe()
    }

    fn tool_context(&self, _tool: &str) -> Option<ToolCallContext> {
        Some(self.context.clone())
    }
}

fn main() -> Result<()> {
    header("cut features / phase");
    let scope = AuthenticatedHostScope::new(
        AccountId::new("bench-account"),
        InstallationId::new("bench-installation"),
        HostSessionBinding::new("bench-session"),
    );
    let trusted = TrustedContext::mint(&scope, "op_bench_headless", Capability::Mcp, EntryCategory::HeadlessMcp)
        .map_err(|error| format!("bench context: {error:?}"))?;
    let context = ToolCallContext::from_trusted(trusted);
    let initialize = json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":solidcraft_mcp::PROTOCOL_VERSION}});
    let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"execute","arguments":{
        "command":"inspect.measure","params":{"names_only":true}
    }}})
    .to_string();
    let wire = format!("{line}\n");
    for cuts in CUTS {
        let wall = Arc::new(Mutex::new(Duration::ZERO));
        let backend = TimedHeadless { inner: Headless::new(plate(cuts)?), wall: wall.clone(), context: context.clone() };
        // This harness-only server mode measures real tools/call → Headless without claiming a
        // paid entry. The default (closed) server is not allowed to run the charged execute tool.
        let mut server = Server::with_forwarded_authority(Box::new(backend));
        if server.handle(initialize.clone()).and_then(|reply| reply.pointer("/result/serverInfo").cloned()).is_none() {
            return Err("server did not initialize".into());
        }
        let _ = server.handle(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        let mut overhead = Vec::new();
        sample(format!("{cuts} / total"), || {
            let total = elapsed(|| {
                let mut bytes = Vec::new();
                server.serve(wire.as_bytes(), &mut bytes)?;
                let reply: Value = serde_json::from_slice(&bytes)?;
                let body: Value = serde_json::from_str(reply.pointer("/result/content/0/text").and_then(Value::as_str).ok_or("no MCP reply")?)?;
                if reply["result"]["isError"] == true || body["body_count"] != 1 {
                    return Err(format!("unexpected MCP response: {reply}").into());
                }
                Ok(())
            })?;
            let inside = *wall.lock().map_err(|_| "backend timing lock poisoned")?;
            overhead.push(total.checked_sub(inside).ok_or("backend exceeded total time")?);
            Ok(total)
        })?;
        let mut overhead = overhead.into_iter();
        sample(format!("{cuts} / outside Headless"), || overhead.next().ok_or_else(|| "missing paired sample".into()))?;
    }
    Ok(())
}
