//! A scripted ACP agent for trying next-step suggestions (INT-1458) without an account: no model
//! runs, so its guesses and its work follow fixed rules, like a designer with set habits.
//!
//! ```text
//! cargo build -p solidcraft-ui-egui --example acp_next_step_agent
//! # Settings → AI → Add agent → custom command: target/debug/examples/acp_next_step_agent
//! # then turn on "Suggest the next step after a pause"
//! ```
//!
//! - Asked for a next-step guess (the message starts `[Intcad next-step suggestion.`), it reads
//!   the History and the usual values in the message and answers one step as JSON: round the
//!   newest body's vertical edges, then shell it from the top if it is tall enough, then drill a
//!   hole in its centre; sizes from the design's usual values where it has them.
//! - Sent one of those steps as a message (Tab), it does it through the Intcad tools the chat
//!   gives it: it measures the body for its size and runs the command.
//!
//! It reports usage for each answer so the time-and-cost log has numbers to show. These are NOT
//! measured: tokens are the text's characters ÷ 4, and the cost is those tokens at
//! [`PRICE_IN`] and [`PRICE_OUT`] US dollars per million.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// Assumed prices for the usage it reports (US dollars per million tokens, in and out).
const PRICE_IN: f64 = 3.0;
const PRICE_OUT: f64 = 15.0;

fn send(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

fn update(session: &str, u: Value) {
    send(&json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": session, "update": u}}));
}

fn say(session: &str, text: &str) {
    update(session, json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}));
}

fn tool(session: &str, id: &str, title: &str, status: &str, output: Option<&str>) {
    let mut u = json!({"sessionUpdate": if status == "pending" { "tool_call" } else { "tool_call_update" }, "toolCallId": id, "title": title, "kind": "edit", "status": status});
    if let Some(o) = output {
        u["content"] = json!([{"type": "content", "content": {"type": "text", "text": o}}]);
    }
    update(session, u);
}

/// Tokens as characters ÷ 4 (an estimate, not a count).
fn tokens(text: &str) -> u64 {
    (text.chars().count() as u64).div_ceil(4)
}

/// The usage of one answer: a `usage_update` with the session's cost, and the reply's `usage`.
fn usage(session: &str, cost: &mut f64, prompt: &str, answer: &str) -> Value {
    let (i, o) = (tokens(prompt), tokens(answer));
    *cost += (i as f64 * PRICE_IN + o as f64 * PRICE_OUT) / 1e6;
    update(session, json!({"sessionUpdate": "usage_update", "used": i + o, "size": 200_000, "cost": {"amount": *cost, "currency": "USD"}}));
    json!({"inputTokens": i, "outputTokens": o, "totalTokens": i + o})
}

/// The MCP server the chat handed over, as a child speaking JSON-RPC lines.
struct Mcp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Mcp {
    fn start(server: &Value) -> Option<Mcp> {
        let program = server.get("command")?.as_str()?;
        let args: Vec<String> = server.get("args")?.as_array()?.iter().filter_map(|a| a.as_str().map(str::to_string)).collect();
        let mut child = Command::new(program).args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        let mut m = Mcp { child, stdin, stdout, next: 0 };
        m.call(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "acp-next-step-agent", "version": "1"}}),
        )?;
        let _ = writeln!(m.stdin, "{}", json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        Some(m)
    }

    fn call(&mut self, method: &str, params: Value) -> Option<Value> {
        self.next += 1;
        let id = self.next;
        writeln!(self.stdin, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).ok()?;
        self.stdin.flush().ok()?;
        loop {
            let mut line = String::new();
            if self.stdout.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let v: Value = serde_json::from_str(line.trim()).ok()?;
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                return Some(v);
            }
        }
    }

    /// A tool's text and whether it failed.
    fn tool(&mut self, name: &str, args: Value) -> (String, bool) {
        match self.call("tools/call", json!({"name": name, "arguments": args})) {
            Some(r) => {
                let text = r.pointer("/result/content/0/text").and_then(Value::as_str).unwrap_or("").to_string();
                let failed = r.pointer("/result/isError").and_then(Value::as_bool).unwrap_or(false) || r.get("error").is_some();
                (text, failed)
            }
            None => ("the MCP server did not answer".into(), true),
        }
    }

    /// A body's box: (min, max).
    fn bbox(&mut self, body: &str) -> Option<([f64; 3], [f64; 3])> {
        let (text, failed) = self.tool("measure", json!({"body": body}));
        if failed {
            return None;
        }
        let v: Value = serde_json::from_str(&text).ok()?;
        let bodies = v.get("bodies").and_then(Value::as_array)?;
        let b = bodies.iter().find(|b| b.get("name").and_then(Value::as_str) == Some(body)).or_else(|| bodies.first())?;
        let corner = |k: &str| -> Option<[f64; 3]> {
            let a = b.pointer(&format!("/bbox/{k}"))?.as_array()?;
            Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
        };
        Some((corner("min")?, corner("max")?))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The first number after `key` in `text` ("height 25" → 25).
fn number_after(text: &str, key: &str) -> Option<f64> {
    let rest = &text[text.find(key)? + key.len()..];
    let num: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    num.parse().ok()
}

/// A size for a millimetre value in words: `3`, `2.5`.
fn mm(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The guess: one step from the History and the usual values in the message.
fn guess(prompt: &str) -> (String, String) {
    let history: Vec<&str> =
        prompt.lines().filter(|l| l.split_once(". ").is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty())).collect();
    let usual = prompt.lines().find_map(|l| l.strip_prefix("Usual values in this design: ")).unwrap_or("");
    let usual_of = |name: &str| number_after(usual, name);
    // Steps the user turned down: the habit after them is offered instead.
    let dismissed: Vec<&str> =
        prompt.lines().skip_while(|l| !l.starts_with("Suggestions the user dismissed")).skip(1).map_while(|l| l.strip_prefix("- ")).collect();
    let turned_down = |step: &str| dismissed.contains(&step);
    // The newest body made from scratch, and what was done after it.
    let Some(at) = history
        .iter()
        .rposition(|l| (l.contains("(BoxFeature") || l.contains("(ExtrudeFeature") || l.contains("(CylinderFeature")) && l.contains("-> bodies: "))
    else {
        return ("Make a 60 x 40 x 25 mm box named Case".into(), "The design is empty; a box is the usual start of an enclosure.".into());
    };
    let line = history[at];
    let body = line.split("-> bodies: ").nth(1).and_then(|b| b.split(',').next()).unwrap_or("Body1").trim().to_string();
    let height = number_after(line, "height ").unwrap_or(0.0);
    let after = &history[at + 1..];
    let done = |kind: &str| after.iter().any(|l| l.contains(kind));
    if !done("(FilletFeature") {
        let (r, why) = match usual_of("fillet radius: ") {
            Some(r) => (r, format!("{} mm is the radius this design already uses.", mm(r))),
            None => (3.0, format!("{body} was just made and its corners are still sharp.")),
        };
        let step = format!("Fillet the 4 vertical edges of {body} with {} mm", mm(r));
        if !turned_down(&step) {
            return (step, why);
        }
    }
    if height >= 15.0 && !done("(ShellFeature") {
        let t = usual_of("shell (wall) thickness: ").unwrap_or(2.0);
        let step = format!("Shell {body} from its top face with {} mm walls", mm(t));
        if !turned_down(&step) {
            return (step, format!("{body} is rounded and {} mm tall: it reads as a case to hollow out.", mm(height)));
        }
    }
    if !done("(HoleFeature") {
        let d = usual_of("hole diameter: ").unwrap_or(5.0);
        let why = if usual_of("hole diameter: ").is_some() {
            format!("{} mm is the hole size this design uses.", mm(d))
        } else {
            "A fixing hole usually follows once the shape is done.".into()
        };
        let step = format!("Drill a {} mm hole through the centre of {body}", mm(d));
        if !turned_down(&step) {
            return (step, why);
        }
    }
    (String::new(), format!("{body} already has its rounds and its hole; nothing obvious is left."))
}

/// Do one of the steps [`guess`] gives; what was done, or why not.
fn work(session: &str, m: &mut Mcp, step: &str, turn: u32) -> String {
    // Tool call ids are the session's: each turn's are its own.
    let (w1, w2) = (format!("w{turn}a"), format!("w{turn}b"));
    let words: Vec<&str> = step.split_whitespace().collect();
    let size = |after: &str| words.iter().position(|w| *w == after).and_then(|i| words.get(i + 1)).and_then(|w| w.parse::<f64>().ok());
    let body_after =
        |after: &str| words.iter().position(|w| *w == after).and_then(|i| words.get(i + 1)).map(|w| w.trim_end_matches([',', '.']).to_string());
    let lower = step.to_lowercase();
    if lower.starts_with("make a ") {
        let dims: Vec<f64> = step.split(|c: char| !(c.is_ascii_digit() || c == '.')).filter_map(|s| s.parse().ok()).collect();
        let name = body_after("named").unwrap_or_else(|| "Case".into());
        let [l, w, h] = [dims.first(), dims.get(1), dims.get(2)].map(|d| d.copied().unwrap_or(20.0));
        tool(session, &w1, "execute: solid.box", "in_progress", None);
        let (out, failed) = m.tool(
            "execute",
            json!({"command": "solid.box", "params": {"length": l, "width": w, "height": h, "corner": [0, 0, 0], "body_name": name}}),
        );
        tool(session, &w1, "execute: solid.box", if failed { "failed" } else { "completed" }, Some(&out));
        return if failed { format!("The box failed: {out}") } else { format!("Made {name}, {} × {} × {} mm.", mm(l), mm(w), mm(h)) };
    }
    if !["fillet", "shell", "drill"].iter().any(|v| lower.starts_with(v)) {
        return "Noted. I'm a scripted test agent: I suggest the next step when you pause, and do the steps I suggest.".into();
    }
    let Some(body) = body_after("of").or_else(|| body_after("Shell")) else { return "I couldn't tell which body that is.".into() };
    let Some((lo, hi)) = m.bbox(&body) else { return format!("I couldn't measure {body}.") };
    let (cx, cy) = ((lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0);
    let (command, params, done) = if lower.starts_with("fillet") {
        let r = size("with").unwrap_or(3.0);
        let z = (lo[2] + hi[2]) / 2.0;
        let edges = json!([[lo[0], lo[1], z], [hi[0], lo[1], z], [hi[0], hi[1], z], [lo[0], hi[1], z]]);
        ("solid.fillet", json!({"edges": edges, "radius": r, "body": body}), format!("Rounded the 4 vertical edges of {body} with {} mm.", mm(r)))
    } else if lower.starts_with("shell") {
        let t = size("with").unwrap_or(2.0);
        (
            "solid.shell",
            json!({"faces": [[cx, cy, hi[2]]], "thickness": t, "body": body}),
            format!("Shelled {body} from its top face, {} mm walls.", mm(t)),
        )
    } else if lower.starts_with("drill") {
        let d = size("a").unwrap_or(5.0);
        // From the bottom face up, through all: through a shelled body's floor, or a plate.
        (
            "solid.hole",
            json!({"position": [cx, cy, lo[2]], "direction": [0, 0, 1], "diameter": d}),
            format!("Drilled a {} mm hole through the centre of {body}.", mm(d)),
        )
    } else {
        return "I'm a scripted test agent: I only know the steps I suggest.".into();
    };
    let title = format!("execute: {command}");
    tool(session, &w2, &title, "in_progress", None);
    let (out, failed) = m.tool("execute", json!({"command": command, "params": params, "verbose": false}));
    tool(session, &w2, &title, if failed { "failed" } else { "completed" }, Some(&out));
    if failed { format!("That didn't work: {}", out.chars().take(300).collect::<String>()) } else { done }
}

fn main() {
    let mut sessions = 0u32;
    let mut session = String::new();
    let mut cost = 0.0;
    let mut mcp: Option<Mcp> = None;
    let mut turns = 0u32;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let reply = |result: Value| send(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
        match msg.get("method").and_then(Value::as_str) {
            Some("initialize") => reply(json!({
                "protocolVersion": 1,
                "agentInfo": {"name": "acp-next-step-agent", "title": "Scripted next-step agent", "version": env!("CARGO_PKG_VERSION")},
                "agentCapabilities": {"promptCapabilities": {"image": false}},
                "authMethods": [],
            })),
            Some("session/new") => {
                sessions += 1;
                session = format!("next-step-{sessions}");
                cost = 0.0;
                mcp = msg.pointer("/params/mcpServers/0").and_then(Mcp::start);
                reply(json!({"sessionId": session}));
            }
            Some("session/prompt") => {
                let prompt: String = msg
                    .pointer("/params/prompt")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"))
                    .unwrap_or_default();
                let answer = if prompt.starts_with("[Intcad next-step suggestion.") {
                    // Each question as it came, for whoever is checking what the agent is told.
                    if let Some(path) = std::env::var_os("NEXT_STEP_AGENT_LOG")
                        && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
                    {
                        let _ = writeln!(f, "===== {session}\n{prompt}");
                    }
                    let (step, reason) = guess(&prompt);
                    // With a project brief, it claims to follow the first note (for trying the
                    // card's memory block; it does not read the notes).
                    if prompt.contains("\n[1] ") {
                        json!({"step": step, "reason": reason, "memory": [1]}).to_string()
                    } else {
                        json!({"step": step, "reason": reason}).to_string()
                    }
                } else {
                    match mcp.as_mut() {
                        Some(m) => {
                            let step = prompt.lines().find(|l| !l.trim().is_empty() && !l.starts_with('[')).unwrap_or("").trim().to_string();
                            turns += 1;
                            work(&session, m, &step, turns)
                        }
                        None => "I have no Intcad tools in this session.".into(),
                    }
                };
                say(&session, &answer);
                let used = usage(&session, &mut cost, &prompt, &answer);
                reply(json!({"stopReason": "end_turn", "usage": used}));
            }
            Some("session/set_mode") => reply(json!({})),
            Some("session/cancel") => {}
            Some(_) if !id.is_null() => send(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}})),
            _ => {}
        }
    }
}
