use std::collections::{HashMap, VecDeque};
use std::io::{self, BufRead, BufWriter, Write};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionCapabilities, SessionFocus,
    SessionInventory, TerminalSessionService, TextInput,
};
use serde_json::{json, Value};

use super::last_send::LastSendStore;

const PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "qol-sessions-mcp";

const ERROR_PARSE: i64 = -32700;
const ERROR_INVALID_REQUEST: i64 = -32600;
const ERROR_METHOD_NOT_FOUND: i64 = -32601;
const ERROR_INVALID_PARAMS: i64 = -32602;

const QUEUE_CAPACITY_PER_SESSION: usize = 8;
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub(super) const WAIT_TIMEOUT_DEFAULT_MS: u64 = 30_000;
pub(super) const WAIT_TIMEOUT_MAX_MS: u64 = 600_000;

struct QueueState {
    queues: HashMap<SessionBinding, VecDeque<(String, DeliveryMode)>>,
    last_error: HashMap<SessionBinding, String>,
}

struct DeliveryQueue {
    terminals: Arc<TerminalSessionService>,
    state: Arc<Mutex<QueueState>>,
    wake: Arc<Condvar>,
}

impl DeliveryQueue {
    fn new(terminals: Arc<TerminalSessionService>, worker: bool) -> Self {
        let queue = Self {
            terminals,
            state: Arc::new(Mutex::new(QueueState {
                queues: HashMap::new(),
                last_error: HashMap::new(),
            })),
            wake: Arc::new(Condvar::new()),
        };
        if worker {
            queue.spawn_worker();
        }
        queue
    }

    fn spawn_worker(&self) {
        let terminals = Arc::clone(&self.terminals);
        let state = Arc::clone(&self.state);
        let wake = Arc::clone(&self.wake);
        std::thread::spawn(move || loop {
            let item = {
                let mut state = state.lock().unwrap();
                loop {
                    let take = {
                        let mut take = None;
                        for (binding, queue) in state.queues.iter_mut() {
                            if let Some(item) = queue.pop_front() {
                                take = Some((binding.clone(), item));
                                break;
                            }
                        }
                        take
                    };
                    if let Some(item) = take {
                        break Some(item);
                    }
                    if state.queues.is_empty() {
                        state = wake.wait(state).unwrap();
                    } else {
                        state.queues.retain(|_, queue| !queue.is_empty());
                        break None;
                    }
                }
            };
            let Some((binding, (text, mode))) = item else {
                continue;
            };
            if let Err(error) = terminals.send_text(&binding, &text, mode) {
                state
                    .lock()
                    .unwrap()
                    .last_error
                    .insert(binding, error.to_string());
            }
        });
    }

    fn enqueue(
        &self,
        binding: SessionBinding,
        text: String,
        mode: DeliveryMode,
    ) -> Result<usize, String> {
        let mut state = self.state.lock().unwrap();
        let queue = state.queues.entry(binding).or_default();
        if queue.len() >= QUEUE_CAPACITY_PER_SESSION {
            return Err(format!(
                "delivery queue full ({QUEUE_CAPACITY_PER_SESSION} pending for this session)"
            ));
        }
        queue.push_back((text, mode));
        self.wake.notify_all();
        Ok(queue.len())
    }

    fn pending(&self, binding: &SessionBinding) -> usize {
        self.state
            .lock()
            .unwrap()
            .queues
            .get(binding)
            .map_or(0, VecDeque::len)
    }

    fn take_last_error(&self, binding: &SessionBinding) -> Option<String> {
        self.state.lock().unwrap().last_error.remove(binding)
    }

    fn wait_for_empty(&self, binding: &SessionBinding) {
        let mut state = self.state.lock().unwrap();
        while state
            .queues
            .get(binding)
            .is_some_and(|queue| !queue.is_empty())
        {
            state = self.wake.wait(state).unwrap();
        }
    }

    #[cfg(test)]
    fn drain_manual(&self) {
        loop {
            let item = {
                let mut state = self.state.lock().unwrap();
                let mut take = None;
                for (binding, queue) in state.queues.iter_mut() {
                    if let Some(item) = queue.pop_front() {
                        take = Some((binding.clone(), item));
                        break;
                    }
                }
                if take.is_none() {
                    state.queues.retain(|_, queue| !queue.is_empty());
                }
                take
            };
            let Some((binding, (text, mode))) = item else {
                break;
            };
            if let Err(error) = self.terminals.send_text(&binding, &text, mode) {
                self.state
                    .lock()
                    .unwrap()
                    .last_error
                    .insert(binding, error.to_string());
            }
        }
    }
}

pub(crate) struct McpSessionServer {
    terminals: Arc<TerminalSessionService>,
    interpreter: CliSessionInterpreter,
    queue: DeliveryQueue,
    store: LastSendStore,
}

impl McpSessionServer {
    pub(crate) fn system() -> Self {
        let terminals = Arc::new(TerminalSessionService::system());
        let queue = DeliveryQueue::new(Arc::clone(&terminals), true);
        Self {
            terminals,
            interpreter: CliSessionInterpreter::system(),
            queue,
            store: LastSendStore::system().unwrap_or_else(|| {
                LastSendStore::with_dir(std::env::temp_dir().join("qol-sessions-last-send"))
            }),
        }
    }

    #[cfg(test)]
    fn with(
        terminals: TerminalSessionService,
        interpreter: CliSessionInterpreter,
        store: LastSendStore,
    ) -> Self {
        let terminals = Arc::new(terminals);
        let queue = DeliveryQueue::new(Arc::clone(&terminals), false);
        Self {
            terminals,
            interpreter,
            queue,
            store,
        }
    }

    fn handle_line(&self, line: &str) -> Option<Value> {
        if line.trim().is_empty() {
            return None;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(_) => return Some(error(None, ERROR_PARSE, "parse error: invalid JSON")),
        };
        message.get("id")?;
        let method = match message.get("method").and_then(Value::as_str) {
            Some(method) => method,
            None => {
                return Some(error(
                    message.get("id").cloned(),
                    ERROR_INVALID_REQUEST,
                    "invalid request: missing method",
                ))
            }
        };
        let id = message.get("id").cloned().unwrap();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        Some(self.handle(method, params, id))
    }

    fn handle(&self, method: &str, params: Value, id: Value) -> Value {
        match method {
            "initialize" => {
                let requested = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .filter(|version| version.starts_with("2024-") || version.starts_with("2025-"))
                    .unwrap_or(PROTOCOL_VERSION);
                result(
                    id,
                    json!({
                        "protocolVersion": requested,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": {
                            "name": SERVER_NAME,
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                )
            }
            "ping" => result(id, json!({})),
            "tools/list" => result(id, json!({ "tools": tool_definitions() })),
            "tools/call" => self.call_tool(id, params),
            "notifications/initialized"
            | "notifications/cancelled"
            | "notifications/roots/list_changed" => result(id, json!({})),
            _ => error(
                Some(id),
                ERROR_METHOD_NOT_FOUND,
                format!("method not found: {method}"),
            ),
        }
    }

    fn call_tool(&self, id: Value, params: Value) -> Value {
        let name = params.get("name").and_then(Value::as_str);
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let Some(name) = name else {
            return error(
                Some(id),
                ERROR_INVALID_PARAMS,
                "tools/call requires a tool name",
            );
        };
        let outcome = match name {
            "sessions_list" => self.tool_list_sessions(),
            "session_read_screen" => self.tool_read_screen(arguments),
            "session_send_text" => self.tool_send_text(arguments),
            "session_wait_output" => self.tool_wait_output(arguments),
            "session_focus" => self.tool_focus(arguments),
            other => {
                return error(
                    Some(id),
                    ERROR_INVALID_PARAMS,
                    format!("unknown tool: {other}"),
                )
            }
        };
        let (text, is_error) = match outcome {
            Ok(text) => (text, false),
            Err(message) => (message, true),
        };
        result(
            id,
            json!({
                "content": [{ "type": "text", "text": text }],
                "isError": is_error,
            }),
        )
    }

    fn tool_list_sessions(&self) -> Result<String, String> {
        let facts = self
            .terminals
            .discover()
            .map_err(|error| format!("discovery failed: {error}"))?;
        let rows = facts
            .iter()
            .filter_map(|session| {
                let binding = session.binding().ok()?;
                let descriptor = self.interpreter.describe(session);
                Some(json!({
                    "session": binding.token(),
                    "backend": session.id.backend().to_string(),
                    "native": session.id.native(),
                    "root_pid": session.root_pid,
                    "tool": descriptor.tool.id.to_string(),
                    "display_name": descriptor.display_name,
                    "title": session.title,
                    "cwd": session.cwd,
                    "at_prompt": session.at_prompt,
                    "activity": descriptor.has_activity,
                    "pending_input": self.queue.pending(&binding),
                    "capabilities": capability_names(&session.capabilities),
                }))
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&rows).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_read_screen(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        self.terminals
            .read_screen(&binding)
            .map_err(|error| format!("screen read failed: {error}"))
    }

    fn tool_send_text(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        let text = arguments
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| "send_text requires a non-empty `text` string".to_owned())?;
        let submit = arguments
            .get("submit")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mode = if submit {
            DeliveryMode::Submit
        } else {
            DeliveryMode::Insert
        };
        let position = self.queue.enqueue(binding.clone(), text.to_owned(), mode)?;
        self.store.record(&binding, text);
        let verb = if submit { "submitted" } else { "inserted" };
        Ok(format!(
            "queued at {position} to {binding}; {verb} in flight"
        ))
    }

    fn tool_wait_output(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(WAIT_TIMEOUT_DEFAULT_MS)
            .clamp(1_000, WAIT_TIMEOUT_MAX_MS);
        let expect = arguments
            .get("expect")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|pattern| !pattern.is_empty());
        self.queue.wait_for_empty(&binding);
        if let Some(error) = self.queue.take_last_error(&binding) {
            return Err(format!("delivery failed: {error}"));
        }
        let last_sent = self.store.last_sent(&binding);
        let (settled, screen, polls, started) = poll_until_settled(
            self.terminals.as_ref(),
            &binding,
            Duration::from_millis(timeout_ms),
            expect.as_deref(),
            last_sent.as_deref(),
        )?;
        Ok(wait_result(settled, screen, polls, started))
    }

    fn tool_focus(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        self.terminals
            .focus(&binding)
            .map_err(|error| format!("focus failed: {error}"))?;
        Ok(format!("focused {binding}"))
    }
}

pub(crate) fn run() -> Result<()> {
    let server = McpSessionServer::system();
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut output = BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = line?;
        if let Some(response) = server.handle_line(&line) {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn binding_argument(arguments: &Value, name: &str) -> Result<SessionBinding, String> {
    let value = arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing `{name}` string argument"))?;
    value
        .parse()
        .map_err(|_| format!("invalid session token `{value}`"))
}

pub(super) fn poll_until_settled(
    terminals: &TerminalSessionService,
    binding: &SessionBinding,
    timeout: Duration,
    expect: Option<&str>,
    last_sent: Option<&str>,
) -> Result<(bool, String, u64, Instant), String> {
    let started = Instant::now();
    let mut previous: Option<String> = None;
    let mut changed = false;
    let mut matched = false;
    let mut polls = 0_u64;
    loop {
        let screen = terminals
            .read_screen(binding)
            .map_err(|error| format!("screen read failed: {error}"))?;
        polls += 1;
        if let Some(pattern) = expect {
            if !matched && pattern_visible(&screen, pattern, last_sent) {
                matched = true;
            }
            if matched && previous.as_deref() == Some(screen.as_str()) {
                return Ok((true, screen, polls, started));
            }
        } else if let Some(last) = &previous {
            if *last != screen {
                changed = true;
            } else if changed {
                return Ok((true, screen, polls, started));
            }
        }
        previous = Some(screen.clone());
        if started.elapsed() >= timeout {
            return Ok((false, screen, polls, started));
        }
        std::thread::sleep(WAIT_POLL_INTERVAL);
    }
}

fn pattern_visible(screen: &str, pattern: &str, last_sent: Option<&str>) -> bool {
    if !screen.contains(pattern) {
        return false;
    }
    let Some(sent) = last_sent.filter(|text| !text.is_empty()) else {
        return true;
    };
    screen
        .lines()
        .any(|line| line.contains(pattern) && !line.contains(sent))
}

fn wait_result(settled: bool, screen: String, polls: u64, started: Instant) -> String {
    let elapsed_ms = started.elapsed().as_millis();
    json!({ "settled": settled, "screen": screen, "polls": polls, "elapsed_ms": elapsed_ms })
        .to_string()
}

fn tool_definitions() -> Vec<Value> {
    super::contract::tool_specs()
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "description": spec.description,
                "inputSchema": spec.input_schema,
            })
        })
        .collect()
}

fn capability_names(capabilities: &SessionCapabilities) -> Vec<&'static str> {
    let mut names = Vec::new();
    if capabilities.contains(SessionCapabilities::SCREEN_READING) {
        names.push("read");
    }
    if capabilities.contains(SessionCapabilities::FOCUS) {
        names.push("focus");
    }
    if capabilities.contains(SessionCapabilities::TEXT_INPUT) {
        names.push("input");
    }
    names
}

fn result(id: Value, value: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": value })
}

fn error(id: Option<Value>, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message.into() }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::{
        SessionFacts, SessionFocus, SessionId, TerminalBackend, TerminalError,
    };
    use std::str::FromStr;
    use std::sync::{Arc, Mutex};

    struct FakeBackend {
        sent: Mutex<Vec<(SessionBinding, String, DeliveryMode)>>,
        screens: Mutex<VecDeque<String>>,
        last: Mutex<Option<String>>,
    }

    impl FakeBackend {
        fn with_screens(screens: Vec<String>) -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
                screens: Mutex::new(screens.into()),
                last: Mutex::new(None),
            }
        }

        fn session() -> SessionFacts {
            SessionFacts {
                id: SessionId::new(qol_terminal_sessions::BackendId::new("fake").unwrap(), "7")
                    .expect("fake session id"),
                root_pid: 123,
                cwd: "/work/demo".to_owned(),
                title: "Demo REPL".to_owned(),
                at_prompt: true,
                reported_cmd: Some("python3".to_owned()),
                foreground_basenames: Vec::new(),
                foreground_pids: Vec::new(),
                capabilities: SessionCapabilities::ALL,
            }
        }
    }

    impl SessionInventory for FakeBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            Ok(vec![Self::session()])
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            let mut screens = self.screens.lock().unwrap();
            if let Some(screen) = screens.pop_front() {
                *self.last.lock().unwrap() = Some(screen.clone());
                return Ok(screen);
            }
            Ok(self
                .last
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| ">>> ready".to_owned()))
        }
    }

    impl SessionFocus for FakeBackend {
        fn focus(&self, _target: &SessionBinding) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TextInput for FakeBackend {
        fn send_text(
            &self,
            target: &SessionBinding,
            text: &str,
            mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            self.sent
                .lock()
                .unwrap()
                .push((target.clone(), text.to_owned(), mode));
            Ok(())
        }
    }

    impl TerminalBackend for FakeBackend {
        fn id(&self) -> &qol_terminal_sessions::BackendId {
            static ID: std::sync::OnceLock<qol_terminal_sessions::BackendId> =
                std::sync::OnceLock::new();
            ID.get_or_init(|| qol_terminal_sessions::BackendId::new("fake").unwrap())
        }

        fn read_screen_from_snapshot(
            &self,
            _snapshot: &qol_terminal_sessions::TerminalSnapshot,
            _target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Ok(">>> ready".to_owned())
        }
    }

    fn server() -> (McpSessionServer, Arc<FakeBackend>) {
        server_with_screens(Vec::new())
    }

    fn server_with_screens(screens: Vec<String>) -> (McpSessionServer, Arc<FakeBackend>) {
        let backend = Arc::new(FakeBackend::with_screens(screens));
        let service =
            TerminalSessionService::from_backends([backend.clone() as Arc<dyn TerminalBackend>])
                .expect("unique fake backend");
        let store_dir = Box::leak(Box::new(tempfile::TempDir::new().unwrap()));
        let store = LastSendStore::with_dir(store_dir.path().to_path_buf());
        (
            McpSessionServer::with(service, CliSessionInterpreter::system(), store),
            backend,
        )
    }

    fn token() -> String {
        FakeBackend::session().binding().unwrap().token()
    }

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn initialize_handshake_echoes_version_and_advertises_tools() {
        let (server, _) = server();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    1,
                    "initialize",
                    json!({ "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {} }),
                ))
                .unwrap(),
            )
            .expect("initialize must answer");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(response["result"]["serverInfo"]["name"], SERVER_NAME);
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_exposes_the_five_tools() {
        let (server, _) = server();
        let response = server
            .handle_line(&serde_json::to_string(&request(2, "tools/list", json!({}))).unwrap())
            .expect("tools/list must answer");
        let names = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "sessions_list",
                "session_read_screen",
                "session_send_text",
                "session_wait_output",
                "session_focus"
            ]
        );
    }

    #[test]
    fn list_tool_returns_session_rows_with_interpreter_fields() {
        let (server, _) = server();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    3,
                    "tools/call",
                    json!({ "name": "sessions_list", "arguments": {} }),
                ))
                .unwrap(),
            )
            .expect("tools/call must answer");
        assert_eq!(response["result"]["isError"], false);
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let rows: Vec<Value> = serde_json::from_str(text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["session"], token());
        assert_eq!(rows[0]["tool"], "generic");
        assert!(rows[0]["capabilities"].as_array().unwrap().len() >= 3);
    }

    #[test]
    fn send_text_tool_delivers_with_submit_mode() {
        let (server, backend) = server();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    4,
                    "tools/call",
                    json!({
                        "name": "session_send_text",
                        "arguments": { "session": token(), "text": "print(6*7)", "submit": true }
                    }),
                ))
                .unwrap(),
            )
            .expect("tools/call must answer");
        assert_eq!(response["result"]["isError"], false);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("submitted"));
        server.queue.drain_manual();
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "print(6*7)");
        assert_eq!(sent[0].2, DeliveryMode::Submit);
    }

    #[test]
    fn send_text_tool_queues_in_fifo_order() {
        let (server, backend) = server();
        let call = |text: &str| {
            server
                .handle_line(
                    &serde_json::to_string(&request(
                        10,
                        "tools/call",
                        json!({
                            "name": "session_send_text",
                            "arguments": { "session": token(), "text": text, "submit": true }
                        }),
                    ))
                    .unwrap(),
                )
                .unwrap()
        };
        assert!(call("first")["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("queued at 1"));
        assert!(call("second")["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("queued at 2"));
        assert_eq!(
            server
                .queue
                .pending(&SessionBinding::from_str(&token()).unwrap()),
            2
        );
        assert!(backend.sent.lock().unwrap().is_empty());
        server.queue.drain_manual();
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].1, "first");
        assert_eq!(sent[1].1, "second");
    }

    #[test]
    fn delivery_queue_worker_delivers_in_background_in_order() {
        let backend = Arc::new(FakeBackend::with_screens(Vec::new()));
        let service =
            TerminalSessionService::from_backends([backend.clone() as Arc<dyn TerminalBackend>])
                .expect("unique fake backend");
        let queue = DeliveryQueue::new(Arc::new(service), true);
        let binding = FakeBackend::session().binding().unwrap();
        queue
            .enqueue(binding.clone(), "first".to_owned(), DeliveryMode::Submit)
            .unwrap();
        queue
            .enqueue(binding, "second".to_owned(), DeliveryMode::Submit)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let sent = backend.sent.lock().unwrap();
            if sent.len() == 2 {
                assert_eq!(sent[0].1, "first");
                assert_eq!(sent[1].1, "second");
                break;
            }
            drop(sent);
            assert!(Instant::now() < deadline, "worker never delivered");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn send_text_tool_rejects_when_queue_is_full() {
        let (server, _) = server();
        for index in 0..QUEUE_CAPACITY_PER_SESSION {
            let response = server
                .handle_line(
                    &serde_json::to_string(&request(
                        20,
                        "tools/call",
                        json!({
                            "name": "session_send_text",
                            "arguments": { "session": token(), "text": format!("item {index}") }
                        }),
                    ))
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(response["result"]["isError"], false);
        }
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    21,
                    "tools/call",
                    json!({
                        "name": "session_send_text",
                        "arguments": { "session": token(), "text": "overflow" }
                    }),
                ))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("queue full"));
    }

    #[test]
    fn wait_output_returns_when_expect_pattern_appears() {
        let (server, _) = server_with_screens(vec![
            ">>> idle".to_owned(),
            ">>> print(6*7)".to_owned(),
            ">>> print(6*7)\n42\n>>>".to_owned(),
        ]);
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    30,
                    "tools/call",
                    json!({
                        "name": "session_wait_output",
                        "arguments": { "session": token(), "expect": "42" }
                    }),
                ))
                .unwrap(),
            )
            .unwrap();
        let text = response["result"]["content"][0]["text"].as_str().unwrap();
        let outcome: Value = serde_json::from_str(text).unwrap();
        assert_eq!(outcome["settled"], true);
        assert!(outcome["screen"].as_str().unwrap().contains("42"));
        assert!(outcome["polls"].as_u64().unwrap() >= 3);
    }

    #[test]
    fn wait_output_returns_settled_false_on_timeout() {
        let (server, _) = server_with_screens(vec![">>> idle".to_owned()]);
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    31,
                    "tools/call",
                    json!({
                        "name": "session_wait_output",
                        "arguments": { "session": token(), "expect": "NEVER", "timeout_ms": 1100 }
                    }),
                ))
                .unwrap(),
            )
            .unwrap();
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["settled"], false);
        assert!(outcome["elapsed_ms"].as_u64().unwrap() >= 1000);
    }

    #[test]
    fn wait_output_settles_after_change_then_stable_screen() {
        let (server, _) = server_with_screens(vec![
            ">>> idle".to_owned(),
            ">>> running".to_owned(),
            ">>> done".to_owned(),
            ">>> done".to_owned(),
        ]);
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    32,
                    "tools/call",
                    json!({
                        "name": "session_wait_output",
                        "arguments": { "session": token(), "timeout_ms": 5000 }
                    }),
                ))
                .unwrap(),
            )
            .unwrap();
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["settled"], true);
        assert_eq!(outcome["screen"], ">>> done");
    }

    #[test]
    fn wait_output_ignores_the_echo_of_the_last_sent_text_until_real_output_lands() {
        let echo = "$ sleep 4; echo relay-slow-ok";
        let (server, _) = server_with_screens(vec![
            echo.to_owned(),
            echo.to_owned(),
            format!("{echo}\nrelay-slow-ok\n$"),
            format!("{echo}\nrelay-slow-ok\n$"),
        ]);
        let send = server
            .handle_line(
                &serde_json::to_string(&request(
                    40,
                    "tools/call",
                    json!({
                        "name": "session_send_text",
                        "arguments": { "session": token(), "text": "sleep 4; echo relay-slow-ok", "submit": true }
                    }),
                ))
                .unwrap(),
            )
            .expect("send must answer");
        assert_eq!(send["result"]["isError"], false);
        server.queue.drain_manual();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    41,
                    "tools/call",
                    json!({
                        "name": "session_wait_output",
                        "arguments": { "session": token(), "expect": "relay-slow-ok", "timeout_ms": 5000 }
                    }),
                ))
                .unwrap(),
            )
            .expect("wait must answer");
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["settled"], true);
        assert!(outcome["polls"].as_u64().unwrap() >= 3);
        assert!(outcome["screen"]
            .as_str()
            .unwrap()
            .contains("relay-slow-ok"));
    }

    #[test]
    fn wait_output_with_expect_confirms_stability_before_returning() {
        let (server, _) = server_with_screens(vec![
            ">>> idle".to_owned(),
            ">>> idle\nrelay-slow-ok".to_owned(),
            ">>> idle\nrelay-slow-ok".to_owned(),
        ]);
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    42,
                    "tools/call",
                    json!({
                        "name": "session_wait_output",
                        "arguments": { "session": token(), "expect": "relay-slow-ok", "timeout_ms": 5000 }
                    }),
                ))
                .unwrap(),
            )
            .expect("wait must answer");
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["settled"], true);
        assert_eq!(outcome["polls"], 3);
        assert!(outcome["screen"]
            .as_str()
            .unwrap()
            .contains("relay-slow-ok"));
    }

    #[test]
    fn wait_output_does_not_settle_on_the_echo_alone() {
        let echo = "$ echo relay-slow-ok";
        let (server, _) = server_with_screens(vec![echo.to_owned(), echo.to_owned()]);
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    43,
                    "tools/call",
                    json!({
                        "name": "session_send_text",
                        "arguments": { "session": token(), "text": "echo relay-slow-ok", "submit": true }
                    }),
                ))
                .unwrap(),
            )
            .expect("send must answer");
        assert_eq!(response["result"]["isError"], false);
        server.queue.drain_manual();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    44,
                    "tools/call",
                    json!({
                        "name": "session_wait_output",
                        "arguments": { "session": token(), "expect": "relay-slow-ok", "timeout_ms": 1100 }
                    }),
                ))
                .unwrap(),
            )
            .expect("wait must answer");
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["settled"], false);
    }

    #[test]
    fn wait_output_matches_a_pattern_that_was_already_on_screen() {
        let (server, _) = server_with_screens(vec![
            ">>> relay-slow-ok already visible".to_owned(),
            ">>> relay-slow-ok already visible".to_owned(),
        ]);
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    45,
                    "tools/call",
                    json!({
                        "name": "session_wait_output",
                        "arguments": { "session": token(), "expect": "relay-slow-ok", "timeout_ms": 5000 }
                    }),
                ))
                .unwrap(),
            )
            .expect("wait must answer");
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["settled"], true);
        assert_eq!(outcome["polls"], 2);
    }

    #[test]
    fn read_screen_tool_returns_backend_text() {
        let (server, _) = server();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    5,
                    "tools/call",
                    json!({ "name": "session_read_screen", "arguments": { "session": token() } }),
                ))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(response["result"]["content"][0]["text"], ">>> ready");
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn invalid_session_token_is_a_tool_error_not_a_protocol_error() {
        let (server, _) = server();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    6,
                    "tools/call",
                    json!({
                        "name": "session_send_text",
                        "arguments": { "session": "nope", "text": "x" }
                    }),
                ))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid session token"));
    }

    #[test]
    fn unknown_tool_is_invalid_params() {
        let (server, _) = server();
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    7,
                    "tools/call",
                    json!({ "name": "explode", "arguments": {} }),
                ))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(response["error"]["code"], ERROR_INVALID_PARAMS);
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let (server, _) = server();
        let response = server
            .handle_line(&serde_json::to_string(&request(8, "resources/list", json!({}))).unwrap())
            .unwrap();
        assert_eq!(response["error"]["code"], ERROR_METHOD_NOT_FOUND);
    }

    #[test]
    fn notifications_get_no_response() {
        let (server, _) = server();
        assert!(server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#)
            .is_none());
        assert!(server.handle_line("").is_none());
    }

    #[test]
    fn malformed_json_is_a_parse_error() {
        let (server, _) = server();
        let response = server.handle_line("{not json").unwrap();
        assert_eq!(response["error"]["code"], ERROR_PARSE);
        assert_eq!(response["id"], Value::Null);
    }

    #[test]
    fn ping_answers_empty() {
        let (server, _) = server();
        let response = server
            .handle_line(&serde_json::to_string(&request(9, "ping", json!({}))).unwrap())
            .unwrap();
        assert_eq!(response["result"], json!({}));
    }
}
