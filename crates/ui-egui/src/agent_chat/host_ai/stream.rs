//! Bounded server-sent-event decoding for the direct providers.
//!
//! The reader thread owns the HTTP body and is always joined by its worker: there is no detached
//! task here. The parser is independent of the panel, host dispatch and CAD; it only turns wire
//! bytes into text deltas and a cached, still-unexecuted tool-call set.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use super::AiError;
use super::provider::{ToolArgumentError, WireProtocol};

/// One SSE event or `data:` line.
pub(crate) const MAX_EVENT_BYTES: usize = 256 << 10;
/// One tool call's accumulated argument text.
pub(crate) const MAX_TOOL_ARGUMENT_BYTES: usize = 1 << 20;
/// All body bytes read for one model round.
pub(crate) const MAX_STREAM_BYTES: usize = 16 << 20;
/// Pending reader-to-worker events. The reader stops rather than dropping a tool result.
pub(crate) const MAX_PENDING_EVENTS: usize = 512;
/// Tool calls declared by one model round.
pub(crate) const MAX_CALLS: usize = 128;

#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamLimits {
    pub event_bytes: usize,
    pub tool_argument_bytes: usize,
    pub stream_bytes: usize,
    pub pending_events: usize,
    pub calls: usize,
}

impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            event_bytes: MAX_EVENT_BYTES,
            tool_argument_bytes: MAX_TOOL_ARGUMENT_BYTES,
            stream_bytes: MAX_STREAM_BYTES,
            pending_events: MAX_PENDING_EVENTS,
            calls: MAX_CALLS,
        }
    }
}

#[derive(Debug)]
pub(crate) struct StreamCall {
    pub id: String,
    pub name: String,
    pub arguments: Result<Value, ToolArgumentError>,
    /// The provider's raw argument text, checked for secrets before any tool item is emitted.
    pub raw_arguments: String,
}

#[derive(Debug)]
pub(crate) struct StreamReply {
    pub calls: Vec<StreamCall>,
    pub finish: String,
    pub usage: Option<[Option<u32>; 3]>,
}

#[derive(Debug)]
pub(crate) enum Parsed {
    Text { thought: bool, text: String },
    Finished(StreamReply),
    Error(AiError),
}

/// Cancellation and failure shared by the worker and its reader.
pub(crate) struct StreamControl {
    closed: AtomicBool,
    failure: std::sync::Mutex<Option<AiError>>,
}

impl StreamControl {
    fn new() -> Self {
        Self { closed: AtomicBool::new(false), failure: std::sync::Mutex::default() }
    }

    fn stopped(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn fail(&self, error: AiError) {
        if let Ok(mut failure) = self.failure.lock()
            && failure.is_none()
        {
            *failure = Some(error);
        }
    }

    fn take_failure(&self) -> Option<AiError> {
        self.failure.lock().ok().and_then(|mut failure| failure.take())
    }
}

/// A reader that is owned, never detached.
pub(crate) struct StreamReader {
    control: Arc<StreamControl>,
    handle: Option<JoinHandle<()>>,
    events: Receiver<Parsed>,
}

impl StreamReader {
    pub(crate) fn spawn(reader: Box<dyn Read + Send>, wire: WireProtocol, limits: StreamLimits) -> Self {
        let control = Arc::new(StreamControl::new());
        let (tx, events) = sync_channel(limits.pending_events.max(1));
        let worker_control = Arc::clone(&control);
        // The handle stays in this value until shutdown joins it; every path in `invoke_stream`
        // calls shutdown, so the reader can never be detached.
        let handle =
            thread::Builder::new().name("solidcraft-ai-stream".into()).spawn(move || read_loop(reader, wire, limits, worker_control, tx)).ok();
        Self { control, handle, events }
    }

    pub(crate) fn recv_timeout(&self, wait: Duration) -> Result<Parsed, RecvTimeoutError> {
        self.events.recv_timeout(wait)
    }

    pub(crate) fn failure(&self) -> Option<AiError> {
        self.control.take_failure()
    }

    /// Ask the reader to stop, wait for the bounded read to return and join it. A slow join or a
    /// panicked reader is reported; it is never detached or described as a clean exit.
    pub(crate) fn shutdown(&mut self) -> Result<(), AiError> {
        self.control.closed.store(true, Ordering::Release);
        let Some(handle) = self.handle.take() else {
            return Err(AiError::internal("stream_reader_spawn"));
        };
        join_managed(handle, "stream_reader")
    }
}

/// Join a managed streaming thread. The transport must already have been shut down (or never
/// block indefinitely); this join is unconditional so a worker is never abandoned. A panic is
/// reported separately from a clean exit.
pub(crate) fn join_managed(handle: JoinHandle<()>, _operation: &'static str) -> Result<(), AiError> {
    match handle.join() {
        Ok(()) => Ok(()),
        Err(_) => Err(AiError::internal("stream_thread_panicked")),
    }
}

fn read_loop(reader: Box<dyn Read + Send>, wire: WireProtocol, limits: StreamLimits, control: Arc<StreamControl>, tx: SyncSender<Parsed>) {
    let mut reader = BufReader::new(reader);
    let result = parse_stream(&mut reader, wire, limits, &control, &tx);
    if let Err(error) = result {
        control.fail(error.clone());
        let _ = tx.try_send(Parsed::Error(error));
    }
}

fn send(tx: &SyncSender<Parsed>, control: &StreamControl, event: Parsed) -> Result<(), AiError> {
    match tx.try_send(event) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            let error = AiError::exhausted("stream_event_queue", MAX_PENDING_EVENTS as u32);
            control.fail(error.clone());
            Err(error)
        }
        Err(TrySendError::Disconnected(_)) => Err(AiError::cancelled()),
    }
}

fn parse_stream<R: BufRead>(
    reader: &mut R,
    wire: WireProtocol,
    limits: StreamLimits,
    control: &StreamControl,
    tx: &SyncSender<Parsed>,
) -> Result<(), AiError> {
    let mut parser = Parser::new(wire, limits);
    let mut event_name = String::new();
    let mut data = String::new();
    let mut total = 0usize;
    loop {
        if control.stopped() {
            return Err(AiError::cancelled());
        }
        let Some(line) = read_line(reader, limits.event_bytes, &mut total, limits.stream_bytes)? else {
            // EOF without the provider's finish signal is a protocol error, never a normal end.
            return Err(AiError::malformed());
        };
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            if !data.is_empty() || !event_name.is_empty() {
                if parser.dispatch(&event_name, &data, tx, control)? {
                    let reply = parser.finish()?;
                    send(tx, control, Parsed::Finished(reply))?;
                    return Ok(());
                }
                data.clear();
                event_name.clear();
            }
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => {
                if value.len() > limits.event_bytes {
                    return Err(AiError::exhausted("stream_event_bytes", limits.event_bytes as u32));
                }
                event_name = value.to_string();
            }
            "data" => {
                if data.len().saturating_add(value.len()) > limits.event_bytes {
                    return Err(AiError::exhausted("stream_event_bytes", limits.event_bytes as u32));
                }
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value);
            }
            _ => {}
        }
    }
}

/// One bounded line, counting raw bytes against the round's budget.
fn read_line<R: BufRead>(reader: &mut R, max: usize, total: &mut usize, stream_max: usize) -> Result<Option<String>, AiError> {
    let mut line = Vec::new();
    loop {
        if *total > stream_max {
            return Err(AiError::exhausted("stream_bytes", stream_max as u32));
        }
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(_) => return Err(AiError::transport()),
        };
        if available.is_empty() {
            return if line.is_empty() { Ok(None) } else { Ok(Some(String::from_utf8_lossy(&line).into_owned())) };
        }
        let take = available.iter().position(|b| *b == b'\n').map_or(available.len(), |at| at + 1);
        line.extend_from_slice(&available[..take]);
        *total = total.saturating_add(take);
        reader.consume(take);
        if *total > stream_max {
            return Err(AiError::exhausted("stream_bytes", stream_max as u32));
        }
        if line.len() > max {
            return Err(AiError::exhausted("stream_event_bytes", max as u32));
        }
        if line.last() == Some(&b'\n') {
            return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
        }
    }
}

struct Parser {
    wire: WireProtocol,
    limits: StreamLimits,
    calls: BTreeMap<usize, Acc>,
    seen_finish: bool,
    finish: Option<String>,
    usage: Option<[Option<u32>; 3]>,
}

#[derive(Default)]
struct Acc {
    id: Option<String>,
    name: String,
    arguments: String,
    complete: bool,
}

impl Parser {
    fn new(wire: WireProtocol, limits: StreamLimits) -> Self {
        Self { wire, limits, calls: BTreeMap::new(), seen_finish: false, finish: None, usage: None }
    }

    fn dispatch(&mut self, name: &str, data: &str, tx: &SyncSender<Parsed>, control: &StreamControl) -> Result<bool, AiError> {
        match self.wire {
            WireProtocol::Chat => self.chat(name, data, tx, control),
            WireProtocol::Messages => self.messages(name, data, tx, control),
            // These protocols are not verified for streaming; the caller must not reach here.
            WireProtocol::Responses | WireProtocol::Gemini => Err(AiError::unsupported("streaming")),
        }
    }

    fn chat(&mut self, _name: &str, data: &str, tx: &SyncSender<Parsed>, control: &StreamControl) -> Result<bool, AiError> {
        if data == "[DONE]" {
            if !self.seen_finish {
                return Err(AiError::malformed());
            }
            return Ok(true);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| AiError::malformed())?;
        if value.get("error").is_some() {
            return Err(AiError::model_failed());
        }
        if let Some(usage) = value.get("usage").and_then(Value::as_object) {
            self.usage = Some([
                usage.get("prompt_tokens").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()),
                usage.get("completion_tokens").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()),
                usage.get("total_tokens").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()),
            ]);
        }
        let Some(choices) = value.get("choices").and_then(Value::as_array) else { return Ok(false) };
        let Some(choice) = choices.first() else { return Ok(false) };
        if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
            if let Some(text) = delta.get("content").and_then(Value::as_str)
                && !text.is_empty()
            {
                send(tx, control, Parsed::Text { thought: false, text: text.to_string() })?;
            }
            if let Some(text) = delta.get("reasoning_content").or_else(|| delta.get("reasoning")).and_then(Value::as_str)
                && !text.is_empty()
            {
                send(tx, control, Parsed::Text { thought: true, text: text.to_string() })?;
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    self.chat_call(call)?;
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str)
            && !reason.is_empty()
        {
            if self.seen_finish && self.finish.as_deref() != Some(reason) {
                return Err(AiError::malformed());
            }
            self.seen_finish = true;
            self.finish = Some(reason.to_string());
            let cut = matches!(reason, "length" | "max_tokens" | "refusal");
            if cut {
                self.calls.clear();
            } else if reason == "tool_calls" {
                if self.calls.is_empty() {
                    return Err(AiError::malformed());
                }
                for call in self.calls.values_mut() {
                    call.complete = true;
                }
            } else if !self.calls.is_empty() {
                return Err(AiError::malformed());
            }
        }
        Ok(false)
    }

    fn chat_call(&mut self, call: &Value) -> Result<(), AiError> {
        let index = call.get("index").and_then(Value::as_u64).ok_or_else(AiError::malformed)? as usize;
        if !self.calls.contains_key(&index) {
            if self.calls.len() >= self.limits.calls {
                return Err(AiError::exhausted("provider_tool_calls", self.limits.calls as u32));
            }
            self.calls.insert(index, Acc::default());
        }
        let limit = self.limits.tool_argument_bytes;
        let acc = self.calls.get_mut(&index).ok_or_else(AiError::malformed)?;
        if let Some(id) = call.get("id").and_then(Value::as_str) {
            Self::assign_id(acc, id)?;
        }
        if let Some(name) = call.pointer("/function/name").and_then(Value::as_str)
            && !name.is_empty()
        {
            if !acc.name.is_empty() && acc.name != name {
                return Err(AiError::malformed());
            }
            acc.name = name.to_string();
        }
        if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
            Self::append_arguments(acc, arguments, limit)?;
        }
        Ok(())
    }

    fn messages(&mut self, name: &str, data: &str, tx: &SyncSender<Parsed>, control: &StreamControl) -> Result<bool, AiError> {
        let value: Value = serde_json::from_str(data).map_err(|_| AiError::malformed())?;
        match name {
            "error" => return Err(AiError::model_failed()),
            "message_start" => {
                if let Some(usage) = value.pointer("/message/usage").and_then(Value::as_object) {
                    self.usage = Some([
                        usage.get("input_tokens").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()),
                        usage.get("output_tokens").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()),
                        None,
                    ]);
                }
            }
            "content_block_start" => {
                let index = value.get("index").and_then(Value::as_u64).ok_or_else(AiError::malformed)? as usize;
                let block = value.get("content_block").ok_or_else(AiError::malformed)?;
                if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                    if !self.calls.contains_key(&index) {
                        if self.calls.len() >= self.limits.calls {
                            return Err(AiError::exhausted("provider_tool_calls", self.limits.calls as u32));
                        }
                        self.calls.insert(index, Acc::default());
                    }
                    let acc = self.calls.get_mut(&index).ok_or_else(AiError::malformed)?;
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        Self::assign_id(acc, id)?;
                    }
                    if let Some(name) = block.get("name").and_then(Value::as_str)
                        && !name.is_empty()
                    {
                        acc.name = name.to_string();
                    }
                }
            }
            "content_block_delta" => {
                let index = value.get("index").and_then(Value::as_u64).ok_or_else(AiError::malformed)? as usize;
                let delta = value.get("delta").ok_or_else(AiError::malformed)?;
                match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            send(tx, control, Parsed::Text { thought: false, text: text.to_string() })?;
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = delta.get("thinking").and_then(Value::as_str)
                            && !text.is_empty()
                        {
                            send(tx, control, Parsed::Text { thought: true, text: text.to_string() })?;
                        }
                    }
                    Some("input_json_delta") => {
                        let part = delta.get("partial_json").and_then(Value::as_str).unwrap_or("");
                        let limit = self.limits.tool_argument_bytes;
                        let acc = self.calls.get_mut(&index).ok_or_else(AiError::malformed)?;
                        Self::append_arguments(acc, part, limit)?;
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let index = value.get("index").and_then(Value::as_u64).ok_or_else(AiError::malformed)? as usize;
                if let Some(acc) = self.calls.get_mut(&index) {
                    acc.complete = true;
                }
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str)
                    && !reason.is_empty()
                {
                    self.seen_finish = true;
                    self.finish = Some(reason.to_string());
                }
                if let Some(usage) = value.get("usage").and_then(Value::as_object) {
                    let input = usage.get("input_tokens").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
                    let output = usage.get("output_tokens").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok());
                    let mut merged = self.usage.unwrap_or([None; 3]);
                    merged[0] = input.or(merged[0]);
                    merged[1] = output.or(merged[1]);
                    merged[2] = merged[0].zip(merged[1]).and_then(|(a, b)| a.checked_add(b));
                    self.usage = Some(merged);
                }
                let cut = matches!(self.finish.as_deref(), Some("max_tokens"));
                if cut {
                    self.calls.clear();
                } else {
                    let tool_use = self.finish.as_deref() == Some("tool_use");
                    if (tool_use && self.calls.is_empty()) || (!tool_use && !self.calls.is_empty()) {
                        return Err(AiError::malformed());
                    }
                }
            }
            "message_stop" => {
                if !self.seen_finish {
                    return Err(AiError::malformed());
                }
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    fn assign_id(acc: &mut Acc, id: &str) -> Result<(), AiError> {
        if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
            return Err(AiError::malformed());
        }
        if let Some(old) = &acc.id {
            if old != id {
                return Err(AiError::malformed());
            }
        } else {
            acc.id = Some(id.to_string());
        }
        Ok(())
    }

    fn append_arguments(acc: &mut Acc, part: &str, limit: usize) -> Result<(), AiError> {
        if acc.arguments.len().saturating_add(part.len()) > limit {
            return Err(AiError::exhausted("tool_json_bytes", limit as u32));
        }
        acc.arguments.push_str(part);
        Ok(())
    }

    fn finish(self) -> Result<StreamReply, AiError> {
        let finish = self.finish.ok_or_else(AiError::malformed)?;
        if !self.seen_finish {
            return Err(AiError::malformed());
        }
        let tool_finish = matches!(finish.as_str(), "tool_calls" | "tool_use");
        let mut ids = BTreeSet::new();
        let mut calls = Vec::with_capacity(self.calls.len());
        for (_, acc) in self.calls {
            if !acc.complete {
                return Err(AiError::malformed());
            }
            let id = acc.id.ok_or_else(AiError::malformed)?;
            let name = acc.name;
            if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(AiError::malformed());
            }
            if !ids.insert(id.clone()) {
                return Err(AiError::malformed());
            }
            let raw_arguments = acc.arguments.clone();
            let arguments = if acc.arguments.trim().is_empty() {
                Ok(serde_json::json!({}))
            } else {
                match serde_json::from_str::<Value>(&acc.arguments) {
                    Ok(value @ Value::Object(_)) => Ok(value),
                    Ok(_) => Err(ToolArgumentError::NotObject),
                    Err(_) => Err(ToolArgumentError::InvalidJson),
                }
            };
            calls.push(StreamCall { id, name, arguments, raw_arguments });
        }
        if tool_finish && calls.is_empty() {
            return Err(AiError::malformed());
        }
        if !tool_finish && !calls.is_empty() {
            return Err(AiError::malformed());
        }
        Ok(StreamReply { calls, finish, usage: self.usage })
    }
}

/// A rolling redactor that never emits the first `secret.len() - 1` characters of a possible key.
pub(crate) struct Redactor {
    secret: Option<String>,
    carry: String,
}

impl Redactor {
    pub(crate) fn new(secret: Option<&str>) -> Self {
        Self { secret: secret.filter(|secret| !secret.is_empty()).map(str::to_string), carry: String::new() }
    }

    /// Keep the rolling carry across model rounds unless the connection's key changed.
    pub(crate) fn set_secret(&mut self, secret: Option<&str>) {
        let next = secret.filter(|secret| !secret.is_empty()).map(str::to_string);
        if self.secret != next {
            self.secret = next;
            self.carry.clear();
        }
    }

    pub(crate) fn push(&mut self, text: &str) -> String {
        let Some(secret) = self.secret.clone() else { return text.to_string() };
        let mut buffer = std::mem::take(&mut self.carry);
        buffer.push_str(text);
        buffer = buffer.replace(&secret, "[redacted]");
        // Withhold only a suffix that could still become the key. Ordinary answer text is
        // emitted immediately; a real partial key is carried to the next chunk/round.
        let chars: Vec<char> = secret.chars().collect();
        let mut keep = 0;
        for len in (1..chars.len()).rev() {
            let prefix: String = chars[..len].iter().collect();
            if buffer.ends_with(&prefix) {
                keep = prefix.len();
                break;
            }
        }
        let split = buffer.len() - keep;
        self.carry = buffer.split_off(split);
        buffer
    }

    pub(crate) fn finish(&mut self) -> String {
        let mut rest = std::mem::take(&mut self.carry);
        if let Some(secret) = self.secret.clone() {
            rest = rest.replace(&secret, "[redacted]");
        }
        rest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn limits() -> StreamLimits {
        StreamLimits::default()
    }

    fn run(wire: WireProtocol, body: &str) -> Vec<Parsed> {
        let mut cursor = Cursor::new(body.as_bytes());
        let control = StreamControl::new();
        let (tx, rx) = sync_channel(MAX_PENDING_EVENTS);
        parse_stream(&mut cursor, wire, limits(), &control, &tx).expect("stream parses");
        drop(tx);
        rx.try_iter().collect()
    }

    fn finished(events: Vec<Parsed>) -> StreamReply {
        let mut reply = None;
        for event in events {
            if let Parsed::Finished(value) = event {
                reply = Some(value);
            }
        }
        reply.expect("one finished")
    }

    #[test]
    fn openai_text_splits_and_tool_arguments_are_only_cached_until_finish() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"he\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"llo\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"box\",\"arguments\":\"{\\\"a\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":4,\"total_tokens\":7}}\n\n",
            "data: [DONE]\n\n",
        );
        let reply = finished(run(WireProtocol::Chat, body));
        assert_eq!(reply.calls.len(), 1);
        assert_eq!(reply.calls[0].id, "call-1");
        assert_eq!(reply.calls[0].arguments.as_ref().unwrap(), &serde_json::json!({"a":1}));
        assert_eq!(reply.usage, Some([Some(3), Some(4), Some(7)]));
    }

    #[test]
    fn anthropic_blocks_stop_before_a_call_is_complete() {
        let body = concat!(
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"call-2\",\"name\":\"box\"}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"a\\\":\"}}\n\n",
            "event: content_block_stop\ndata: {\"index\":0}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let reply = finished(run(WireProtocol::Messages, body));
        assert_eq!(reply.calls[0].id, "call-2");
    }

    #[test]
    fn missing_finish_or_stops_are_protocol_errors() {
        for body in [
            "data: {\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n",
            "event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"c\",\"name\":\"box\"}}\n\n",
        ] {
            let mut cursor = Cursor::new(body.as_bytes());
            let control = StreamControl::new();
            let (tx, _rx) = sync_channel(MAX_PENDING_EVENTS);
            let wire = if body.starts_with("data:") { WireProtocol::Chat } else { WireProtocol::Messages };
            assert!(parse_stream(&mut cursor, wire, limits(), &control, &tx).is_err());
        }
    }

    #[test]
    fn duplicate_and_missing_call_ids_never_become_tools() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"same\",\"function\":{\"name\":\"box\",\"arguments\":\"{}\"}},{\"index\":1,\"id\":\"same\",\"function\":{\"name\":\"box\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let mut cursor = Cursor::new(body.as_bytes());
        let control = StreamControl::new();
        let (tx, _rx) = sync_channel(MAX_PENDING_EVENTS);
        assert!(parse_stream(&mut cursor, WireProtocol::Chat, limits(), &control, &tx).is_err());
    }

    #[test]
    fn a_valid_id_with_bad_arguments_stays_a_failed_call() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-3\",\"function\":{\"name\":\"box\",\"arguments\":\"{bad\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let reply = finished(run(WireProtocol::Chat, body));
        assert!(reply.calls[0].arguments.is_err());
    }

    #[test]
    fn limits_stop_the_reader_without_a_tool() {
        let mut small = limits();
        small.tool_argument_bytes = 4;
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"box\",\"arguments\":\"12345\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let mut cursor = Cursor::new(body.as_bytes());
        let control = StreamControl::new();
        let (tx, _rx) = sync_channel(MAX_PENDING_EVENTS);
        let error = parse_stream(&mut cursor, WireProtocol::Chat, small, &control, &tx).expect_err("limit");
        assert_eq!(error.code(), super::super::AiErrorCode::ResourceExhausted);
    }

    struct OneByte<'a>(&'a [u8]);

    impl std::io::Read for OneByte<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some((first, rest)) = self.0.split_first() else { return Ok(0) };
            if let Some(out) = buf.first_mut() {
                *out = *first;
            }
            self.0 = rest;
            Ok(1)
        }
    }

    #[test]
    fn utf8_split_across_reads_is_decoded_once() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"café\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let mut reader = BufReader::new(OneByte(body.as_bytes()));
        let control = StreamControl::new();
        let (tx, rx) = sync_channel(MAX_PENDING_EVENTS);
        parse_stream(&mut reader, WireProtocol::Chat, limits(), &control, &tx).expect("stream parses");
        drop(tx);
        let text: String = rx.try_iter().filter_map(|e| if let Parsed::Text { text, .. } = e { Some(text) } else { None }).collect();
        assert_eq!(text, "café");
    }

    #[test]
    fn one_event_line_and_the_whole_round_are_bounded() {
        let mut line_limits = limits();
        line_limits.event_bytes = 32;
        let body = format!("data: \"{}\"\n\n", "x".repeat(64));
        let mut cursor = Cursor::new(body.as_bytes());
        let control = StreamControl::new();
        let (tx, _rx) = sync_channel(MAX_PENDING_EVENTS);
        assert!(parse_stream(&mut cursor, WireProtocol::Chat, line_limits, &control, &tx).is_err());

        let mut total_limits = limits();
        total_limits.stream_bytes = 64;
        let body = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n", "y".repeat(128));
        let mut cursor = Cursor::new(body.as_bytes());
        let control = StreamControl::new();
        let (tx, _rx) = sync_channel(MAX_PENDING_EVENTS);
        assert!(parse_stream(&mut cursor, WireProtocol::Chat, total_limits, &control, &tx).is_err());
    }

    struct PanicRead;
    impl Read for PanicRead {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            panic!("synthetic reader panic")
        }
    }

    #[test]
    fn a_panicking_reader_is_joined_and_reported() {
        let mut reader = StreamReader::spawn(Box::new(PanicRead), WireProtocol::Chat, limits());
        thread::sleep(Duration::from_millis(50));
        let error = reader.shutdown().expect_err("panic must be reported");
        assert_eq!(error.code(), super::super::AiErrorCode::InternalError);
    }

    #[test]
    fn anthropic_usage_merges_start_and_delta() {
        let body = concat!(
            "event: message_start\ndata: {\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
            "event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":5}}\n\n",
            "event: message_stop\ndata: {}\n\n",
        );
        let reply = finished(run(WireProtocol::Messages, body));
        assert_eq!(reply.usage, Some([Some(10), Some(5), Some(15)]));
    }

    #[test]
    fn interleaved_calls_keep_provider_index_order_and_do_not_merge() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"second\",\"function\":{\"name\":\"box\",\"arguments\":\"{\\\"b\\\":2}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"first\",\"function\":{\"name\":\"box\",\"arguments\":\"{\\\"a\\\":1}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\" \"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let reply = finished(run(WireProtocol::Chat, body));
        assert_eq!(reply.calls.len(), 2);
        assert_eq!(reply.calls[0].id, "first");
        assert_eq!(reply.calls[1].id, "second");
        assert_eq!(reply.calls[0].arguments.as_ref().unwrap(), &serde_json::json!({"a":1}));
        assert_eq!(reply.calls[1].arguments.as_ref().unwrap(), &serde_json::json!({"b":2}));
    }

    #[test]
    fn redaction_carries_across_chunks_and_utf8_boundaries() {
        let secret = "sk-secret-123";
        let mut redactor = Redactor::new(Some(secret));
        let mut shown = String::new();
        for part in ["x sk-se", "cret-1", "23 y", " é"] {
            shown.push_str(&redactor.push(part));
        }
        shown.push_str(&redactor.finish());
        assert!(!shown.contains(secret));
        assert!(shown.contains("x "));
        assert!(shown.contains(" y "));
    }

    #[test]
    fn a_secret_split_exactly_at_a_round_boundary_is_still_hidden() {
        let mut redactor = Redactor::new(Some("round-secret"));
        let first = redactor.push("round-");
        assert!(!first.contains("round-secret"));
        let second = redactor.push("secret");
        let last = redactor.finish();
        let all = format!("{first}{second}{last}");
        assert!(!all.contains("round-secret"));
    }
}
