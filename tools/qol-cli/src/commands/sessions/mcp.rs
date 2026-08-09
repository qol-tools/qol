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

#[cfg(test)]
const TEST_ROUND_TIMEOUT: Duration = Duration::from_millis(1_000);

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub(super) const WAIT_TIMEOUT_MIN_MS: u64 = 1_000;
pub(super) const WAIT_TIMEOUT_DEFAULT_MS: u64 = 30_000;
pub(super) const WAIT_TIMEOUT_MAX_MS: u64 = 600_000;

pub(crate) struct McpSessionServer {
    terminals: Arc<TerminalSessionService>,
    interpreter: CliSessionInterpreter,
    pending: super::bridge::PendingBridgeStore,
    locks: super::spawn::SpawnLocks,
    round_timeout: Duration,
    #[cfg(test)]
    _pending_root: Option<tempfile::TempDir>,
}

impl McpSessionServer {
    pub(crate) fn system() -> Result<Self> {
        Ok(Self {
            terminals: Arc::new(TerminalSessionService::system()),
            interpreter: CliSessionInterpreter::system(),
            pending: super::bridge::PendingBridgeStore::system()?,
            locks: super::spawn::SpawnLocks::system()?,
            round_timeout: Duration::from_millis(super::bridge::TIMEOUT_MAX_MS),
            #[cfg(test)]
            _pending_root: None,
        })
    }

    #[cfg(test)]
    fn with(terminals: TerminalSessionService, interpreter: CliSessionInterpreter) -> Self {
        let root = tempfile::TempDir::new().unwrap();
        let pending = super::bridge::PendingBridgeStore::with_dir(root.path().to_path_buf());
        Self {
            terminals: Arc::new(terminals),
            interpreter,
            pending,
            locks: super::spawn::SpawnLocks::with_dir(root.path().join("spawn-locks")),
            round_timeout: TEST_ROUND_TIMEOUT,
            _pending_root: Some(root),
        }
    }

    #[cfg(test)]
    fn with_pending_dir(
        terminals: TerminalSessionService,
        interpreter: CliSessionInterpreter,
        dir: std::path::PathBuf,
    ) -> Self {
        Self {
            terminals: Arc::new(terminals),
            interpreter,
            pending: super::bridge::PendingBridgeStore::with_dir(dir.clone()),
            locks: super::spawn::SpawnLocks::with_dir(dir.join("spawn-locks")),
            round_timeout: TEST_ROUND_TIMEOUT,
            _pending_root: None,
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
            "session_spawn" => self.tool_spawn(arguments),
            "session_bridge" => self.tool_bridge(arguments),
            "session_loop_close" => self.close_loop(arguments),
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

    fn tool_spawn(&self, arguments: Value) -> Result<String, String> {
        let tool = string_argument(&arguments, "tool")?;
        let cwd = string_argument(&arguments, "cwd")?;
        let key = string_argument(&arguments, "key")?;
        let surface = arguments
            .get("surface")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "session_spawn `surface` must be a string".to_owned())
            })
            .transpose()?;
        let outcome = super::spawn::spawn_or_reuse(
            self.terminals.as_ref(),
            &self.interpreter,
            tool,
            cwd,
            Some(key),
            surface.as_deref(),
            super::spawn::config_surface().map_err(|error| error.to_string())?,
            &self.locks,
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_bridge(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        let task = arguments
            .get("task")
            .and_then(Value::as_str)
            .ok_or_else(|| "session_bridge requires a `task` string".to_owned())?;
        if arguments.get("timeout_ms").is_some() {
            return Err("session_bridge takes no `timeout_ms`; the round stays open until the implementation emits its completion signal. Resend this call without that argument.".to_owned());
        }
        let acknowledge_marker = arguments
            .get("acknowledge_marker")
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    "session_bridge `acknowledge_marker` must be a string".to_owned()
                })
            })
            .transpose()?;
        let outcome = super::bridge::execute(
            self.terminals.as_ref(),
            &self.interpreter,
            &binding,
            task,
            self.round_timeout,
            &self.pending,
            acknowledge_marker,
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }

    fn close_loop(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        let completion_marker = string_argument(&arguments, "completion_marker")?;
        let outcome = string_argument(&arguments, "outcome")?;
        if !matches!(outcome, "accepted" | "paused") {
            return Err("session_loop_close `outcome` must be `accepted` or `paused`".to_owned());
        }
        let receipt = render_close_receipt(&arguments, outcome)?;
        self.pending
            .acknowledge(&binding, completion_marker, outcome == "accepted")
            .map_err(|error| error.to_string())?;
        Ok(receipt)
    }
}

fn render_close_receipt(arguments: &Value, outcome: &str) -> Result<String, String> {
    let landed = non_empty_argument(arguments, "landed")?;
    let before = non_empty_argument(arguments, "before")?;
    let now = non_empty_argument(arguments, "now")?;
    let verification = non_empty_argument(arguments, "verification")?;
    let remaining = non_empty_argument(arguments, "remaining")?;
    let final_report = format!(
        "## What landed\n\n{landed}\n\n## Before\n\n{before}\n\n## Now\n\n{now}\n\n## Verification\n\n{verification}\n\n## Remaining\n\n{remaining}"
    );
    serde_json::to_string(&json!({
        "loop_closed": true,
        "outcome": outcome,
        "final_report": final_report,
    }))
    .map_err(|error| format!("serialization failed: {error}"))
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
    let server = McpSessionServer::system()?;
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
    "qol sessions mcp\n\nRun the sessions Model Context Protocol server over stdio.\n\nUsage:\n  qol sessions mcp\n  qol sessions mcp --help\n  qol sessions mcp help\n\nTools:\n  sessions_list, session_spawn, session_bridge, session_loop_close\n\nProtocol:\n  One JSON-RPC 2.0 message per line (protocol 2025-03-26). session_spawn\n  launches a tagged harness for a registered tool or reuses the single live\n  session already carrying the key, returning the live session facts.\n  session_bridge submits once and waits for the implementation terminal's\n  generated completion signal before returning. A reviewed completion marker\n  explicitly acknowledges the prior response before another task can be\n  submitted. session_loop_close acknowledges the final response and records\n  the architect's explicit accepted or paused terminal transition.\n\nExit:\n  Exits zero on EOF.\n"
}

fn string_argument<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing `{name}` string argument"))
}

fn non_empty_argument<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    let value = string_argument(arguments, name)?.trim();
    if value.is_empty() {
        return Err(format!("session_loop_close `{name}` must not be empty"));
    }
    Ok(value)
}

fn binding_argument(arguments: &Value, name: &str) -> Result<SessionBinding, String> {
    let value = string_argument(arguments, name)?;
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex;

    struct FakeBackend {
        id: BackendId,
        sent: Mutex<Vec<(SessionBinding, String, DeliveryMode)>>,
        screens: Mutex<VecDeque<String>>,
        last: Mutex<Option<String>>,
        complete_bridge: AtomicBool,
        fail_send: bool,
        current: Mutex<bool>,
        spawner_enabled: AtomicBool,
        spawn_count: std::sync::atomic::AtomicUsize,
        spawn_identity: Mutex<Option<qol_terminal_sessions::SpawnIdentity>>,
        spawn_cwd: Mutex<Option<std::path::PathBuf>>,
    }

    impl FakeBackend {
        fn new(screens: Vec<String>, complete_bridge: bool, fail_send: bool) -> Self {
            Self {
                id: BackendId::new("fake").unwrap(),
                sent: Mutex::new(Vec::new()),
                screens: Mutex::new(screens.into()),
                last: Mutex::new(None),
                complete_bridge: AtomicBool::new(complete_bridge),
                fail_send,
                current: Mutex::new(false),
                spawner_enabled: AtomicBool::new(false),
                spawn_count: std::sync::atomic::AtomicUsize::new(0),
                spawn_identity: Mutex::new(None),
                spawn_cwd: Mutex::new(None),
            }
        }

        fn with_id(mut self, id: BackendId) -> Self {
            self.id = id;
            self
        }

        fn enable_spawner(&self) {
            self.spawner_enabled.store(true, Ordering::Relaxed);
        }

        fn spawned_session(&self) -> Option<SessionFacts> {
            let identity = self.spawn_identity.lock().unwrap().clone()?;
            let cwd = self.spawn_cwd.lock().unwrap().clone()?;
            Some(SessionFacts {
                id: SessionId::new(self.id.clone(), format!("spawn-{}", identity.key)).unwrap(),
                root_pid: 200,
                cwd: cwd.display().to_string(),
                title: "Spawned".to_owned(),
                at_prompt: true,
                reported_cmd: None,
                foreground_basenames: vec![identity.tool.to_string()],
                foreground_pids: Vec::new(),
                capabilities: SessionCapabilities::ALL,
                spawn_identity: Some(identity),
            })
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
                spawn_identity: None,
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
            let mut facts = vec![Self::session()];
            if let Some(spawned) = self.spawned_session() {
                facts.push(spawned);
            }
            Ok(facts)
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            if let Some(screen) = self.screens.lock().unwrap().pop_front() {
                *self.last.lock().unwrap() = Some(screen.clone());
                return Ok(screen);
            }
            if self.complete_bridge.load(Ordering::Relaxed) {
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
            self.screens
                .lock()
                .unwrap()
                .push_back(format!(">>> {text}"));
            Ok(())
        }

        fn send_key(&self, target: &SessionBinding, key: &str) -> Result<(), TerminalError> {
            if self.fail_send {
                return Err(TerminalError::TargetMissing(target.clone()));
            }
            self.sent.lock().unwrap().push((
                target.clone(),
                format!("key:{key}"),
                DeliveryMode::Insert,
            ));
            Ok(())
        }
    }

    impl TerminalBackend for FakeBackend {
        fn id(&self) -> &BackendId {
            &self.id
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

        fn spawner(&self) -> Option<&dyn qol_terminal_sessions::SessionSpawner> {
            self.spawner_enabled.load(Ordering::Relaxed).then_some(self)
        }
    }

    impl qol_terminal_sessions::SessionSpawner for FakeBackend {
        fn supports(&self, surface: qol_terminal_sessions::SpawnSurface) -> bool {
            matches!(surface, qol_terminal_sessions::SpawnSurface::Tab)
        }

        fn spawn(
            &self,
            request: &qol_terminal_sessions::SpawnRequest,
        ) -> Result<SessionId, TerminalError> {
            self.spawn_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            *self.spawn_identity.lock().unwrap() = Some(request.identity.clone());
            *self.spawn_cwd.lock().unwrap() = Some(request.cwd.clone());
            SessionId::new(self.id.clone(), format!("spawn-{}", request.identity.key)).map_err(
                |error| TerminalError::SpawnFailed {
                    backend: self.id.clone(),
                    message: error.to_string(),
                },
            )
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

    fn server_with_backend(
        backend: Arc<FakeBackend>,
        pending_dir: std::path::PathBuf,
    ) -> McpSessionServer {
        let terminals =
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>]).unwrap();
        McpSessionServer::with_pending_dir(terminals, CliSessionInterpreter::system(), pending_dir)
    }

    fn token() -> String {
        FakeBackend::session().binding().unwrap().token()
    }

    fn close_arguments(outcome: &str) -> Value {
        json!({
            "session": token(),
            "completion_marker": "QOL_BRIDGE_DONE_final",
            "outcome": outcome,
            "landed": "The feature landed.",
            "before": "The loop stopped at round boundaries.",
            "now": "The loop continues through acceptance.",
            "verification": "Focused tests pass.",
            "remaining": "None.",
        })
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
    fn an_idle_target_without_the_marker_returns_stalled_instead_of_blocking() {
        use std::str::FromStr;
        let backend = Arc::new(FakeBackend::new(
            vec!["working prompt".to_owned(); 64],
            false,
            false,
        ));
        let terminals =
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>]).unwrap();
        let binding = qol_terminal_sessions::SessionBinding::from_str(&token()).unwrap();
        let (_tx, rx) = std::sync::mpsc::sync_channel(1);
        let outcome = super::super::bridge::wait_for_completion(
            &terminals,
            &binding,
            "QOL_BRIDGE_DONE_never",
            Duration::from_secs(30),
            rx,
            false,
            true,
            &|| Some(false),
            Duration::from_millis(50),
        )
        .unwrap();
        assert!(!outcome.completed);
        assert!(outcome.stalled);
        assert!(outcome.elapsed_ms < 10_000);

        let (_tx, rx) = std::sync::mpsc::sync_channel(1);
        let backend = Arc::new(FakeBackend::new(vec!["busy".to_owned(); 8], false, false));
        let terminals =
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>]).unwrap();
        let outcome = super::super::bridge::wait_for_completion(
            &terminals,
            &binding,
            "QOL_BRIDGE_DONE_never",
            Duration::from_millis(1_500),
            rx,
            false,
            true,
            &|| Some(true),
            Duration::from_millis(50),
        )
        .unwrap();
        assert!(!outcome.completed);
        assert!(!outcome.stalled);
    }

    #[test]
    fn unobserved_delivery_is_reported_instead_of_trusted() {
        use std::str::FromStr;
        let binding = qol_terminal_sessions::SessionBinding::from_str(&token()).unwrap();
        let quiet = |screens: Vec<String>| {
            let backend = Arc::new(FakeBackend::new(screens, false, false));
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>]).unwrap()
        };
        let window = Duration::from_millis(100);

        let terminals = quiet(vec!["prompt".to_owned(); 16]);
        let pre = terminals.read_screen(&binding).unwrap();
        let observed = super::super::bridge::delivery_observed(
            &terminals,
            &binding,
            "QOL_BRIDGE_DONE_",
            &pre,
            &|| Some(false),
            window,
        )
        .unwrap();
        assert!(!observed);

        let terminals = quiet(vec![
            "prompt".to_owned(),
            "prompt [qol session bridge] QOL_BRIDGE_DONE_".to_owned(),
        ]);
        let pre = terminals.read_screen(&binding).unwrap();
        let observed = super::super::bridge::delivery_observed(
            &terminals,
            &binding,
            "QOL_BRIDGE_DONE_",
            &pre,
            &|| Some(false),
            window,
        )
        .unwrap();
        assert!(observed);

        let terminals = quiet(vec!["prompt".to_owned(); 2]);
        let pre = terminals.read_screen(&binding).unwrap();
        let observed = super::super::bridge::delivery_observed(
            &terminals,
            &binding,
            "QOL_BRIDGE_DONE_",
            &pre,
            &|| None,
            window,
        )
        .unwrap();
        assert!(!observed);

        let terminals = quiet(vec!["prompt".to_owned(); 2]);
        let pre = terminals.read_screen(&binding).unwrap();
        let observed = super::super::bridge::delivery_observed(
            &terminals,
            &binding,
            "QOL_BRIDGE_DONE_",
            &pre,
            &|| Some(true),
            window,
        )
        .unwrap();
        assert!(observed);
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
    fn tools_list_exposes_discovery_spawn_bridge_and_loop_closure() {
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
        assert_eq!(
            names,
            [
                "sessions_list",
                "session_spawn",
                "session_bridge",
                "session_loop_close"
            ]
        );
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
            json!({ "session": token(), "task": "keep working" }),
        );
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["completed"], false);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn next_bridge_recovers_a_late_response_before_submitting_new_work() {
        let root = tempfile::TempDir::new().unwrap();
        let backend = Arc::new(FakeBackend::new(Vec::new(), false, false));
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let first = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "task": "first task" }),
        );
        let first_outcome: Value =
            serde_json::from_str(first["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(first_outcome["completed"], false);
        assert_eq!(first_outcome["submitted"], true);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);
        drop(server);

        backend.complete_bridge.store(true, Ordering::Relaxed);
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let recovered = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "task": "second task" }),
        );
        let recovered_outcome: Value =
            serde_json::from_str(recovered["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(recovered_outcome["completed"], true);
        assert_eq!(recovered_outcome["submitted"], false);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);

        let mismatched = tool_call(
            &server,
            "session_bridge",
            json!({
                "session": token(),
                "task": "second task",
                "acknowledge_marker": "QOL_BRIDGE_DONE_wrong",
            }),
        );
        assert_eq!(mismatched["result"]["isError"], true);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);

        let second = tool_call(
            &server,
            "session_bridge",
            json!({
                "session": token(),
                "task": "second task",
                "acknowledge_marker": recovered_outcome["completion_marker"],
            }),
        );
        let second_outcome: Value =
            serde_json::from_str(second["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(second_outcome["completed"], true);
        assert_eq!(second_outcome["submitted"], true);
        assert_eq!(backend.sent.lock().unwrap().len(), 2);
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
    fn bridge_refuses_a_caller_supplied_round_deadline() {
        let (server, backend) = server(Vec::new(), true, false);
        for timeout in [json!(600_000), json!(-1), json!("soon")] {
            let response = tool_call(
                &server,
                "session_bridge",
                json!({ "session": token(), "task": "safe task", "timeout_ms": timeout }),
            );
            assert_eq!(response["result"]["isError"], true);
            assert!(response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("takes no `timeout_ms`"));
        }
        assert!(backend.sent.lock().unwrap().is_empty());

        let schema = super::super::contract::tool_specs()
            .iter()
            .find(|spec| spec.name == "session_bridge")
            .unwrap()
            .input_schema
            .clone();
        assert!(schema["properties"].get("timeout_ms").is_none());
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
    fn loop_close_returns_a_typed_terminal_receipt() {
        let (server, _) = server(Vec::new(), false, false);
        for outcome in ["accepted", "paused"] {
            let binding: SessionBinding = token().parse().unwrap();
            server
                .pending
                .start(&binding, "QOL_BRIDGE_DONE_final")
                .unwrap();
            server
                .pending
                .observe(&binding, "QOL_BRIDGE_DONE_final", true)
                .unwrap();
            let response = tool_call(&server, "session_loop_close", close_arguments(outcome));
            assert_eq!(response["result"]["isError"], false);
            let receipt: Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(receipt["loop_closed"], true);
            assert_eq!(receipt["outcome"], outcome);
            assert!(receipt["final_report"]
                .as_str()
                .unwrap()
                .contains("## What landed"));
            assert!(receipt["final_report"]
                .as_str()
                .unwrap()
                .contains("## Before"));
            assert!(receipt["final_report"].as_str().unwrap().contains("## Now"));
        }
    }

    #[test]
    fn loop_close_rejects_ambiguous_or_unexplained_outcomes() {
        let (server, _) = server(Vec::new(), false, false);
        let mut invalid_outcome = close_arguments("done");
        let mut empty_landed = close_arguments("accepted");
        empty_landed["landed"] = json!("  ");
        let missing_fields = json!({ "outcome": "paused" });
        for arguments in [invalid_outcome.take(), empty_landed, missing_fields] {
            let response = tool_call(&server, "session_loop_close", arguments);
            assert_eq!(response["result"]["isError"], true);
        }
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

    fn spawn_arguments(tool: &str, key: &str, surface: Option<&str>, cwd: &str) -> Value {
        let mut arguments = json!({
            "tool": tool,
            "cwd": cwd,
            "key": key,
        });
        if let Some(surface) = surface {
            arguments["surface"] = json!(surface);
        }
        arguments
    }

    fn spawn_cwd(root: &tempfile::TempDir) -> String {
        std::fs::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn session_spawn_launches_and_returns_actual_facts() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let response = tool_call(
            &server,
            "session_spawn",
            spawn_arguments("codex", "mcp-lane", None, &cwd),
        );
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["session"], "v1:kitty:spawn-mcp-lane:200");
        assert_eq!(outcome["tool"], "codex");
        assert_eq!(outcome["key"], "mcp-lane");
        assert_eq!(outcome["reused"], false);
        assert_eq!(outcome["cwd"], json!(cwd));
        assert_eq!(outcome["surface"], "tab");
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn session_spawn_reuses_the_live_match_and_never_launches_twice() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let first = tool_call(
            &server,
            "session_spawn",
            spawn_arguments("codex", "mcp-lane", None, &cwd),
        );
        assert_eq!(first["result"]["isError"], false);
        let second = tool_call(
            &server,
            "session_spawn",
            spawn_arguments("codex", "mcp-lane", None, "ignored-missing-cwd"),
        );
        assert_eq!(second["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(second["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(outcome["reused"], true);
        assert_eq!(outcome["session"], "v1:kitty:spawn-mcp-lane:200");
        assert_eq!(outcome["cwd"], json!(cwd));
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn session_spawn_same_key_with_a_different_tool_conflicts() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        tool_call(
            &server,
            "session_spawn",
            spawn_arguments("codex", "mcp-lane", None, &cwd),
        );
        let conflict = tool_call(
            &server,
            "session_spawn",
            spawn_arguments("claude", "mcp-lane", None, &cwd),
        );
        assert_eq!(conflict["result"]["isError"], true);
        assert!(conflict["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("already held by tool `codex`"));
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn session_spawn_requires_tool_cwd_and_key() {
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(
            backend.clone(),
            tempfile::TempDir::new().unwrap().path().to_path_buf(),
        );
        for (name, arguments) in [
            ("tool", json!({ "cwd": "/work", "key": "k" })),
            ("cwd", json!({ "tool": "codex", "key": "k" })),
            ("key", json!({ "tool": "codex", "cwd": "/work" })),
        ] {
            let response = tool_call(&server, "session_spawn", arguments);
            assert_eq!(response["result"]["isError"], true, "{name}");
            assert!(
                response["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("missing `{name}` string argument")),
                "{name}"
            );
        }
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn session_spawn_rejects_unknown_and_unsupported_surfaces() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let unknown = tool_call(
            &server,
            "session_spawn",
            spawn_arguments("codex", "mcp-lane", Some("floating"), &cwd),
        );
        assert_eq!(unknown["result"]["isError"], true);
        assert!(unknown["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid surface `floating`"));

        let unsupported = tool_call(
            &server,
            "session_spawn",
            spawn_arguments("codex", "mcp-lane", Some("os-window"), &cwd),
        );
        assert_eq!(unsupported["result"]["isError"], true);
        assert!(unsupported["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("refused the spawn request"));
    }

    #[test]
    fn session_spawn_rejects_unregistered_tools_as_orchestration_errors() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let response = tool_call(
            &server,
            "session_spawn",
            spawn_arguments("generic", "mcp-lane", None, &cwd),
        );
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no launch strategy"));
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }
}
