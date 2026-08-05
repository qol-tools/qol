use std::io::{self, BufRead, BufWriter, Write};

use anyhow::Result;
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    DeliveryMode, ScreenReader, SessionBinding, SessionCapabilities, SessionFocus,
    SessionInventory, TerminalSessionService, TextInput,
};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "qol-sessions-mcp";

const ERROR_PARSE: i64 = -32700;
const ERROR_INVALID_REQUEST: i64 = -32600;
const ERROR_METHOD_NOT_FOUND: i64 = -32601;
const ERROR_INVALID_PARAMS: i64 = -32602;

pub(crate) struct McpSessionServer {
    terminals: TerminalSessionService,
    interpreter: CliSessionInterpreter,
}

impl McpSessionServer {
    pub(crate) fn system() -> Self {
        Self {
            terminals: TerminalSessionService::system(),
            interpreter: CliSessionInterpreter::system(),
        }
    }

    #[cfg(test)]
    fn with(terminals: TerminalSessionService, interpreter: CliSessionInterpreter) -> Self {
        Self {
            terminals,
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
        self.terminals
            .send_text(&binding, text, mode)
            .map_err(|error| format!("delivery failed: {error}"))?;
        Ok(if submit {
            format!("delivered and submitted to {binding}")
        } else {
            format!("delivered to {binding}")
        })
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

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "sessions_list",
            "description": "List live terminal sessions with their tool, activity hint, and capabilities. Session tokens are stable identifiers accepted by the other session tools.",
            "inputSchema": { "type": "object", "properties": {} }
        }),
        json!({
            "name": "session_read_screen",
            "description": "Read the current screen text of a live terminal session.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string", "description": "Session token from sessions_list" } },
                "required": ["session"]
            }
        }),
        json!({
            "name": "session_send_text",
            "description": "Deliver text into a live terminal session's CLI. With submit=true an Enter keypress is appended so the CLI executes the text.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session": { "type": "string", "description": "Session token from sessions_list" },
                    "text": { "type": "string", "description": "Text to deliver" },
                    "submit": { "type": "boolean", "description": "Append Enter after the text (default false)" }
                },
                "required": ["session", "text"]
            }
        }),
        json!({
            "name": "session_focus",
            "description": "Raise the window of a live terminal session.",
            "inputSchema": {
                "type": "object",
                "properties": { "session": { "type": "string", "description": "Session token from sessions_list" } },
                "required": ["session"]
            }
        }),
    ]
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
    use std::sync::{Arc, Mutex};

    struct FakeBackend {
        sent: Mutex<Vec<(SessionBinding, String, DeliveryMode)>>,
    }

    impl FakeBackend {
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
            Ok(">>> ready".to_owned())
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
        let backend = Arc::new(FakeBackend {
            sent: Mutex::new(Vec::new()),
        });
        let service =
            TerminalSessionService::from_backends([backend.clone() as Arc<dyn TerminalBackend>])
                .expect("unique fake backend");
        (
            McpSessionServer::with(service, CliSessionInterpreter::system()),
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
    fn tools_list_exposes_the_four_tools() {
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
        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].1, "print(6*7)");
        assert_eq!(sent[0].2, DeliveryMode::Submit);
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
