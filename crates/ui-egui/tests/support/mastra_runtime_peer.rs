//! Original test-only, std-only exact-byte peer; not a product executable.
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CAP: usize = 32 << 20;
const ESSENTIALS: &[&str] = &["SYSTEMROOT", "WINDIR", "TEMP", "TMP", "TMPDIR", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "SSL_CERT_FILE", "SSL_CERT_DIR"];

fn bytes(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() > 32768 || !hex.len().is_multiple_of(2) {
        return Err("invalid bounded hex".into());
    }
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| "hex")?;
            u8::from_str_radix(text, 16).map_err(|_| "hex".into())
        })
        .collect()
}

#[cfg(unix)]
struct Bridge(std::os::unix::net::UnixStream);
#[cfg(not(unix))]
struct Bridge;

#[cfg(unix)]
impl Bridge {
    fn from_fd(fd: i32) -> Result<Self, String> {
        use std::os::fd::FromRawFd;
        if fd < 0 {
            return Err("negative bridge fd".into());
        }
        // The peer is a test-only executable; the host cleared CLOEXEC on this duplicate.
        let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        Ok(Self(stream))
    }

    fn read_line(&mut self) -> Result<Vec<u8>, String> {
        let mut line = Vec::new();
        let mut byte = [0_u8; 1];
        loop {
            if self.0.read(&mut byte).map_err(|e| e.to_string())? == 0 {
                return Err("bridge EOF".into());
            }
            if byte[0] == b'\n' {
                return Ok(line);
            }
            if line.len() >= 8 << 20 {
                return Err("bridge line too large".into());
            }
            line.push(byte[0]);
        }
    }
}

fn quoted(line: &str, key: &str) -> Option<String> {
    let start = line.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

fn object_after(line: &str, key: &str) -> Option<String> {
    let start = line.find(&format!("\"{key}\":"))? + key.len() + 3;
    let rest = &line[start..];
    let open = rest.find('{')?;
    let mut depth = 0;
    for (index, byte) in rest.bytes().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(rest[open..=index].to_owned());
                }
            }
            _ => {}
        }
    }
    None
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        return Err("one exact script path required".into());
    }
    if std::env::vars_os().any(|(key, _)| !ESSENTIALS.iter().any(|allowed| key.eq_ignore_ascii_case(allowed))) {
        return Err("unexpected inherited environment".into());
    }
    let path = Path::new(&args[1]);
    if path.parent() != Some(std::env::current_dir().map_err(|e| e.to_string())?.as_path()) {
        return Err("wrong cwd".into());
    }
    let mut script = String::new();
    std::fs::File::open(path).map_err(|e| e.to_string())?.take(1 << 20).read_to_string(&mut script).map_err(|e| e.to_string())?;
    let mut bridge_fd = None;
    for index in 2..args.len() {
        if let Some(value) = args[index].strip_prefix("--bridge-fd=") {
            bridge_fd = Some(value.parse::<i32>().map_err(|_| "bridge fd")?);
        } else if args[index] == "--bridge-fd" {
            bridge_fd = Some(args.get(index + 1).ok_or("bridge fd")?.parse::<i32>().map_err(|_| "bridge fd")?);
        } else if !args[index].starts_with("--bridge-capabilities") {
            return Err("unexpected peer argument".into());
        }
    }
    #[cfg(unix)]
    let mut bridge = bridge_fd.map(Bridge::from_fd).transpose()?;
    #[cfg(not(unix))]
    let mut bridge: Option<Bridge> = None;
    let _ = &mut bridge;
    let mut input = std::io::stdin().lock();
    let mut current = String::new();
    let mut last_request = String::new();
    let mut last_scope = String::new();
    for line in script.lines() {
        let mut words = line.split_whitespace();
        let Some(op) = words.next() else { continue };
        match op {
            "READ_PROTO" => {
                current.clear();
                input.read_line(&mut current).map_err(|e| e.to_string())?;
                last_request = quoted(&current, "requestId").ok_or("requestId")?;
                last_scope = object_after(&current, "scope").ok_or("scope")?;
            }
            "REPLY_METHOD_HEX" => {
                let method = String::from_utf8(bytes(words.next().ok_or("method hex")?)?).map_err(|_| "method")?;
                let result = bytes(words.next().ok_or("result hex")?)?;
                if !current.contains(&format!("\"method\":\"{method}\"")) {
                    return Err(format!("expected request {method}"));
                }
                let reply = format!(
                    "{{\"protocolVersion\":\"1.0\",\"kind\":\"response\",\"requestId\":\"{last_request}\",\"scope\":{last_scope},\"outcome\":\"ok\",\"result\":{}}}",
                    String::from_utf8(result).map_err(|_| "result text")?
                );
                std::io::stdout().write_all(reply.as_bytes()).map_err(|e| e.to_string())?;
                std::io::stdout().write_all(b"\n").map_err(|e| e.to_string())?;
                std::io::stdout().flush().map_err(|e| e.to_string())?;
            }
            "EVENT_LAST" => {
                let sequence: u32 = words.next().ok_or("sequence")?.parse().map_err(|_| "sequence")?;
                let event = words.next().ok_or("event")?;
                let data = String::from_utf8(bytes(words.next().ok_or("data hex")?)?).map_err(|_| "data")?;
                let frame = format!(
                    "{{\"protocolVersion\":\"1.0\",\"kind\":\"event\",\"sequence\":{sequence},\"event\":\"{event}\",\"requestId\":\"{last_request}\",\"scope\":{last_scope},\"data\":{data}}}"
                );
                std::io::stdout().write_all(frame.as_bytes()).map_err(|e| e.to_string())?;
                std::io::stdout().write_all(b"\n").map_err(|e| e.to_string())?;
                std::io::stdout().flush().map_err(|e| e.to_string())?;
            }
            "MCP_WRITE_HEX" => {
                #[cfg(not(unix))]
                return Err("the MCP bridge is unavailable".into());
                #[cfg(unix)]
                {
                    let bridge = bridge.as_mut().ok_or("no bridge fd")?;
                    let line = bytes(words.next().ok_or("missing hex")?)?;
                    bridge.0.write_all(&line).map_err(|e| e.to_string())?;
                    bridge.0.write_all(b"\n").map_err(|e| e.to_string())?;
                    bridge.0.flush().map_err(|e| e.to_string())?;
                }
            }
            "MCP_READ_LINE" => {
                #[cfg(not(unix))]
                return Err("the MCP bridge is unavailable".into());
                #[cfg(unix)]
                {
                    let bridge = bridge.as_mut().ok_or("no bridge fd")?;
                    let _line = bridge.read_line()?;
                }
            }
            "EXPECT_PAIR_HEX" => {
                let first = bytes(words.next().ok_or("first frame")?)?;
                let second = bytes(words.next().ok_or("second frame")?)?;
                let mut got = vec![0; first.len() + second.len()];
                input.read_exact(&mut got).map_err(|e| e.to_string())?;
                if got != [first.as_slice(), second.as_slice()].concat() && got != [second.as_slice(), first.as_slice()].concat() {
                    return Err("atomic frame pair differs".into());
                }
            }
            "WRITE_HEX" | "STDERR_HEX" | "EXPECT_HEX" => {
                let pattern = bytes(words.next().ok_or("missing hex")?)?;
                if pattern.is_empty() {
                    return Err("empty hex".into());
                }
                let repeats: usize = words.next().unwrap_or("1").parse().map_err(|_| "repeat")?;
                let total = pattern.len().checked_mul(repeats).filter(|n| *n <= CAP).ok_or("byte cap")?;
                let mut block = [0_u8; 8192];
                let mut offset = 0;
                while offset < total {
                    let count = (total - offset).min(block.len());
                    for (i, b) in block.iter_mut().take(count).enumerate() {
                        *b = pattern[(offset + i) % pattern.len()];
                    }
                    let chunk = &block[..count];
                    if op == "EXPECT_HEX" {
                        let mut got = [0_u8; 8192];
                        input.read_exact(&mut got[..count]).map_err(|e| e.to_string())?;
                        if &got[..count] != chunk {
                            return Err("input bytes differ".into());
                        }
                    } else if op == "WRITE_HEX" {
                        std::io::stdout().write_all(chunk).map_err(|e| e.to_string())?;
                    } else {
                        std::io::stderr().write_all(chunk).map_err(|e| e.to_string())?;
                    }
                    offset += count;
                }
                if op == "WRITE_HEX" {
                    std::io::stdout().flush().map_err(|e| e.to_string())?;
                }
                if op == "STDERR_HEX" {
                    std::io::stderr().flush().map_err(|e| e.to_string())?;
                }
            }
            "EXPECT_EOF" => {
                let mut byte = [0];
                if input.read(&mut byte).map_err(|e| e.to_string())? != 0 {
                    return Err("expected EOF".into());
                }
            }
            "SPAWN_HELPER" => {
                let helper_script = line.strip_prefix(op).map(str::trim).filter(|s| !s.is_empty()).ok_or("helper script")?;
                for _ in words.by_ref() {}
                let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                    .arg(helper_script)
                    .stdin(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                println!("{}", child.id());
                std::io::stdout().flush().map_err(|e| e.to_string())?;
            }
            "SPAWN_HELPER_TRACKED" => {
                let script = String::from_utf8(bytes(words.next().ok_or("helper script hex")?)?).map_err(|_| "helper script path")?;
                let marker = String::from_utf8(bytes(words.next().ok_or("helper marker hex")?)?).map_err(|_| "helper marker path")?;
                let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
                if Path::new(&script).parent() != Some(cwd.as_path()) || Path::new(&marker).parent() != Some(cwd.as_path()) {
                    return Err("helper outside scenario".into());
                }
                let child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
                    .arg(script)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                std::fs::write(marker, child.id().to_string()).map_err(|e| e.to_string())?;
            }
            "HOLD_INPUT" => {
                let marker = Path::new(line.strip_prefix(op).map(str::trim).filter(|s| !s.is_empty()).ok_or("release marker")?);
                for _ in words.by_ref() {}
                let until = Instant::now() + Duration::from_secs(30);
                while !marker.exists() {
                    if Instant::now() >= until {
                        return Err("peer watchdog".into());
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            "MARK" => {
                let marker = Path::new(line.strip_prefix(op).map(str::trim).filter(|s| !s.is_empty()).ok_or("marker")?);
                for _ in words.by_ref() {}
                if marker.parent() != Some(std::env::current_dir().map_err(|e| e.to_string())?.as_path()) {
                    return Err("marker outside scenario".into());
                }
                std::fs::write(marker, b"reached").map_err(|e| e.to_string())?;
            }
            "EXIT" => {
                let code: i32 = words.next().ok_or("exit code")?.parse().map_err(|_| "exit code")?;
                std::process::exit(code);
            }
            _ => return Err("unknown operation".into()),
        }
        if words.next().is_some() {
            return Err("extra script arguments".into());
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("peer failed: {error}");
        std::process::exit(1);
    }
}
