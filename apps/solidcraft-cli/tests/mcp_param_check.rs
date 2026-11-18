//! `mcp --in script --param-check <level>`: the level applies to the script the session starts
//! from too, so a misspelled key in it is refused (strict) or reported on stderr (warn) instead of
//! silently ignored.

use std::process::{Command, Stdio};

#[test]
fn param_check_applies_to_the_starting_script() {
    let dir = std::env::temp_dir().join(format!("solidcraft-mcp-param-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("typo.json");
    let box_ = r#"{"command": "solid.box", "params": {"length": 5, "width": 5, "height": 5, "operaton": "cut"}}"#;
    std::fs::write(&script, format!(r#"{{"commands": [{box_}]}}"#)).unwrap();
    let run = |level: &str| {
        Command::new(env!("CARGO_BIN_EXE_intcad-cli"))
            .args(["mcp", "--in", script.to_str().unwrap(), "--param-check", level])
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let strict = run("strict");
    let stderr = String::from_utf8_lossy(&strict.stderr);
    assert!(!strict.status.success(), "{stderr}");
    assert!(stderr.contains("unknown key `operaton`; did you mean `operation`?"), "{stderr}");
    // Warn: the script runs, and its warnings go to stderr.
    let warn = run("warn");
    let stderr = String::from_utf8_lossy(&warn.stderr);
    assert!(warn.status.success(), "{stderr}");
    assert!(stderr.contains("step 1: unknown key `operaton`; did you mean `operation`?"), "{stderr}");
    // A warned step's warning is printed even when a later step fails.
    let failing = dir.join("typo_then_failure.json");
    std::fs::write(&failing, format!(r#"{{"commands": [{box_}, {{"command": "no.such_command"}}]}}"#)).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_intcad-cli"))
        .args(["mcp", "--in", failing.to_str().unwrap(), "--param-check", "warn"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("step 1: unknown key `operaton`") && stderr.contains("step 2"), "{stderr}");
    // Off: the script runs as before, and the server ends with its input.
    let off = run("off");
    assert!(off.status.success(), "{}", String::from_utf8_lossy(&off.stderr));
    assert!(!String::from_utf8_lossy(&off.stderr).contains("operaton"));
    // A bare flag is an error, not off.
    let bare = Command::new(env!("CARGO_BIN_EXE_intcad-cli")).args(["mcp", "--param-check"]).stdin(Stdio::null()).output().unwrap();
    assert!(!bare.status.success());
    assert!(String::from_utf8_lossy(&bare.stderr).contains("--param-check wants off, warn or strict"));
    std::fs::remove_dir_all(&dir).ok();
}

/// `mcp --compact` and `SOLIDCRAFT_MCP_COMPACT=1` are accepted and shape tool JSON: pretty-printed
/// by default, one line when on (the flag has to reach `Server::with_options`). A typo is an error,
/// not silently off. A headless stdio session keeps the test off any running app.
#[test]
fn compact_is_accepted_and_applies_to_tool_json() {
    use std::io::{BufRead, BufReader, Read, Write};

    fn ask(stdin: &mut impl Write, stdout: &mut impl BufRead, id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
        writeln!(stdin, "{}", serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"))
    }

    let run = |compact: bool, env: Option<&str>| -> (bool, String, String) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_intcad-cli"));
        cmd.args(["mcp", "--headless"]).env_remove("SOLIDCRAFT_MCP_COMPACT");
        if compact {
            cmd.arg("--compact");
        }
        if let Some(value) = env {
            cmd.env("SOLIDCRAFT_MCP_COMPACT", value);
        }
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut stderr = child.stderr.take().unwrap();
        ask(&mut stdin, &mut stdout, 1, "initialize", serde_json::json!({"protocolVersion": "2025-06-18"}));
        let reply = ask(&mut stdin, &mut stdout, 2, "tools/call", serde_json::json!({"name": "list_commands", "arguments": {"filter": "solid.box"}}));
        drop(stdin);
        let status = child.wait().unwrap();
        let mut err = String::new();
        stderr.read_to_string(&mut err).unwrap();
        (status.success(), reply["result"]["content"][0]["text"].as_str().unwrap_or_default().to_string(), err)
    };

    let (ok, pretty, err) = run(false, None);
    assert!(ok, "{err}");
    assert!(pretty.contains('\n'), "default tool JSON is pretty-printed: {pretty:?}");
    assert!(!err.contains("compact output"), "{err}");

    let (ok, compact, err) = run(true, None);
    assert!(ok, "{err}");
    assert!(!compact.contains('\n'), "--compact writes tool JSON on one line: {compact:?}");
    assert!(err.contains("compact output"), "the flag is announced on stderr: {err}");

    let (ok, from_env, err) = run(false, Some("1"));
    assert!(ok, "{err}");
    assert_eq!(from_env, compact, "SOLIDCRAFT_MCP_COMPACT=1 does what --compact does");

    // A typo is refused at start, before any protocol line is served.
    let bad = Command::new(env!("CARGO_BIN_EXE_intcad-cli"))
        .args(["mcp", "--headless"])
        .env("SOLIDCRAFT_MCP_COMPACT", "maybe")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(!bad.status.success(), "an unrecognised value is not silently off: {err}");
    assert!(err.contains("SOLIDCRAFT_MCP_COMPACT"), "{err}");
}
