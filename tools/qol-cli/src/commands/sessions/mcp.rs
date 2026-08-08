use std::io::{self, BufRead, BufWriter, Write};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    ScreenReader, SessionBinding, SessionInventory, TerminalSessionService,
};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "qol-sessions-mcp";

const ERROR_PARSE: i64 = -32700;
const ERROR_INVALID_REQUEST: i64 = -32600;
const ERROR_METHOD_NOT_FOUND: i64 = -32601;
const ERROR_INVALID_PARAMS: i64 = -32602;

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub(super) const WAIT_TIMEOUT_MIN_MS: u64 = 1_000;
pub(super) const WAIT_TIMEOUT_DEFAULT_MS: u64 = 30_000;
pub(super) const WAIT_TIMEOUT_MAX_MS: u64 = 600_000;

pub(crate) struct McpSessionServer {
    terminals: Arc<TerminalSessionService>,
    interpreter: CliSessionInterpreter,
}

impl McpSessionServer {
    pub(crate) fn system() -> Self {
        Self {
            terminals: Arc::new(TerminalSessionService::system()),
            interpreter: CliSessionInterpreter::system(),
        }
    }

    #[cfg(test)]
    fn with(terminals: TerminalSessionService, interpreter: CliSessionInterpreter) -> Self {
        Self {
            terminals: Arc::new(terminals),
            interpreter,
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
                ));
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
            "session_bridge" => self.tool_bridge(arguments),
            other => {
                return error(
                    Some(id),
                    ERROR_INVALID_PARAMS,
                    format!("unknown tool: {other}"),
                );
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
                Some(super::contract::session_row(session, &binding, &descriptor))
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&rows).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_bridge(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        let task = arguments
            .get("task")
            .and_then(Value::as_str)
            .ok_or_else(|| "session_bridge requires a `task` string".to_owned())?;
        let timeout_ms = match arguments.get("timeout_ms") {
            Some(value) => value.as_u64().ok_or_else(|| {
                "session_bridge `timeout_ms` must be a non-negative integer".to_owned()
            })?,
            None => super::bridge::TIMEOUT_DEFAULT_MS,
        }
        .clamp(super::bridge::TIMEOUT_MIN_MS, super::bridge::TIMEOUT_MAX_MS);
        let outcome = super::bridge::execute(
            self.terminals.as_ref(),
            &self.interpreter,
            &binding,
            task,
            Duration::from_millis(timeout_ms),
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }
}

pub(crate) fn run(args: &[std::ffi::OsString]) -> Result<()> {
    if args
        .first()
        .and_then(|argument| argument.to_str())
        .is_some_and(|argument| matches!(argument, "help" | "-h" | "--help"))
    {
        print!("{}", help_text());
        return Ok(());
    }
    if !args.is_empty() {
        bail!("usage: {}", help_text().trim_end());
    }
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

fn help_text() -> &'static str {
    "qol sessions mcp\n\nRun the sessions Model Context Protocol server over stdio.\n\nUsage:\n  qol sessions mcp\n  qol sessions mcp --help\n  qol sessions mcp help\n\nTools:\n  sessions_list, session_bridge\n\nProtocol:\n  One JSON-RPC 2.0 message per line (protocol 2025-03-26). session_bridge\n  submits once and waits for the implementation terminal's generated\n  completion signal before returning.\n\nExit:\n  Exits zero on EOF.\n"
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
        BackendId, DeliveryMode, SessionCapabilities, SessionFacts, SessionFocus, SessionId,
        TerminalBackend, TerminalError, TerminalSnapshot, TextInput,
    };
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeBackend {
        sent: Mutex<Vec<(SessionBinding, String, DeliveryMode)>>,
        screens: Mutex<VecDeque<String>>,
        last: Mutex<Option<String>>,
        complete_bridge: bool,
        fail_send: bool,
        current: Mutex<bool>,
    }

    impl FakeBackend {
        fn new(screens: Vec<String>, complete_bridge: bool, fail_send: bool) -> Self {
            Self {
                sent: Mutex::new(Vec::new()),
                screens: Mutex::new(screens.into()),
                last: Mutex::new(None),
                complete_bridge,
                fail_send,
                current: Mutex::new(false),
            }
        }

        fn session() -> SessionFacts {
            SessionFacts {
                id: SessionId::new(BackendId::new("fake").unwrap(), "7").unwrap(),
                root_pid: 123,
                cwd: "/work/demo".to_owned(),
                title: "Demo REPL".to_owned(),
                at_prompt: true,
                reported_cmd: Some("agent".to_owned()),
                foreground_basenames: Vec::new(),
                foreground_pids: Vec::new(),
                capabilities: SessionCapabilities::ALL,
            }
        }

        fn generated_completion(&self) -> Option<String> {
            let sent = self.sent.lock().unwrap();
            let prompt = &sent.last()?.1;
            let fragments = prompt
                .split('`')
                .enumerate()
                .filter_map(|(index, part)| (index % 2 == 1).then_some(part))
                .collect::<Vec<_>>();
            let right = fragments.last()?;
            let left = fragments.get(fragments.len().checked_sub(2)?)?;
            Some(format!("implementation complete\n{left}{right}"))
        }
    }

    impl SessionInventory for FakeBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            Ok(vec![Self::session()])
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            if let Some(screen) = self.screens.lock().unwrap().pop_front() {
                *self.last.lock().unwrap() = Some(screen.clone());
                return Ok(screen);
            }
            if self.complete_bridge {
                if let Some(screen) = self.generated_completion() {
                    *self.last.lock().unwrap() = Some(screen.clone());
                    return Ok(screen);
                }
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
            if self.fail_send {
                return Err(TerminalError::TargetMissing(target.clone()));
            }
            self.sent
                .lock()
                .unwrap()
                .push((target.clone(), text.to_owned(), mode));
            Ok(())
        }
    }

    impl TerminalBackend for FakeBackend {
        fn id(&self) -> &BackendId {
            static ID: std::sync::OnceLock<BackendId> = std::sync::OnceLock::new();
            ID.get_or_init(|| BackendId::new("fake").unwrap())
        }

        fn read_screen_from_snapshot(
            &self,
            _snapshot: &TerminalSnapshot,
            _target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Ok(">>> ready".to_owned())
        }

        fn current_session_id(&self) -> Option<SessionId> {
            (*self.current.lock().unwrap()).then(|| Self::session().id)
        }
    }

    fn server(
        screens: Vec<String>,
        complete_bridge: bool,
        fail_send: bool,
    ) -> (McpSessionServer, Arc<FakeBackend>) {
        let backend = Arc::new(FakeBackend::new(screens, complete_bridge, fail_send));
        let terminals =
            TerminalSessionService::from_backends([backend.clone() as Arc<dyn TerminalBackend>])
                .unwrap();
        (
            McpSessionServer::with(terminals, CliSessionInterpreter::system()),
            backend,
        )
    }

    fn token() -> String {
        FakeBackend::session().binding().unwrap().token()
    }

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn tool_call(server: &McpSessionServer, name: &str, arguments: Value) -> Value {
        server
            .handle_line(
                &serde_json::to_string(&request(
                    1,
                    "tools/call",
                    json!({ "name": name, "arguments": arguments }),
                ))
                .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn initialize_handshake_echoes_version_and_advertises_tools() {
        let (server, _) = server(Vec::new(), false, false);
        let response = server
            .handle_line(
                &serde_json::to_string(&request(
                    1,
                    "initialize",
                    json!({ "protocolVersion": "2025-03-26" }),
                ))
                .unwrap(),
            )
            .unwrap();
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(response["result"]["serverInfo"]["name"], SERVER_NAME);
    }

    #[test]
    fn tools_list_exposes_only_discovery_and_bridge() {
        let (server, _) = server(Vec::new(), false, false);
        let response = server
            .handle_line(&serde_json::to_string(&request(2, "tools/list", json!({}))).unwrap())
            .unwrap();
        let names = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, ["sessions_list", "session_bridge"]);
    }

    #[test]
    fn list_tool_returns_live_session_identity() {
        let (server, _) = server(Vec::new(), false, false);
        let response = tool_call(&server, "sessions_list", json!({}));
        let rows: Vec<Value> =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(rows[0]["session"], token());
        assert_eq!(rows[0]["tool"], "generic");
        assert_eq!(rows[0]["display_name"], "agent");
    }

    #[test]
    fn bridge_submits_once_and_waits_for_the_joined_completion_signal() {
        let (server, backend) = server(Vec::new(), true, false);
        let response = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "task": "implement and test the bounded change" }),
        );
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["completed"], true);
        assert!(outcome["screen"]
            .as_str()
            .unwrap()
            .contains("QOL_BRIDGE_DONE_"));

        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].2, DeliveryMode::Submit);
        let fragments = sent[0]
            .1
            .split('`')
            .enumerate()
            .filter_map(|(index, part)| (index % 2 == 1).then_some(part))
            .collect::<Vec<_>>();
        let joined = format!(
            "{}{}",
            fragments[fragments.len() - 2],
            fragments[fragments.len() - 1]
        );
        assert!(!sent[0].1.contains(&joined));
    }

    #[test]
    fn bridge_timeout_does_not_resubmit_the_task() {
        let (server, backend) = server(vec![">>> working".to_owned()], false, false);
        let response = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "task": "keep working", "timeout_ms": 1000 }),
        );
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["completed"], false);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn bridge_surfaces_validation_and_delivery_failures() {
        let (valid_server, _) = server(Vec::new(), false, false);
        let invalid = tool_call(
            &valid_server,
            "session_bridge",
            json!({ "session": token(), "task": "bad\u{1b}[31m" }),
        );
        assert_eq!(invalid["result"]["isError"], true);
        assert!(invalid["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("control characters"));

        let (server, _) = server(Vec::new(), false, true);
        let failed = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "task": "safe task" }),
        );
        assert_eq!(failed["result"]["isError"], true);
        assert!(failed["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("delivery failed"));
    }

    #[test]
    fn bridge_rejects_invalid_timeout_types() {
        let (server, backend) = server(Vec::new(), true, false);
        for timeout in [json!(-1), json!("soon"), json!(1.5)] {
            let response = tool_call(
                &server,
                "session_bridge",
                json!({ "session": token(), "task": "safe task", "timeout_ms": timeout }),
            );
            assert_eq!(response["result"]["isError"], true);
            assert!(response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("non-negative integer"));
        }
        assert!(backend.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn bridge_rejects_the_calling_terminal_before_delivery() {
        let (server, backend) = server(Vec::new(), true, false);
        *backend.current.lock().unwrap() = true;
        let response = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "task": "self deadlock" }),
        );
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("calling terminal"));
        assert!(backend.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn raw_wait_still_ignores_last_send_echo() {
        let echo = "$ echo relay-ok";
        let (server, _) = server(
            vec![
                echo.to_owned(),
                format!("{echo}\nrelay-ok"),
                format!("{echo}\nrelay-ok"),
            ],
            false,
            false,
        );
        let binding: SessionBinding = token().parse().unwrap();
        let outcome = poll_until_settled(
            server.terminals.as_ref(),
            &binding,
            Duration::from_secs(2),
            Some("relay-ok"),
            Some("echo relay-ok"),
        )
        .unwrap();
        assert!(outcome.0);
        assert!(outcome.1.contains("relay-ok"));
        assert!(outcome.2 >= 3);
    }

    #[test]
    fn protocol_errors_and_notifications_keep_their_shape() {
        let (server, _) = server(Vec::new(), false, false);
        assert_eq!(
            server.handle_line("{not json").unwrap()["error"]["code"],
            ERROR_PARSE
        );
        assert_eq!(
            tool_call(&server, "unknown", json!({}))["error"]["code"],
            ERROR_INVALID_PARAMS
        );
        let unknown = server
            .handle_line(&serde_json::to_string(&request(8, "resources/list", json!({}))).unwrap())
            .unwrap();
        assert_eq!(unknown["error"]["code"], ERROR_METHOD_NOT_FOUND);
        assert!(server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none());
    }

    #[test]
    fn run_rejects_unknown_arguments_instead_of_blocking() {
        let error = run(&[std::ffi::OsString::from("--port")]).unwrap_err();
        assert!(error.to_string().contains("usage"));
    }
}
