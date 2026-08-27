use std::collections::HashMap;
use std::io::{self, BufRead, BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{
    ScreenReader, SessionBinding, SessionInventory, SpawnSurface, TerminalSessionService,
};
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2025-03-26";
const SERVER_NAME: &str = "qol-sessions-mcp";

const ERROR_PARSE: i64 = -32700;
const ERROR_INVALID_REQUEST: i64 = -32600;
const ERROR_METHOD_NOT_FOUND: i64 = -32601;
const ERROR_INVALID_PARAMS: i64 = -32602;

#[cfg(test)]
const TEST_ROUND_TIMEOUT: Duration = Duration::from_millis(250);

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(250);
pub(super) const WAIT_TIMEOUT_MIN_MS: u64 = 1_000;
pub(super) const WAIT_TIMEOUT_DEFAULT_MS: u64 = 30_000;
pub(super) const WAIT_TIMEOUT_MAX_MS: u64 = 600_000;
const PROGRESS_NOTIFY_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) struct McpSessionServer {
    terminals: Arc<TerminalSessionService>,
    interpreter: CliSessionInterpreter,
    pending: super::bridge::PendingBridgeStore,
    locks: super::spawn::SpawnLocks,
    ledger: super::spawn::SpawnLedger,
    spawn_model: Option<String>,
    allowed_models: Vec<String>,
    spawn_surface: Option<SpawnSurface>,
    spawn_cap: Option<super::spawn::SpawnCapConfig>,
    round_timeout: Duration,
    watcher: super::watch_owner::ClientWatcher,
    reports_dir: std::path::PathBuf,
    forks: super::fork::ForkStore,
    #[cfg(test)]
    _pending_root: Option<tempfile::TempDir>,
}

impl McpSessionServer {
    pub(crate) fn system() -> Result<Self> {
        let terminals = Arc::new(TerminalSessionService::system());
        let watcher = super::watch_owner::ClientWatcher::for_terminal(&terminals);
        Ok(Self {
            terminals: Arc::clone(&terminals),
            interpreter: CliSessionInterpreter::system(),
            pending: super::bridge::PendingBridgeStore::system()?,
            locks: super::spawn::SpawnLocks::system()?,
            ledger: super::spawn::SpawnLedger::system()?,
            spawn_model: super::spawn::config_spawn_model()?,
            allowed_models: super::spawn::config_allowed_models()?,
            spawn_surface: super::spawn::config_surface()?,
            spawn_cap: super::spawn::resolve_spawn_cap(super::spawn::config_spawn_cap()?),
            round_timeout: Duration::from_millis(super::bridge::TIMEOUT_MAX_MS),
            watcher,
            reports_dir: qol_config::data_subdir("sessions")
                .unwrap_or_else(|| std::path::PathBuf::from(".")),
            forks: super::fork::ForkStore::system()?,
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
            ledger: super::spawn::SpawnLedger::with_dir(root.path().join("spawn-records")),
            spawn_model: None,
            allowed_models: Vec::new(),
            spawn_surface: None,
            spawn_cap: None,
            round_timeout: TEST_ROUND_TIMEOUT,
            watcher: super::watch_owner::ClientWatcher::with_dir(
                root.path().join("watch-state"),
                "test-owner".to_owned(),
            ),
            reports_dir: root.path().to_path_buf(),
            forks: super::fork::ForkStore::with_dir(root.path().join("forks")),
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
            ledger: super::spawn::SpawnLedger::with_dir(dir.join("spawn-records")),
            spawn_model: None,
            allowed_models: Vec::new(),
            spawn_surface: None,
            spawn_cap: None,
            round_timeout: TEST_ROUND_TIMEOUT,
            watcher: super::watch_owner::ClientWatcher::with_dir(
                dir.join("watch-state"),
                "test-owner".to_owned(),
            ),
            reports_dir: dir.clone(),
            forks: super::fork::ForkStore::with_dir(dir.join("forks")),
            _pending_root: None,
        }
    }

    #[cfg(test)]
    fn handle_line(&self, line: &str) -> Option<Value> {
        let message = match parse_json_line(line) {
            Ok(Some(message)) => message,
            Ok(None) => return None,
            Err(response) => return Some(response),
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
        self.call_tool_with_cancel(&id, &params, None)
    }

    fn call_tool_with_cancel(
        &self,
        id: &Value,
        params: &Value,
        cancel: Option<&AtomicBool>,
    ) -> Value {
        let name = params.get("name").and_then(Value::as_str);
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        let Some(name) = name else {
            return error(
                Some(id.clone()),
                ERROR_INVALID_PARAMS,
                "tools/call requires a tool name",
            );
        };
        let outcome = match name {
            "sessions_list" => self.tool_list_sessions(),
            "session_spawn" => self.tool_spawn(arguments, cancel),
            "session_fork" => self.tool_fork(arguments),
            "session_submit" => self.tool_submit(arguments),
            "session_bridge" => self.tool_bridge(arguments, cancel),
            "session_loop_close" => self.close_loop(arguments),
            "session_close" => self.tool_close(arguments),
            other => {
                return error(
                    Some(id.clone()),
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
            id.clone(),
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

    fn tool_spawn(&self, arguments: Value, cancel: Option<&AtomicBool>) -> Result<String, String> {
        if arguments.get("lanes").is_some() {
            return self.tool_spawn_lanes(&arguments, cancel);
        }
        let tool = string_argument(&arguments, "tool")?;
        let cwd = string_argument(&arguments, "cwd")?;
        let key = string_argument(&arguments, "key")?;
        if arguments.get("background").is_some() {
            return Err("session_spawn takes no `background`: background is the only mode and every spawn embeds its task in the launch. Drop that argument.".to_owned());
        }
        if arguments.get("autoclose").is_some() {
            return Err("`autoclose` was removed; lanes always close on completion".to_owned());
        }
        let surface = arguments
            .get("surface")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "session_spawn `surface` must be a string".to_owned())
            })
            .transpose()?;
        let model_flag = arguments
            .get("model")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "session_spawn `model` must be a string".to_owned())
            })
            .transpose()?;
        let title = arguments
            .get("title")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "session_spawn `title` must be a string".to_owned())
            })
            .transpose()?;
        let task = string_argument(&arguments, "task")?;
        let group = arguments
            .get("group")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "session_spawn `group` must be a string".to_owned())
            })
            .transpose()?;
        let resume = arguments
            .get("resume")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| "session_spawn `resume` must be a boolean".to_owned())
            })
            .transpose()?;
        let model = super::spawn::resolve_allowed_model_with(
            model_flag.as_deref(),
            self.spawn_model.clone(),
            &self.allowed_models,
        )
        .map_err(|error| error.to_string())?;
        let outcome = super::spawn::spawn_or_reuse(
            self.terminals.as_ref(),
            &self.interpreter,
            tool,
            cwd,
            Some(key),
            surface.as_deref(),
            model.as_deref(),
            title.as_deref(),
            self.spawn_surface,
            self.spawn_cap.as_ref(),
            &self.locks,
            &self.ledger,
            true,
            true,
            resume,
            Some(task),
            group.as_deref(),
            &self.pending,
            &self.reports_dir,
            cancel,
        )
        .map_err(|error| error.to_string())?;
        if outcome.task_submitted == Some(true) {
            self.watcher.record_token(&outcome.session, &self.pending);
        }
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_spawn_lanes(
        &self,
        arguments: &Value,
        cancel: Option<&AtomicBool>,
    ) -> Result<String, String> {
        let tool = string_argument(arguments, "tool")?;
        let cwd = string_argument(arguments, "cwd")?;
        for rejected in ["key", "task", "title"] {
            if arguments.get(rejected).is_some() {
                return Err(format!(
                    "`lanes` carries every lane's key, task and title, so a top-level `{rejected}` is ambiguous. Move it into the matching lane entry."
                ));
            }
        }
        let lanes = arguments
            .get("lanes")
            .expect("the caller checked that lanes is present");
        let lanes: Vec<super::spawn::LaneSpec> =
            serde_json::from_value(lanes.clone()).map_err(|error| {
                format!("`lanes` must be an array of {{key, task, title?}} objects: {error}")
            })?;
        let surface = optional_string(arguments, "surface", "session_spawn")?;
        let model_flag = optional_string(arguments, "model", "session_spawn")?;
        let group = optional_string(arguments, "group", "session_spawn")?;
        let resume = arguments
            .get("resume")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| "session_spawn `resume` must be a boolean".to_owned())
            })
            .transpose()?;
        let model = super::spawn::resolve_allowed_model_with(
            model_flag.as_deref(),
            self.spawn_model.clone(),
            &self.allowed_models,
        )
        .map_err(|error| error.to_string())?;
        let outcome = super::spawn::spawn_lanes(
            self.terminals.as_ref(),
            &self.interpreter,
            tool,
            cwd,
            &lanes,
            surface.as_deref(),
            model.as_deref(),
            self.spawn_surface,
            self.spawn_cap.as_ref(),
            &self.locks,
            &self.ledger,
            resume,
            group.as_deref(),
            &self.pending,
            &self.reports_dir,
            cancel,
        )
        .map_err(|error| error.to_string())?;
        for lane in &outcome.lanes {
            if lane.task_submitted == Some(true) {
                self.watcher.record_token(&lane.session, &self.pending);
            }
        }
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_fork(&self, arguments: Value) -> Result<String, String> {
        let cwd = string_argument(&arguments, "cwd")?;
        let key = string_argument(&arguments, "key")?;
        let model = string_argument(&arguments, "model")?;
        let brief = string_argument(&arguments, "brief")?;
        let tool = optional_string(&arguments, "tool", "session_fork")?;
        let effort = optional_string(&arguments, "effort", "session_fork")?;
        let title = optional_string(&arguments, "title", "session_fork")?;
        let surface = optional_string(&arguments, "surface", "session_fork")?;
        if arguments.get("task").is_some() {
            return Err("session_fork takes no `task`: a fork owns its brief end to end and never reports a round. Put the problem in `brief`.".to_owned());
        }
        if arguments.get("group").is_some() {
            return Err("session_fork takes no `group`: a fork is a new tree, not a member of the caller's grouped set".to_owned());
        }
        let parent = Some(super::bridge::driver_token(self.terminals.as_ref()))
            .filter(|token| !token.is_empty());
        let outcome = super::fork::fork(
            self.terminals.as_ref(),
            &self.interpreter,
            &self.ledger,
            &self.locks,
            &self.forks,
            tool.as_deref().unwrap_or("claude"),
            cwd,
            key,
            surface.as_deref(),
            model,
            effort.as_deref(),
            title.as_deref(),
            brief,
            parent.as_deref(),
            self.spawn_cap.as_ref(),
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_submit(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        let task = string_argument(&arguments, "task")?;
        let acknowledge_marker = arguments
            .get("acknowledge_marker")
            .map(|value| {
                value.as_str().ok_or_else(|| {
                    "session_submit `acknowledge_marker` must be a string".to_owned()
                })
            })
            .transpose()?;
        if arguments.get("autoclose").is_some() {
            return Err("`autoclose` was removed; lanes always close on completion".to_owned());
        }
        let resume = arguments
            .get("resume")
            .map(|value| {
                value
                    .as_bool()
                    .ok_or_else(|| "session_submit `resume` must be a boolean".to_owned())
            })
            .transpose()?
            .unwrap_or(false);
        let outcome = super::bridge::submit(
            self.terminals.as_ref(),
            &self.interpreter,
            &binding,
            task,
            &self.pending,
            acknowledge_marker,
            resume,
        )
        .map_err(|error| error.to_string())?;
        if outcome.submitted {
            self.watcher.record_token(&outcome.session, &self.pending);
        }
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_bridge(&self, arguments: Value, cancel: Option<&AtomicBool>) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        if arguments.get("task").is_some() {
            return Err("session_bridge takes no `task`: delivery belongs to session_spawn and session_submit, and bridge only collects the round after its wake. Resend this call without that argument.".to_owned());
        }
        if arguments.get("timeout_ms").is_some() {
            return Err("session_bridge takes no `timeout_ms`; the round stays open until the implementation emits its completion signal. Resend this call without that argument.".to_owned());
        }
        if arguments.get("acknowledge_marker").is_some() {
            return Err("session_bridge takes no `acknowledge_marker`; acknowledge the reviewed round on the next submit or the loop close".to_owned());
        }
        let outcome = super::bridge::resume(
            self.terminals.as_ref(),
            &self.interpreter,
            &binding,
            self.round_timeout,
            &self.pending,
            &self.ledger,
            &self.locks,
            &self.reports_dir,
            false,
            cancel,
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&outcome).map_err(|error| format!("serialization failed: {error}"))
    }

    fn tool_close(&self, arguments: Value) -> Result<String, String> {
        let binding = binding_argument(&arguments, "session")?;
        let outcome = super::close::execute(self.terminals.as_ref(), &self.pending, &binding)
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
        let command = LoopCloseCommand {
            binding: &binding,
            completion_marker,
            accepted: outcome == "accepted",
            narrative: LoopCloseNarrative::ToolArguments(&arguments),
        };
        let receipt = execute_loop_close(
            self.terminals.as_ref(),
            &self.pending,
            &self.reports_dir,
            &command,
        )
        .map_err(|error| error.to_string())?;
        serde_json::to_string(&receipt).map_err(|error| format!("serialization failed: {error}"))
    }
}

pub(super) struct LoopCloseCommand<'a> {
    pub(super) binding: &'a SessionBinding,
    pub(super) completion_marker: &'a str,
    pub(super) accepted: bool,
    pub(super) narrative: LoopCloseNarrative<'a>,
}

pub(super) enum LoopCloseNarrative<'a> {
    ToolArguments(&'a Value),
    Filled(&'a [String; 5]),
}

impl LoopCloseNarrative<'_> {
    fn fields(&self) -> Result<(&str, &str, &str, &str, &str), String> {
        match self {
            LoopCloseNarrative::ToolArguments(arguments) => Ok((
                non_empty_argument(arguments, "landed")?,
                non_empty_argument(arguments, "before")?,
                non_empty_argument(arguments, "now")?,
                non_empty_argument(arguments, "verification")?,
                non_empty_argument(arguments, "remaining")?,
            )),
            LoopCloseNarrative::Filled(fields) => {
                Ok((&fields[0], &fields[1], &fields[2], &fields[3], &fields[4]))
            }
        }
    }
}

pub(super) fn execute_loop_close(
    terminals: &TerminalSessionService,
    pending: &super::bridge::PendingBridgeStore,
    reports_dir: &std::path::Path,
    command: &LoopCloseCommand<'_>,
) -> Result<Value> {
    let initiator = pending
        .pending_round(command.binding)?
        .map(|round| round.driver)
        .filter(|driver| !driver.is_empty());
    let outcome = if command.accepted {
        "accepted"
    } else {
        "paused"
    };
    let fields = command
        .narrative
        .fields()
        .map_err(|error| anyhow!("{error}"))?;
    let mut receipt = render_close_receipt(fields, outcome);
    pending.acknowledge(command.binding, command.completion_marker, command.accepted)?;
    if command.accepted {
        let close = super::close::close_spawned_terminal(terminals, command.binding)?;
        receipt["terminal_closed"] = json!(close.closed);
        receipt["terminal_state"] = json!(close.terminal_state);
        if let Some(detail) = close.close_detail {
            receipt["close_detail"] = json!(detail);
        }
        if let Some(initiator) = initiator {
            let siblings =
                super::close::close_loop_siblings(terminals, pending, &initiator, command.binding)?;
            if !siblings.is_empty() {
                let labels = pending
                    .pending_rounds()?
                    .into_iter()
                    .map(|round| (round.session, round.label))
                    .collect::<std::collections::HashMap<_, _>>();
                let terse = tersify_sibling_lanes(&siblings, reports_dir, &labels)
                    .map_err(|error| anyhow!("{error}"))?;
                receipt["sibling_lanes"] = json!(terse);
            }
        }
    }
    Ok(receipt)
}

fn render_close_receipt(fields: (&str, &str, &str, &str, &str), outcome: &str) -> Value {
    let (landed, before, now, verification, remaining) = fields;
    let final_report = format!(
        "## What landed\n\n{landed}\n\n## Before\n\n{before}\n\n## Now\n\n{now}\n\n## Verification\n\n{verification}\n\n## Remaining\n\n{remaining}"
    );
    json!({
        "loop_closed": true,
        "outcome": outcome,
        "final_report": final_report,
    })
}

const MAX_SIBLING_LANES: usize = 50;
const MAX_SIBLING_FIELD_CHARS: usize = 200;
const TRUNCATE_MARKER: &str = "...";

fn tersify_sibling_lanes(
    siblings: &[super::close::SiblingLaneClose],
    reports_dir: &std::path::Path,
    labels: &std::collections::HashMap<String, Option<String>>,
) -> Result<Vec<Value>, String> {
    let total = siblings.len();
    let kept = siblings.len().min(MAX_SIBLING_LANES);
    let mut entries = Vec::with_capacity(kept);
    for sibling in &siblings[..kept] {
        let mut entry = json!({
            "session": sibling.session,
            "closed": sibling.closed,
            "terminal_state": sibling.terminal_state,
        });
        if let Some(key) = &sibling.key {
            entry["key"] = json!(cap_string(key));
        }
        if let Some(tool) = &sibling.tool {
            entry["tool"] = json!(cap_string(tool));
        }
        if let Some(detail) = &sibling.close_detail {
            entry["close_detail"] = json!(cap_string(detail));
        }
        if sibling.report.trim().is_empty() {
            entries.push(entry);
            continue;
        }
        let dir = reports_dir.join("sibling-reports");
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("failed to create sibling report dir {:?}: {error}", dir))?;
        let base = format!("sibling-report-{}.md", sibling.session);
        let slug = super::watch::label_slug(
            labels
                .get(&sibling.session)
                .and_then(|label| label.as_deref()),
        );
        let name = if slug.is_empty() {
            base
        } else {
            format!("{slug}_{base}")
        };
        let path = dir.join(name);
        std::fs::write(&path, &sibling.report)
            .map_err(|error| format!("failed to write sibling report {:?}: {error}", path))?;
        entry["report"] = json!(path.display().to_string());
        entry["report_file"] = json!(path.display().to_string());
        entries.push(entry);
    }
    if total > kept {
        entries.push(json!({
            "dropped_sibling_lanes": total - kept,
        }));
    }
    Ok(entries)
}

fn cap_string(value: &str) -> String {
    if value.chars().count() <= MAX_SIBLING_FIELD_CHARS {
        return value.to_owned();
    }
    let mut capped: String = value.chars().take(MAX_SIBLING_FIELD_CHARS).collect();
    capped.push_str(TRUNCATE_MARKER);
    capped
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
    let server = Arc::new(McpSessionServer::system()?);
    server.watcher.start(&server.pending);
    let stdout = io::stdout();
    let writer = Arc::new(Mutex::new(BufWriter::new(stdout)));
    let cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let server = Arc::clone(&server);
        let writer = Arc::clone(&writer);
        let cancellations = Arc::clone(&cancellations);
        let workers = Arc::clone(&workers);
        std::thread::spawn(move || -> Result<()> {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                dispatch_line(&server, &writer, &cancellations, &workers, &line)?;
            }
            let pending_flags: Vec<Arc<AtomicBool>> =
                cancellations.lock().unwrap().values().cloned().collect();
            for flag in pending_flags {
                flag.store(true, Ordering::SeqCst);
            }
            for worker in workers.lock().unwrap().drain(..) {
                let _ = worker.join();
            }
            Ok(())
        })
    };
    reader
        .join()
        .map_err(|_| anyhow::anyhow!("the sessions MCP reader thread panicked"))??;
    server.watcher.stop();
    Ok(())
}

fn dispatch_line<W: Write + Send + Sync + 'static>(
    server: &Arc<McpSessionServer>,
    writer: &Arc<Mutex<BufWriter<W>>>,
    cancellations: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    workers: &Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    line: &str,
) -> Result<()> {
    let message = match parse_json_line(line) {
        Ok(Some(message)) => message,
        Ok(None) => return Ok(()),
        Err(response) => {
            write_response(writer, &response)?;
            return Ok(());
        }
    };
    let id = message.get("id").cloned();
    let method = message.get("method").and_then(Value::as_str);
    if method == Some("notifications/cancelled") {
        if let Some(request_id) = message.pointer("/params/requestId").cloned() {
            let key = request_id_key(&request_id);
            if let Some(flag) = cancellations.lock().unwrap().get(&key) {
                flag.store(true, Ordering::SeqCst);
            }
        }
        if let Some(id) = id {
            write_response(writer, &result(id, json!({})))?;
        }
        return Ok(());
    }
    let Some(id) = id else {
        return Ok(());
    };
    let Some(method) = method else {
        write_response(
            writer,
            &error(
                Some(id),
                ERROR_INVALID_REQUEST,
                "invalid request: missing method",
            ),
        )?;
        return Ok(());
    };
    if method == "tools/call" {
        let flag = Arc::new(AtomicBool::new(false));
        cancellations
            .lock()
            .unwrap()
            .insert(request_id_key(&id), Arc::clone(&flag));
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let progress_token = params
            .get("_meta")
            .and_then(|meta| meta.get("progressToken"))
            .cloned();
        let server = Arc::clone(server);
        let writer = Arc::clone(writer);
        let cancellations = Arc::clone(cancellations);
        let worker = std::thread::spawn(move || {
            let done = Arc::new(AtomicBool::new(false));
            if let Some(token) = &progress_token {
                let writer = Arc::clone(&writer);
                let done = Arc::clone(&done);
                let token = token.clone();
                std::thread::spawn(move || loop {
                    std::thread::sleep(PROGRESS_NOTIFY_INTERVAL);
                    if done.load(Ordering::SeqCst) {
                        break;
                    }
                    let _ = write_response(
                        &writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/progress",
                            "params": {
                                "progressToken": token,
                                "progress": 0,
                                "total": 1,
                                "message": "round still open",
                            },
                        }),
                    );
                });
            }
            let response = server.call_tool_with_cancel(&id, &params, Some(&flag));
            done.store(true, Ordering::SeqCst);
            if let Err(error) = write_response(&writer, &response) {
                qol_runtime::probe!(
                    "CLI_SESSION_MCP",
                    "event=response_write_failed id={} error={}",
                    id,
                    error
                );
            }
            cancellations.lock().unwrap().remove(&request_id_key(&id));
        });
        workers.lock().unwrap().push(worker);
        return Ok(());
    }
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    write_response(writer, &server.handle(method, params, id))?;
    Ok(())
}

fn write_response<W: Write + Send + Sync + 'static>(
    writer: &Arc<Mutex<BufWriter<W>>>,
    response: &Value,
) -> io::Result<()> {
    let mut output = writer.lock().unwrap();
    serde_json::to_writer(&mut *output, response)?;
    output.write_all(b"\n")?;
    output.flush()
}

fn request_id_key(id: &Value) -> String {
    id.to_string()
}

fn parse_json_line(line: &str) -> Result<Option<Value>, Value> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str(line) {
        Ok(message) => Ok(Some(message)),
        Err(_) => Err(error(None, ERROR_PARSE, "parse error: invalid JSON")),
    }
}

fn help_text() -> String {
    format!("qol sessions mcp\n\nRun the sessions Model Context Protocol server over stdio.\n\nUsage:\n  qol sessions mcp\n  qol sessions mcp --help\n  qol sessions mcp help\n\nTools:\n  {tool_names}\n\nProtocol:\n  One JSON-RPC 2.0 message per line (protocol 2025-03-26). session_spawn\n  launches a tagged harness for a registered tool or reuses the single live\n  session already carrying the key, returning the live session facts. An\n  optional `title` names the new tab (the lane key by default), a `model`\n  argument is required when launching a new session (the sessions.toml\n  `spawn_model` entry is the fallback; the reuse path needs no model), and a\n  `task` is required: every spawn embeds its first round in the launch and\n  returns with the round already open (background delivery is the only mode,\n  so an explicit `background` is an error; lanes always close when the\n  watcher confirms completion and sessions without a spawn identity are never\n  closed; a `resume` argument forces a resume, which is otherwise automatic\n  when the spawn ledger holds a session id for the key (same tool and cwd),\n  `resume: false` opts out, and the outcome reports `resume` and\n  `resume_detail`). An optional `group` string registers the lane as a member\n  of a grouped-research set; when every member completes, its fragments are\n  concatenated under the sessions data dir and the initiator receives one\n  combined wake instead of one wake per lane.\n  session_submit delivers one bounded task without waiting and returns with\n  the round open; submitted rounds close the lane terminal when the watcher\n  confirms completion, and sessions without a spawn identity are never\n  closed. session_bridge takes no `task`: it only collects the round\n  a spawn or submit left open, waiting for the implementation terminal's\n  generated completion signal before returning. The\n  round envelope is generated server-side from the target's durable role record\n  (lane marker written at spawn; absent means architect): bridging a non-lane\n  session is an architect-receiver round - the receiver may accept the request\n  into its own loop or decline with a reason, and returns the completion\n  fragments either way. The caller never chooses the receiver's role. A\n  reviewed completion marker explicitly acknowledges the prior response\n  before another task can be submitted. session_loop_close accepted\n  acknowledges the final response, records the transition, and terminates\n  the implementation terminal; a paused close keeps the terminal open.\n  An accepted close also terminates the other completed sibling lanes of\n  the same loop; each completed sibling gets a terse entry in the\n  receipt's `sibling_lanes` field, and its final report is written to\n  a file under the sessions data dir whose path that entry carries.\n  session_close remains the standalone closer for spawned sessions.\n\nExit:\n  Exits zero on EOF.\n", tool_names = super::contract::tool_names())
}

fn string_argument<'a>(arguments: &'a Value, name: &str) -> Result<&'a str, String> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing `{name}` string argument"))
}

fn optional_string(arguments: &Value, name: &str, tool: &str) -> Result<Option<String>, String> {
    arguments
        .get(name)
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{tool} `{name}` must be a string"))
        })
        .transpose()
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
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return Ok((false, screen, polls, started));
        }
        std::thread::sleep(WAIT_POLL_INTERVAL.min(timeout - elapsed));
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
        gone: AtomicBool,
        fail_close: AtomicBool,
        spawner_enabled: AtomicBool,
        spawn_count: std::sync::atomic::AtomicUsize,
        spawn_identity: Mutex<Option<qol_terminal_sessions::SpawnIdentity>>,
        spawn_cwd: Mutex<Option<std::path::PathBuf>>,
        spawn_launch: Mutex<Option<qol_terminal_sessions::cli::CliLaunchProgram>>,
        current_facts: Mutex<Option<SessionFacts>>,
        lanes: Mutex<Vec<SessionFacts>>,
        closed: Mutex<Vec<SessionBinding>>,
        spawns: Mutex<
            Vec<(
                qol_terminal_sessions::SpawnIdentity,
                std::path::PathBuf,
                qol_terminal_sessions::cli::CliLaunchProgram,
            )>,
        >,
        lane_screens: Mutex<std::collections::HashMap<String, String>>,
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
                gone: AtomicBool::new(false),
                fail_close: AtomicBool::new(false),
                spawner_enabled: AtomicBool::new(false),
                spawn_count: std::sync::atomic::AtomicUsize::new(0),
                spawn_identity: Mutex::new(None),
                spawn_cwd: Mutex::new(None),
                spawn_launch: Mutex::new(None),
                current_facts: Mutex::new(None),
                lanes: Mutex::new(Vec::new()),
                closed: Mutex::new(Vec::new()),
                spawns: Mutex::new(Vec::new()),
                lane_screens: Mutex::new(std::collections::HashMap::new()),
            }
        }

        fn with_id(mut self, id: BackendId) -> Self {
            self.id = id;
            self
        }

        fn enable_spawner(&self) {
            self.spawner_enabled.store(true, Ordering::Relaxed);
        }

        fn mark_gone(&self) {
            self.gone.store(true, Ordering::Relaxed);
        }

        fn fail_closing(&self) {
            self.fail_close.store(true, Ordering::Relaxed);
        }

        fn add_lane(&self, facts: SessionFacts) {
            self.lanes.lock().unwrap().push(facts);
        }

        fn set_current(&self, facts: SessionFacts) {
            *self.current.lock().unwrap() = true;
            *self.current_facts.lock().unwrap() = Some(facts);
        }

        fn spawned_sessions(&self) -> Vec<SessionFacts> {
            self.spawns
                .lock()
                .unwrap()
                .iter()
                .map(|(identity, cwd, _)| SessionFacts {
                    id: SessionId::new(self.id.clone(), format!("spawn-{}", identity.key)).unwrap(),
                    root_pid: 200,
                    cwd: cwd.display().to_string(),
                    title: "Spawned".to_owned(),
                    at_prompt: true,
                    reported_cmd: None,
                    foreground_basenames: vec![identity.tool.to_string()],
                    foreground_pids: Vec::new(),
                    capabilities: SessionCapabilities::ALL,
                    spawn_identity: Some(identity.clone()),
                })
                .collect()
        }

        fn lane_marker(&self, key: &str) -> Option<String> {
            let spawns = self.spawns.lock().unwrap();
            let (_, _, launch) = spawns
                .iter()
                .find(|(identity, _, _)| identity.key.as_str() == key)?;
            marker_in_prompt(launch.args.last()?)
        }

        fn show_lane_screen(&self, key: &str, screen: String) {
            self.lane_screens
                .lock()
                .unwrap()
                .insert(format!("spawn-{key}"), screen);
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
            let prompt = self
                .sent
                .lock()
                .unwrap()
                .last()
                .map(|(_, text, _)| text.clone())
                .or_else(|| {
                    self.spawn_launch
                        .lock()
                        .unwrap()
                        .as_ref()
                        .and_then(|launch| launch.args.last().cloned())
                })?;
            Some(format!(
                "implementation complete\n{}",
                marker_in_prompt(&prompt)?
            ))
        }
    }

    fn marker_in_prompt(prompt: &str) -> Option<String> {
        let fragments = prompt
            .split('`')
            .enumerate()
            .filter_map(|(index, part)| (index % 2 == 1).then_some(part))
            .collect::<Vec<_>>();
        if fragments.len() >= 2 {
            let right = fragments.last()?;
            let left = fragments.get(fragments.len().checked_sub(2)?)?;
            return Some(format!("{left}{right}"));
        }
        prompt
            .lines()
            .rev()
            .find(|line| line.starts_with("QOL_BRIDGE_DONE_"))
            .map(str::to_owned)
    }

    impl SessionInventory for FakeBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            if self.gone.load(Ordering::Relaxed) {
                return Ok(Vec::new());
            }
            let mut facts = vec![Self::session()];
            if let Some(current) = self.current_facts.lock().unwrap().clone() {
                facts.push(current);
            }
            if let Some(spawned) = self.spawned_session() {
                facts.push(spawned);
            }
            for extra in self.spawned_sessions() {
                if !facts.iter().any(|known| known.id == extra.id) {
                    facts.push(extra);
                }
            }
            facts.extend(self.lanes.lock().unwrap().iter().cloned());
            Ok(facts)
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            if let Some(screen) = self
                .lane_screens
                .lock()
                .unwrap()
                .get(_target.session_id().native())
            {
                return Ok(screen.clone());
            }
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
            if !*self.current.lock().unwrap() {
                return None;
            }
            Some(
                self.current_facts
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(Self::session)
                    .id,
            )
        }

        fn spawner(&self) -> Option<&dyn qol_terminal_sessions::SessionSpawner> {
            self.spawner_enabled.load(Ordering::Relaxed).then_some(self)
        }

        fn closer(&self) -> Option<&dyn qol_terminal_sessions::SessionCloser> {
            Some(self)
        }
    }

    impl qol_terminal_sessions::SessionCloser for FakeBackend {
        fn close(&self, target: &SessionBinding) -> Result<(), TerminalError> {
            if self.fail_close.load(Ordering::Relaxed) {
                return Err(TerminalError::CommandFailed {
                    backend: self.id.clone(),
                    operation: "close",
                    code: Some(1),
                    stderr: "fake close refused".to_owned(),
                });
            }
            self.closed.lock().unwrap().push(target.clone());
            Ok(())
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
            *self.spawn_launch.lock().unwrap() = Some(request.launch.clone());
            self.spawns.lock().unwrap().push((
                request.identity.clone(),
                request.cwd.clone(),
                request.launch.clone(),
            ));
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
                .unwrap()
                .with_wait_backoff(Duration::from_millis(2), Duration::from_millis(4));
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
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>])
                .unwrap()
                .with_wait_backoff(Duration::from_millis(2), Duration::from_millis(4));
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

    fn lane_facts(key: &str, cwd: &str) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("kitty").unwrap(), format!("spawn-{key}")).unwrap(),
            root_pid: 200,
            cwd: cwd.to_owned(),
            title: "Spawned".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: Some(qol_terminal_sessions::SpawnIdentity {
                key: qol_terminal_sessions::SpawnKey::new(key).unwrap(),
                tool: qol_terminal_sessions::cli::CliToolId::new("codex").unwrap(),
                surface: qol_terminal_sessions::SpawnSurface::Tab,
            }),
        }
    }

    fn architect_facts(cwd: &str) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("kitty").unwrap(), "architect").unwrap(),
            root_pid: 900,
            cwd: cwd.to_owned(),
            title: "Architect".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        }
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
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>])
                .unwrap()
                .with_wait_backoff(Duration::from_millis(2), Duration::from_millis(4));
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
            super::super::bridge::Role::Architect,
            None,
        )
        .unwrap();
        assert!(!outcome.completed);
        assert!(outcome.stalled);
        assert!(outcome.elapsed_ms < 10_000);

        let (_tx, rx) = std::sync::mpsc::sync_channel(1);
        let backend = Arc::new(FakeBackend::new(vec!["busy".to_owned(); 8], false, false));
        let terminals =
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>])
                .unwrap()
                .with_wait_backoff(Duration::from_millis(2), Duration::from_millis(4));
        let outcome = super::super::bridge::wait_for_completion(
            &terminals,
            &binding,
            "QOL_BRIDGE_DONE_never",
            Duration::from_millis(200),
            rx,
            false,
            true,
            &|| Some(true),
            Duration::from_millis(50),
            super::super::bridge::Role::Architect,
            None,
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
            TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>])
                .unwrap()
                .with_wait_backoff(Duration::from_millis(2), Duration::from_millis(4))
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
                "session_fork",
                "session_submit",
                "session_bridge",
                "session_loop_close",
                "session_close"
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
    fn session_submit_rejects_an_explicit_autoclose_argument() {
        let (server, backend) = server(Vec::new(), false, false);
        for autoclose in [json!(true), json!(false), json!("yes")] {
            let response = tool_call(
                &server,
                "session_submit",
                json!({
                    "session": token(),
                    "task": "implement the bounded change",
                    "autoclose": autoclose,
                }),
            );
            assert_eq!(response["result"]["isError"], true);
            assert!(response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("`autoclose` was removed; lanes always close on completion"));
        }
        assert!(backend.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn session_submit_never_autocloses_a_session_without_a_spawn_identity() {
        let (server, _) = server(Vec::new(), false, false);
        let response = tool_call(
            &server,
            "session_submit",
            json!({ "session": token(), "task": "implement the bounded change" }),
        );
        assert_eq!(response["result"]["isError"], false);
        let binding: SessionBinding = token().parse().unwrap();
        let round = server.pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            !round.autoclose,
            "rounds without a spawn identity are never closable"
        );
    }

    #[test]
    fn session_submit_delivers_and_leaves_the_round_open() {
        let (server, backend) = server(Vec::new(), false, false);
        let response = tool_call(
            &server,
            "session_submit",
            json!({ "session": token(), "task": "implement the bounded change" }),
        );
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["submitted"], true);
        assert_eq!(outcome["completed"], false);
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        assert!(marker.starts_with("QOL_BRIDGE_DONE_"));

        let sent = backend.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].2, DeliveryMode::Submit);
        assert!(
            sent[0].1.starts_with("[qol session bridge to architect]"),
            "a session without a role record is an architect receiver"
        );
        drop(sent);

        let binding: SessionBinding = token().parse().unwrap();
        let round = server.pending.pending_round(&binding).unwrap();
        assert!(round.is_some());
        assert_eq!(round.unwrap().completion_marker, marker);
    }

    #[test]
    fn session_submit_records_the_round_token_in_the_watch_owner_state() {
        let root = tempfile::TempDir::new().unwrap();
        let backend = Arc::new(FakeBackend::new(Vec::new(), false, false));
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let response = tool_call(
            &server,
            "session_submit",
            json!({ "session": token(), "task": "implement the bounded change" }),
        );
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["submitted"], true);

        let watcher = crate::commands::sessions::watch_owner::ClientWatcher::with_dir(
            root.path().join("watch-state"),
            "test-owner".to_owned(),
        );
        assert_eq!(
            watcher.read_tokens(),
            vec![outcome["session"].as_str().unwrap().to_owned()],
            "a submitted round must arm the client watcher so the initiator wakes"
        );
    }

    #[test]
    fn session_submit_refuses_when_a_round_is_already_pending() {
        let (server, backend) = server(Vec::new(), false, false);
        let first = tool_call(
            &server,
            "session_submit",
            json!({ "session": token(), "task": "first task" }),
        );
        assert_eq!(first["result"]["isError"], false);
        let second = tool_call(
            &server,
            "session_submit",
            json!({ "session": token(), "task": "second task" }),
        );
        assert_eq!(second["result"]["isError"], true);
        assert!(second["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("already pending"));
        assert_eq!(backend.sent.lock().unwrap().len(), 1);
    }

    fn live_pi_screen() -> String {
        [
            "conversation output",
            "\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
            "/tmp",
            "$0.400 47.3%/1.0M (auto)",
        ]
        .join("\n")
    }

    #[test]
    fn spawn_carries_title_and_task_and_the_round_stays_open_for_bridge() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(vec![live_pi_screen()], true, false)
                .with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut arguments = spawn_arguments("codex", "lane-one", None, &cwd);
        arguments["model"] = json!("flash-x");
        arguments["title"] = json!("Lane One");
        arguments["task"] = json!("implement and test the bounded change");
        let response = tool_call(&server, "session_spawn", arguments);
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["reused"], false);
        assert_eq!(outcome["title"], "Lane One");
        assert_eq!(outcome["task_submitted"], true);
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        assert!(marker.starts_with("QOL_BRIDGE_DONE_"));
        assert_eq!(outcome["session"], "v1:kitty:spawn-lane-one:200");

        let request = backend.spawn_launch.lock().unwrap().clone().unwrap();
        assert_eq!(request.program, "codex");
        assert_eq!(backend.spawn_count.load(Ordering::Relaxed), 1);
        let binding: SessionBinding = outcome["session"].as_str().unwrap().parse().unwrap();
        let round = server.pending.pending_round(&binding).unwrap();
        assert!(round.is_some());

        let waited = tool_call(
            &server,
            "session_bridge",
            json!({ "session": outcome["session"] }),
        );
        assert_eq!(waited["result"]["isError"], false);
        let waited_outcome: Value =
            serde_json::from_str(waited["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(waited_outcome["completed"], true);
        assert_eq!(waited_outcome["completion_marker"], marker);
    }

    #[test]
    fn bridge_with_a_task_is_rejected() {
        let (server, backend) = server(Vec::new(), true, false);
        let response = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "task": "implement and test the bounded change" }),
        );
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("delivery belongs to session_spawn and session_submit"));
        assert_eq!(backend.sent.lock().unwrap().len(), 0);
    }

    #[test]
    fn bridge_waits_on_the_pending_round_without_delivering_again() {
        let (server, backend) = server(vec![">>> working".to_owned()], false, false);
        let submitted = tool_call(
            &server,
            "session_submit",
            json!({ "session": token(), "task": "keep working" }),
        );
        assert_eq!(submitted["result"]["isError"], false);
        let response = tool_call(&server, "session_bridge", json!({ "session": token() }));
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
            "session_submit",
            json!({ "session": token(), "task": "first task" }),
        );
        let first_outcome: Value =
            serde_json::from_str(first["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(first_outcome["submitted"], true);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);
        let first_wait = tool_call(&server, "session_bridge", json!({ "session": token() }));
        let first_wait_outcome: Value =
            serde_json::from_str(first_wait["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(first_wait_outcome["completed"], false);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);
        drop(server);

        backend.complete_bridge.store(true, Ordering::Relaxed);
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let recovered = tool_call(&server, "session_bridge", json!({ "session": token() }));
        let recovered_outcome: Value =
            serde_json::from_str(recovered["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(recovered_outcome["completed"], true);
        assert_eq!(recovered_outcome["submitted"], false);
        assert_eq!(backend.sent.lock().unwrap().len(), 1);

        let with_marker = tool_call(
            &server,
            "session_bridge",
            json!({ "session": token(), "acknowledge_marker": "QOL_BRIDGE_DONE_wrong" }),
        );
        assert_eq!(with_marker["result"]["isError"], true);
        assert!(with_marker["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("takes no `acknowledge_marker`"));
        assert_eq!(backend.sent.lock().unwrap().len(), 1);

        let second = tool_call(
            &server,
            "session_submit",
            json!({
                "session": token(),
                "task": "second task",
                "acknowledge_marker": recovered_outcome["completion_marker"],
            }),
        );
        assert_eq!(second["result"]["isError"], false);
        let second_submit: Value =
            serde_json::from_str(second["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(second_submit["submitted"], true);
        assert_eq!(backend.sent.lock().unwrap().len(), 2);

        let second_wait = tool_call(&server, "session_bridge", json!({ "session": token() }));
        let second_wait_outcome: Value = serde_json::from_str(
            second_wait["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(second_wait_outcome["completed"], true);
        assert_eq!(backend.sent.lock().unwrap().len(), 2);
    }

    #[test]
    fn submit_surfaces_validation_and_delivery_failures() {
        let (valid_server, _) = server(Vec::new(), false, false);
        let invalid = tool_call(
            &valid_server,
            "session_submit",
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
            "session_submit",
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
                json!({ "session": token(), "timeout_ms": timeout }),
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
    fn bridge_without_a_pending_round_is_rejected() {
        let (server, backend) = server(Vec::new(), true, false);
        let response = tool_call(&server, "session_bridge", json!({ "session": token() }));
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("no pending bridge exists"));
        assert!(backend.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn loop_close_returns_a_typed_terminal_receipt() {
        let (server, backend) = server(Vec::new(), false, false);
        let binding: SessionBinding = token().parse().unwrap();
        server
            .pending
            .start(&binding, "QOL_BRIDGE_DONE_final", "", false, None)
            .unwrap();
        server
            .pending
            .observe(&binding, "QOL_BRIDGE_DONE_final", true)
            .unwrap();
        let response = tool_call(&server, "session_loop_close", close_arguments("paused"));
        assert_eq!(response["result"]["isError"], false);
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(receipt["loop_closed"], true);
        assert_eq!(receipt["outcome"], "paused");
        assert!(receipt["final_report"]
            .as_str()
            .unwrap()
            .contains("## What landed"));
        assert!(receipt["final_report"]
            .as_str()
            .unwrap()
            .contains("## Before"));
        assert!(receipt["final_report"].as_str().unwrap().contains("## Now"));
        assert!(receipt.get("terminal_closed").is_none());
        assert!(receipt.get("terminal_state").is_none());
        assert!(receipt.get("close_detail").is_none());
        assert!(backend.closed.lock().unwrap().is_empty());

        server
            .pending
            .start(&binding, "QOL_BRIDGE_DONE_final", "", false, None)
            .unwrap();
        server
            .pending
            .observe(&binding, "QOL_BRIDGE_DONE_final", true)
            .unwrap();
        let response = tool_call(&server, "session_loop_close", close_arguments("accepted"));
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("was not spawned"));
        assert!(backend.closed.lock().unwrap().is_empty());
    }

    #[test]
    fn loop_close_accepted_terminates_a_spawned_implementation_terminal() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut spawned_arguments = spawn_arguments("codex", "mcp-lane", None, &cwd);
        spawned_arguments["model"] = json!("flash-x");
        spawned_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", spawned_arguments);
        assert_eq!(spawned["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let session = outcome["session"].as_str().unwrap().to_owned();
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        let binding: SessionBinding = session.parse().unwrap();
        server.pending.observe(&binding, &marker, true).unwrap();

        let mut arguments = close_arguments("accepted");
        arguments["session"] = json!(session);
        arguments["completion_marker"] = json!(marker);
        let response = tool_call(&server, "session_loop_close", arguments);
        assert_eq!(response["result"]["isError"], false);
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(receipt["loop_closed"], true);
        assert_eq!(receipt["outcome"], "accepted");
        assert_eq!(receipt["terminal_closed"], true);
        assert_eq!(receipt["terminal_state"], "closed");
        assert!(receipt.get("close_detail").is_none());
        let closed = backend.closed.lock().unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0], binding);
    }

    #[test]
    fn loop_close_accepted_reports_a_terminal_that_is_already_gone() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut spawned_arguments = spawn_arguments("codex", "mcp-lane-gone", None, &cwd);
        spawned_arguments["model"] = json!("flash-x");
        spawned_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", spawned_arguments);
        assert_eq!(spawned["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let session = outcome["session"].as_str().unwrap().to_owned();
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        let binding: SessionBinding = session.parse().unwrap();
        server.pending.observe(&binding, &marker, true).unwrap();
        backend.mark_gone();

        let mut arguments = close_arguments("accepted");
        arguments["session"] = json!(session);
        arguments["completion_marker"] = json!(marker);
        let response = tool_call(&server, "session_loop_close", arguments);
        assert_eq!(response["result"]["isError"], false);
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(receipt["loop_closed"], true);
        assert_eq!(receipt["terminal_closed"], true);
        assert_eq!(receipt["terminal_state"], "already_gone");
        assert!(receipt["close_detail"]
            .as_str()
            .unwrap()
            .contains("no longer live"));
        assert!(backend.closed.lock().unwrap().is_empty());
    }

    #[test]
    fn loop_close_accepted_works_on_an_autoclosed_lane_whose_checkpoint_is_closed() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut spawned_arguments = spawn_arguments("codex", "mcp-lane-auto", None, &cwd);
        spawned_arguments["model"] = json!("flash-x");
        spawned_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", spawned_arguments);
        assert_eq!(spawned["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let session = outcome["session"].as_str().unwrap().to_owned();
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        let binding: SessionBinding = session.parse().unwrap();
        server.pending.observe(&binding, &marker, true).unwrap();
        server
            .pending
            .close_checkpoints_for_session(&binding.token())
            .unwrap();
        backend.mark_gone();

        let mut arguments = close_arguments("accepted");
        arguments["session"] = json!(session);
        arguments["completion_marker"] = json!(marker);
        let response = tool_call(&server, "session_loop_close", arguments);
        assert_eq!(
            response["result"]["isError"], false,
            "the architect must be able to close the feature loop after the lane autoclosed: {}",
            response["result"]["content"][0]["text"]
        );
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(receipt["loop_closed"], true);
        assert_eq!(receipt["outcome"], "accepted");
        assert_eq!(receipt["terminal_closed"], true);
        assert_eq!(receipt["terminal_state"], "already_gone");
        assert!(
            server.pending.discard(&binding).is_err(),
            "the loop close must delete the closed checkpoint"
        );
        assert!(backend.closed.lock().unwrap().is_empty());
    }

    #[test]
    fn loop_close_accepted_surfaces_a_failed_terminal_close() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut spawned_arguments = spawn_arguments("codex", "mcp-lane-fail", None, &cwd);
        spawned_arguments["model"] = json!("flash-x");
        spawned_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", spawned_arguments);
        assert_eq!(spawned["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let session = outcome["session"].as_str().unwrap().to_owned();
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        let binding: SessionBinding = session.parse().unwrap();
        server.pending.observe(&binding, &marker, true).unwrap();
        backend.fail_closing();

        let mut arguments = close_arguments("accepted");
        arguments["session"] = json!(session);
        arguments["completion_marker"] = json!(marker);
        let response = tool_call(&server, "session_loop_close", arguments);
        assert_eq!(response["result"]["isError"], false);
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(receipt["loop_closed"], true);
        assert_eq!(receipt["terminal_closed"], false);
        assert_eq!(receipt["terminal_state"], "close_failed");
        assert!(receipt["close_detail"]
            .as_str()
            .unwrap()
            .contains("fake close refused"));
        assert!(backend.closed.lock().unwrap().is_empty());
    }

    #[test]
    fn loop_close_accepted_closes_completed_sibling_lanes_and_routes_their_reports() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        backend.set_current(architect_facts(&cwd));
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let initiator = super::super::bridge::driver_token(server.terminals.as_ref());
        assert!(!initiator.is_empty(), "the initiator must be identifiable");

        let mut named_arguments = spawn_arguments("codex", "mcp-lane", None, &cwd);
        named_arguments["model"] = json!("flash-x");
        named_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", named_arguments);
        assert_eq!(
            spawned["result"]["isError"], false,
            "{}",
            spawned["result"]["content"][0]["text"]
        );
        let named_outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let named_session = named_outcome["session"].as_str().unwrap().to_owned();
        let named_marker = named_outcome["completion_marker"]
            .as_str()
            .unwrap()
            .to_owned();
        let named_binding: SessionBinding = named_session.parse().unwrap();
        server
            .pending
            .observe(&named_binding, &named_marker, true)
            .unwrap();

        let sibling_binding: SessionBinding = "v1:kitty:spawn-sib-lane:200".parse().unwrap();
        backend.add_lane(lane_facts("sib-lane", &cwd));
        server
            .pending
            .set_role(&sibling_binding, super::super::bridge::Role::Lane)
            .unwrap();
        server
            .pending
            .start(
                &sibling_binding,
                "QOL_BRIDGE_DONE_sib",
                &initiator,
                true,
                None,
            )
            .unwrap();
        server
            .pending
            .observe(&sibling_binding, "QOL_BRIDGE_DONE_sib", true)
            .unwrap();
        server
            .pending
            .store_screen(
                &sibling_binding,
                "QOL_BRIDGE_DONE_sib",
                "sibling final report",
            )
            .unwrap();

        let mut arguments = close_arguments("accepted");
        arguments["session"] = json!(named_session);
        arguments["completion_marker"] = json!(named_marker);
        let response = tool_call(&server, "session_loop_close", arguments);
        assert_eq!(response["result"]["isError"], false);
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(receipt["terminal_closed"], true);
        let siblings = receipt["sibling_lanes"].as_array().unwrap();
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings[0]["session"], sibling_binding.token());
        assert_eq!(siblings[0]["key"], "sib-lane");
        assert_eq!(siblings[0]["tool"], "codex");
        assert_eq!(siblings[0]["terminal_state"], "closed");
        assert_eq!(siblings[0]["closed"], true);
        let report_file = siblings[0]["report_file"]
            .as_str()
            .expect("report_file path");
        assert!(report_file.ends_with(&format!(
            "sibling-reports/sibling-report-{}.md",
            sibling_binding.token()
        )));
        assert_eq!(siblings[0]["report"], report_file);
        assert_eq!(
            std::fs::read_to_string(report_file).unwrap(),
            "sibling final report"
        );
        let closed = backend.closed.lock().unwrap();
        assert_eq!(closed.len(), 2);
        assert!(closed.contains(&named_binding));
        assert!(closed.contains(&sibling_binding));
    }

    #[test]
    fn loop_close_accepted_leaves_uncompleted_sibling_lanes_untouched() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        backend.set_current(architect_facts(&cwd));
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let initiator = super::super::bridge::driver_token(server.terminals.as_ref());

        let mut named_arguments = spawn_arguments("codex", "mcp-lane", None, &cwd);
        named_arguments["model"] = json!("flash-x");
        named_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", named_arguments);
        assert_eq!(spawned["result"]["isError"], false);
        let named_outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let named_session = named_outcome["session"].as_str().unwrap().to_owned();
        let named_marker = named_outcome["completion_marker"]
            .as_str()
            .unwrap()
            .to_owned();
        let named_binding: SessionBinding = named_session.parse().unwrap();
        server
            .pending
            .observe(&named_binding, &named_marker, true)
            .unwrap();

        let open_binding: SessionBinding = "v1:kitty:spawn-open-lane:200".parse().unwrap();
        backend.add_lane(lane_facts("open-lane", &cwd));
        server
            .pending
            .set_role(&open_binding, super::super::bridge::Role::Lane)
            .unwrap();
        server
            .pending
            .start(
                &open_binding,
                "QOL_BRIDGE_DONE_open",
                &initiator,
                true,
                None,
            )
            .unwrap();

        let mut arguments = close_arguments("accepted");
        arguments["session"] = json!(named_session);
        arguments["completion_marker"] = json!(named_marker);
        let response = tool_call(&server, "session_loop_close", arguments);
        assert_eq!(response["result"]["isError"], false);
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert!(receipt.get("sibling_lanes").is_none());
        let closed = backend.closed.lock().unwrap();
        assert_eq!(closed.as_slice(), &[named_binding]);
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
    fn loop_close_mcp_receipt_is_the_canonical_final_report() {
        let (server, backend) = server(Vec::new(), false, false);
        let binding: SessionBinding = token().parse().unwrap();
        server
            .pending
            .start(&binding, "QOL_BRIDGE_DONE_final", "", false, None)
            .unwrap();
        server
            .pending
            .observe(&binding, "QOL_BRIDGE_DONE_final", true)
            .unwrap();
        let response = tool_call(&server, "session_loop_close", close_arguments("paused"));
        assert_eq!(response["result"]["isError"], false);
        let receipt: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(
            receipt,
            json!({
                "loop_closed": true,
                "outcome": "paused",
                "final_report": "## What landed\n\nThe feature landed.\n\n## Before\n\nThe loop stopped at round boundaries.\n\n## Now\n\nThe loop continues through acceptance.\n\n## Verification\n\nFocused tests pass.\n\n## Remaining\n\nNone.",
            })
        );
        assert!(backend.closed.lock().unwrap().is_empty());
    }

    #[test]
    fn loop_close_cli_accepted_closes_the_spawned_terminal() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut spawned_arguments = spawn_arguments("codex", "cli-lane", None, &cwd);
        spawned_arguments["model"] = json!("flash-x");
        spawned_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", spawned_arguments);
        assert_eq!(spawned["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let session = outcome["session"].as_str().unwrap().to_owned();
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        let binding: SessionBinding = session.parse().unwrap();
        server.pending.observe(&binding, &marker, true).unwrap();

        let receipt = super::super::close::loop_close_from_args(
            server.terminals.as_ref(),
            &server.pending,
            root.path(),
            &[
                std::ffi::OsString::from(session.as_str()),
                std::ffi::OsString::from("--completion-marker"),
                std::ffi::OsString::from(marker.as_str()),
                std::ffi::OsString::from("--outcome"),
                std::ffi::OsString::from("accepted"),
                std::ffi::OsString::from("--landed"),
                std::ffi::OsString::from("the loop-close verb"),
                std::ffi::OsString::from("--verification"),
                std::ffi::OsString::from("focused tests pass"),
            ],
        )
        .unwrap();
        assert_eq!(receipt["loop_closed"], true);
        assert_eq!(receipt["outcome"], "accepted");
        assert_eq!(receipt["terminal_closed"], true);
        assert_eq!(receipt["terminal_state"], "closed");
        assert!(receipt["final_report"]
            .as_str()
            .unwrap()
            .contains("the loop-close verb"));
        assert!(server.pending.pending_round(&binding).unwrap().is_none());
        let closed = backend.closed.lock().unwrap();
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0], binding);
    }

    #[test]
    fn loop_close_cli_paused_leaves_the_terminal_open() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut spawned_arguments = spawn_arguments("codex", "cli-pause-lane", None, &cwd);
        spawned_arguments["model"] = json!("flash-x");
        spawned_arguments["task"] = json!("build the bounded change");
        let spawned = tool_call(&server, "session_spawn", spawned_arguments);
        assert_eq!(spawned["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(spawned["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let session = outcome["session"].as_str().unwrap().to_owned();
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        let binding: SessionBinding = session.parse().unwrap();

        let receipt = super::super::close::loop_close_from_args(
            server.terminals.as_ref(),
            &server.pending,
            root.path(),
            &[
                std::ffi::OsString::from(session.as_str()),
                std::ffi::OsString::from("--completion-marker"),
                std::ffi::OsString::from(marker.as_str()),
                std::ffi::OsString::from("--outcome"),
                std::ffi::OsString::from("paused"),
            ],
        )
        .unwrap();
        assert_eq!(receipt["loop_closed"], true);
        assert_eq!(receipt["outcome"], "paused");
        assert!(receipt.get("terminal_closed").is_none());
        assert!(receipt.get("terminal_state").is_none());
        assert!(receipt["final_report"]
            .as_str()
            .unwrap()
            .contains("(not provided)"));
        assert!(server.pending.pending_round(&binding).unwrap().is_none());
        assert!(backend.closed.lock().unwrap().is_empty());
        assert!(server
            .terminals
            .discover()
            .unwrap()
            .iter()
            .any(|facts| facts.id == *binding.session_id()));
    }

    #[test]
    fn loop_close_cli_rejects_invalid_outcome_and_missing_required_input() {
        let root = tempfile::TempDir::new().unwrap();
        let (server, backend) = server(Vec::new(), false, false);
        let binding: SessionBinding = token().parse().unwrap();
        server
            .pending
            .start(&binding, "QOL_BRIDGE_DONE_final", "", false, None)
            .unwrap();
        let call = |arguments: &[&str]| {
            super::super::close::loop_close_from_args(
                server.terminals.as_ref(),
                &server.pending,
                root.path(),
                &arguments
                    .iter()
                    .map(std::ffi::OsString::from)
                    .collect::<Vec<_>>(),
            )
        };

        let error = call(&[
            token().as_str(),
            "--completion-marker",
            "QOL_BRIDGE_DONE_final",
            "--outcome",
            "done",
        ])
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("`--outcome` must be `accepted` or `paused`"),
            "{error}"
        );
        assert!(server.pending.pending_round(&binding).unwrap().is_some());

        let error = call(&[token().as_str(), "--outcome", "paused"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("--completion-marker"), "{error}");

        let error = call(&[]).unwrap_err().to_string();
        assert!(error.contains("usage:"), "{error}");

        let error = call(&[token().as_str(), "--bogus", "x"])
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown loop-close flag"), "{error}");
        assert!(backend.closed.lock().unwrap().is_empty());
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

    #[test]
    fn session_close_requires_a_spawned_session_with_a_closed_loop() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());

        let unspawned = tool_call(
            &server,
            "session_close",
            json!({ "session": FakeBackend::session().binding().unwrap().token() }),
        );
        assert_eq!(unspawned["result"]["isError"], true);
        assert!(unspawned["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("was not spawned"));

        let mut close_lane = spawn_arguments("codex", "mcp-lane", None, &cwd);
        close_lane["model"] = json!("flash-x");
        close_lane["task"] = json!("build the bounded change");
        let spawned_response = tool_call(&server, "session_spawn", close_lane);
        assert_eq!(spawned_response["result"]["isError"], false);
        let spawned_outcome: Value = serde_json::from_str(
            spawned_response["result"]["content"][0]["text"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let spawned_marker = spawned_outcome["completion_marker"]
            .as_str()
            .unwrap()
            .to_owned();
        let spawned = "v1:kitty:spawn-mcp-lane:200";
        let binding: SessionBinding = spawned.parse().unwrap();
        let open_loop = tool_call(&server, "session_close", json!({ "session": spawned }));
        assert_eq!(open_loop["result"]["isError"], true);
        assert!(open_loop["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("open feature loop"));
        assert!(backend.closed.lock().unwrap().is_empty());

        server
            .pending
            .acknowledge(&binding, &spawned_marker, false)
            .unwrap();
        let closed = tool_call(&server, "session_close", json!({ "session": spawned }));
        assert_eq!(closed["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(closed["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(outcome["closed"], true);
        assert_eq!(outcome["key"], "mcp-lane");
        assert_eq!(outcome["tool"], "codex");
        assert_eq!(backend.closed.lock().unwrap().len(), 1);
    }

    #[test]
    fn session_close_refuses_the_calling_terminal_and_dead_targets() {
        let (server, backend) = server(Vec::new(), false, false);
        *backend.current.lock().unwrap() = true;
        let refused = tool_call(&server, "session_close", json!({ "session": token() }));
        assert_eq!(refused["result"]["isError"], true);
        assert!(refused["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("calling terminal"));

        *backend.current.lock().unwrap() = false;
        let missing = tool_call(
            &server,
            "session_close",
            json!({ "session": "v1:fake:999:1" }),
        );
        assert_eq!(missing["result"]["isError"], true);
        assert!(missing["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("not a live session"));
        assert!(backend.closed.lock().unwrap().is_empty());
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
        let mut arguments = spawn_arguments("codex", "mcp-lane", None, &cwd);
        arguments["model"] = json!("flash-x");
        arguments["task"] = json!("implement the fix");
        let response = tool_call(&server, "session_spawn", arguments);
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
    fn session_spawn_model_override_reaches_the_launch_and_outcome() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut arguments = spawn_arguments("pi", "mcp-lane-model", None, &cwd);
        arguments["model"] = json!("flash-x");
        arguments["task"] = json!("implement the fix");
        let response = tool_call(&server, "session_spawn", arguments);
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["model"], json!("flash-x"));
        let request = backend.spawn_launch.lock().unwrap().clone().unwrap();
        assert_eq!(request.program, "pi");
        assert_eq!(request.args.len(), 3);
        assert_eq!(
            request.args[..2],
            vec!["--model".to_owned(), "flash-x".to_owned()]
        );
        assert!(
            request.args[2].contains("[qol session bridge]"),
            "the launch must embed the first round after the model flags"
        );
    }

    #[test]
    fn session_spawn_model_rejects_non_string_values() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut arguments = spawn_arguments("pi", "mcp-lane-model", None, &cwd);
        arguments["model"] = json!(42);
        let response = tool_call(&server, "session_spawn", arguments);
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("model` must be a string"));
    }

    #[test]
    fn session_spawn_refuses_a_new_lane_without_an_explicit_model() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());

        let mut missing = spawn_arguments("codex", "mcp-lane", None, &cwd);
        missing["task"] = json!("implement the fix");
        let missing = tool_call(&server, "session_spawn", missing);
        assert_eq!(missing["result"]["isError"], true);
        assert!(missing["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("--model"));

        let mut empty = spawn_arguments("codex", "mcp-lane-empty", None, &cwd);
        empty["model"] = json!("");
        empty["task"] = json!("implement the fix");
        let empty = tool_call(&server, "session_spawn", empty);
        assert_eq!(empty["result"]["isError"], true);
        assert!(empty["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("--model"));
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn spawn_queues_the_round_without_pasting_and_reuse_delivers_on_top() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(vec![live_pi_screen()], false, false)
                .with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut arguments = spawn_arguments("pi", "mcp-bg", None, &cwd);
        arguments["model"] = json!("flash-x");
        arguments["task"] = json!("implement the fix");
        let response = tool_call(&server, "session_spawn", arguments);
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["background"], true);
        assert_eq!(outcome["reused"], false);
        assert_eq!(outcome["task_submitted"], true);
        assert_eq!(outcome["session"], "v1:kitty:spawn-mcp-bg:200");
        let marker = outcome["completion_marker"].as_str().unwrap().to_owned();
        assert!(marker.starts_with("QOL_BRIDGE_DONE_"));
        assert_eq!(
            backend.sent.lock().unwrap().len(),
            0,
            "the spawn embeds the task in the launch instead of pasting it"
        );

        let launch = backend.spawn_launch.lock().unwrap().clone().unwrap();
        assert_eq!(launch.program, "pi");
        assert!(launch.args.last().unwrap().contains("[qol session bridge]"));
        assert!(launch.args.last().unwrap().contains("implement the fix"));

        let binding: SessionBinding = outcome["session"].as_str().unwrap().parse().unwrap();
        let round = server.pending.pending_round(&binding).unwrap().unwrap();
        assert_eq!(round.completion_marker, marker);
        assert!(!round.completed);

        let second = tool_call(
            &server,
            "session_spawn",
            json!({
                "tool": "pi",
                "cwd": cwd,
                "key": "mcp-bg",
                "model": "flash-x",
                "task": "second round",
            }),
        );
        assert_eq!(second["result"]["isError"], true);
        assert!(second["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("already pending"));
        assert_eq!(backend.sent.lock().unwrap().len(), 0);

        server
            .pending
            .acknowledge(&binding, &marker, false)
            .unwrap();
        let reused = tool_call(
            &server,
            "session_spawn",
            json!({
                "tool": "pi",
                "cwd": cwd,
                "key": "mcp-bg",
                "model": "flash-x",
                "task": "second round",
            }),
        );
        assert_eq!(reused["result"]["isError"], false);
        let reused_outcome: Value =
            serde_json::from_str(reused["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(reused_outcome["reused"], true);
        assert_eq!(
            backend.sent.lock().unwrap().len(),
            1,
            "the reuse path pastes the task into the live session"
        );
    }

    #[test]
    fn spawn_rejects_an_explicit_background_argument() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(FakeBackend::new(Vec::new(), false, false));
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        for background in [json!(true), json!("yes")] {
            let mut arguments = spawn_arguments("pi", "mcp-bg", None, &cwd);
            arguments["model"] = json!("flash-x");
            arguments["task"] = json!("implement the fix");
            arguments["background"] = background;
            let response = tool_call(&server, "session_spawn", arguments);
            assert_eq!(response["result"]["isError"], true);
            assert!(response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("background is the only mode"));
        }
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn spawn_requires_a_task() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(FakeBackend::new(Vec::new(), false, false));
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut no_task = spawn_arguments("pi", "mcp-bg", None, &cwd);
        no_task["model"] = json!("flash-x");
        let response = tool_call(&server, "session_spawn", no_task);
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("missing `task` string argument"));
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    #[test]
    fn autoclose_spawn_marks_the_outcome_and_round_and_rejects_the_argument() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(vec![live_pi_screen()], false, false)
                .with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut arguments = spawn_arguments("pi", "mcp-auto", None, &cwd);
        arguments["model"] = json!("flash-x");
        arguments["task"] = json!("implement the fix");
        let response = tool_call(&server, "session_spawn", arguments);
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["autoclose"], true);
        let binding: SessionBinding = outcome["session"].as_str().unwrap().parse().unwrap();
        let round = server.pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            round.autoclose,
            "the queued round must carry autoclose for the watcher"
        );

        for autoclose in [json!(true), json!(false), json!("yes")] {
            let mut rejected = spawn_arguments("pi", "mcp-auto-rejected", None, &cwd);
            rejected["model"] = json!("flash-x");
            rejected["task"] = json!("implement the fix");
            rejected["autoclose"] = autoclose;
            let response = tool_call(&server, "session_spawn", rejected);
            assert_eq!(response["result"]["isError"], true);
            assert!(response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("`autoclose` was removed; lanes always close on completion"));
        }
        assert_eq!(
            backend
                .spawn_count
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn spawn_defaults_autoclose_to_true_and_rejects_an_opt_out() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(vec![live_pi_screen()], false, false)
                .with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut arguments = spawn_arguments("pi", "mcp-defaults", None, &cwd);
        arguments["model"] = json!("flash-x");
        arguments["task"] = json!("implement the fix");
        let response = tool_call(&server, "session_spawn", arguments);
        assert_eq!(response["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["autoclose"], true);
        let binding: SessionBinding = outcome["session"].as_str().unwrap().parse().unwrap();
        let round = server.pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            round.autoclose,
            "the queued round must default to autoclose"
        );

        let mut opted_out = spawn_arguments("pi", "mcp-no-auto", None, &cwd);
        opted_out["model"] = json!("flash-x");
        opted_out["task"] = json!("implement the fix");
        opted_out["autoclose"] = json!(false);
        let response = tool_call(&server, "session_spawn", opted_out);
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("`autoclose` was removed; lanes always close on completion"));
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
            FakeBackend::new(vec![live_pi_screen()], false, false)
                .with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let mut first_arguments = spawn_arguments("pi", "mcp-lane", None, &cwd);
        first_arguments["model"] = json!("flash-x");
        first_arguments["task"] = json!("first round");
        let first = tool_call(&server, "session_spawn", first_arguments);
        assert_eq!(first["result"]["isError"], false);
        let first_outcome: Value =
            serde_json::from_str(first["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        let binding: SessionBinding = first_outcome["session"].as_str().unwrap().parse().unwrap();
        let marker = first_outcome["completion_marker"]
            .as_str()
            .unwrap()
            .to_owned();

        let mut second_arguments = spawn_arguments("pi", "mcp-lane", None, "ignored-missing-cwd");
        second_arguments["task"] = json!("second round");
        let second = tool_call(&server, "session_spawn", second_arguments.clone());
        assert_eq!(second["result"]["isError"], true);
        assert!(second["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("already pending"));

        server
            .pending
            .acknowledge(&binding, &marker, false)
            .unwrap();
        let third = tool_call(&server, "session_spawn", second_arguments);
        assert_eq!(third["result"]["isError"], false);
        let outcome: Value =
            serde_json::from_str(third["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
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
        let mut first_arguments = spawn_arguments("codex", "mcp-lane", None, &cwd);
        first_arguments["model"] = json!("flash-x");
        first_arguments["task"] = json!("first round");
        tool_call(&server, "session_spawn", first_arguments);
        let mut conflict_arguments = spawn_arguments("claude", "mcp-lane", None, &cwd);
        conflict_arguments["task"] = json!("conflicting round");
        let conflict = tool_call(&server, "session_spawn", conflict_arguments);
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
        let mut unknown_arguments = spawn_arguments("codex", "mcp-lane", Some("floating"), &cwd);
        unknown_arguments["task"] = json!("first round");
        let unknown = tool_call(&server, "session_spawn", unknown_arguments);
        assert_eq!(unknown["result"]["isError"], true);
        assert!(unknown["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid surface `floating`"));

        let mut unsupported_arguments =
            spawn_arguments("codex", "mcp-lane", Some("os-window"), &cwd);
        unsupported_arguments["model"] = json!("flash-x");
        unsupported_arguments["task"] = json!("first round");
        let unsupported = tool_call(&server, "session_spawn", unsupported_arguments);
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
        let mut generic_arguments = spawn_arguments("generic", "mcp-lane", None, &cwd);
        generic_arguments["task"] = json!("first round");
        let response = tool_call(&server, "session_spawn", generic_arguments);
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

    #[test]
    fn tersify_sibling_lanes_caps_fields_routes_reports_and_drops_overflow() {
        use super::super::close::{SiblingLaneClose, TerminalCloseState};
        let root = tempfile::TempDir::new().unwrap();
        let long_detail = "x".repeat(300);
        let siblings: Vec<SiblingLaneClose> = (0..52)
            .map(|index| SiblingLaneClose {
                session: format!("v1:kitty:lane-{index}:200"),
                key: Some(format!("lane-{index}")),
                tool: Some("codex".to_owned()),
                closed: true,
                terminal_state: TerminalCloseState::Closed,
                report: if index == 0 {
                    "sibling report body".to_owned()
                } else {
                    String::new()
                },
                close_detail: if index % 2 == 0 {
                    Some(long_detail.clone())
                } else {
                    None
                },
            })
            .collect();
        let entries =
            tersify_sibling_lanes(&siblings, root.path(), &std::collections::HashMap::new())
                .unwrap();
        assert_eq!(entries.len(), 51, "50 kept lanes plus the dropped note");
        assert_eq!(entries[50]["dropped_sibling_lanes"], 2);
        let first = &entries[0];
        assert_eq!(first["session"], "v1:kitty:lane-0:200");
        let report_file = first["report_file"].as_str().unwrap();
        assert!(report_file.ends_with("sibling-reports/sibling-report-v1:kitty:lane-0:200.md"));
        assert_eq!(first["report"], report_file);
        assert_eq!(
            std::fs::read_to_string(report_file).unwrap(),
            "sibling report body"
        );
        let detail = entries[2]["close_detail"].as_str().unwrap();
        assert!(detail.ends_with("..."));
        assert_eq!(
            detail.chars().count(),
            MAX_SIBLING_FIELD_CHARS + TRUNCATE_MARKER.len()
        );
        assert!(
            entries[1].get("report").is_none(),
            "empty reports are dropped"
        );
    }
    fn spawn_lane(
        server: &McpSessionServer,
        backend: &Arc<FakeBackend>,
        key: &str,
        cwd: &str,
        group: Option<&str>,
    ) -> (String, String) {
        let mut arguments = spawn_arguments("codex", key, None, cwd);
        arguments["model"] = json!("flash-x");
        arguments["task"] = json!("research the bounded question and write the report");
        if let Some(group) = group {
            arguments["group"] = json!(group);
        }
        let response = tool_call(server, "session_spawn", arguments);
        assert_eq!(
            response["result"]["isError"], false,
            "spawn failed: {response}"
        );
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let marker = backend
            .lane_marker(key)
            .expect("the launch prompt carries the round marker");
        assert_eq!(outcome["completion_marker"], marker);
        (outcome["session"].as_str().unwrap().to_owned(), marker)
    }

    fn run_watcher(
        server: &McpSessionServer,
        tokens: &[String],
        trace_dir: &std::path::Path,
    ) -> Vec<Value> {
        let mut out = Vec::new();
        super::super::watch::watch(
            server.terminals.as_ref(),
            &server.interpreter,
            &server.pending,
            &server.ledger,
            &server.locks,
            tokens,
            &mut out,
            trace_dir,
            super::super::watch::WatchConfig::fast_for_tests(),
        )
        .unwrap();
        String::from_utf8_lossy(&out)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn e2e_a_spawned_lane_completes_through_the_watcher_and_bridge_collects_it() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());

        let (session, marker) = spawn_lane(&server, &backend, "lane-one", &cwd, None);
        backend.show_lane_screen("lane-one", format!("the report is written\n{marker}"));
        let events = run_watcher(&server, std::slice::from_ref(&session), root.path());
        assert_eq!(events.len(), 1, "events: {events:?}");
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["session"], session);

        let waited = tool_call(&server, "session_bridge", json!({ "session": session }));
        assert_eq!(
            waited["result"]["isError"], false,
            "bridge failed: {waited}"
        );
        let outcome: Value =
            serde_json::from_str(waited["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(outcome["completed"], true);
        assert_eq!(outcome["completion_marker"], marker);
        assert!(
            outcome["screen"]
                .as_str()
                .unwrap()
                .contains("the report is written"),
            "the bridge returns the lane's own report: {outcome}"
        );
    }

    fn spawn_lane_set(
        server: &McpSessionServer,
        backend: &Arc<FakeBackend>,
        keys: &[&str],
        cwd: &str,
    ) -> (Value, Vec<(String, String)>) {
        let lanes = keys
            .iter()
            .map(|key| json!({ "key": key, "task": format!("cover the {key} slice of the work") }))
            .collect::<Vec<_>>();
        let response = tool_call(
            server,
            "session_spawn",
            json!({
                "tool": "codex",
                "cwd": cwd,
                "model": "flash-x",
                "lanes": lanes,
            }),
        );
        assert_eq!(
            response["result"]["isError"], false,
            "lane set spawn failed: {response}"
        );
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        let lanes = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let marker = backend
                    .lane_marker(key)
                    .expect("every lane launch carries its own round marker");
                assert_eq!(outcome["lanes"][index]["completion_marker"], marker);
                (
                    outcome["lanes"][index]["session"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                    marker,
                )
            })
            .collect::<Vec<_>>();
        (outcome, lanes)
    }

    #[test]
    fn e2e_a_lane_set_spawned_in_one_call_wakes_the_architect_once() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        backend.set_current(architect_facts(&cwd));
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let keys = ["set-a", "set-b"];

        let (outcome, lanes) = spawn_lane_set(&server, &backend, &keys, &cwd);
        assert_eq!(outcome["combined_report"], true);
        let group = outcome["group"]
            .as_str()
            .expect("two lanes in one call are grouped automatically")
            .to_owned();
        assert_eq!(outcome["lanes"].as_array().unwrap().len(), keys.len());

        for (index, (key, (session, marker))) in keys.iter().zip(&lanes).enumerate() {
            backend.show_lane_screen(key, format!("findings from {key}\n{marker}"));
            let events = run_watcher(&server, std::slice::from_ref(session), root.path());
            if index + 1 == keys.len() {
                assert_eq!(events.len(), 1, "the final lane wakes once: {events:?}");
            } else {
                assert!(events.is_empty(), "an early lane stays silent: {events:?}");
            }
        }

        let wakes = backend
            .sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, text, _)| text.contains("grouped research"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(wakes.len(), 1, "exactly one combined wake: {wakes:?}");
        assert!(
            wakes[0].1.contains("all 2 lanes finished"),
            "the wake counts every lane in the set: {:?}",
            wakes[0].1
        );
        let combined =
            std::fs::read_to_string(root.path().join("groups").join(&group).join("combined.md"))
                .expect("the set writes one combined report");
        for key in keys {
            assert!(
                combined.contains(&format!("findings from {key}")),
                "the combined report holds every lane: {combined}"
            );
        }
    }

    #[test]
    fn e2e_a_second_ungrouped_lane_is_refused() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        backend.set_current(architect_facts(&cwd));
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());

        spawn_lane(&server, &backend, "solo-one", &cwd, None);
        let mut arguments = spawn_arguments("codex", "solo-two", None, &cwd);
        arguments["model"] = json!("flash-x");
        arguments["task"] = json!("chase the second thread");
        let response = tool_call(&server, "session_spawn", arguments);
        assert_eq!(
            response["result"]["isError"], true,
            "a second ungrouped lane must be refused: {response}"
        );
        let message = response["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            message.contains("lanes") && message.contains("solo-one"),
            "the refusal names the live lane and the lanes argument: {message}"
        );
    }

    #[test]
    fn a_lane_set_refuses_a_top_level_task() {
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
            json!({
                "tool": "codex",
                "cwd": cwd,
                "model": "flash-x",
                "task": "do everything",
                "lanes": [{ "key": "one", "task": "first slice" }],
            }),
        );
        assert_eq!(response["result"]["isError"], true, "{response}");
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Move it into the matching lane entry"));
    }

    #[test]
    fn e2e_grouped_spawns_wake_the_architect_once_with_every_lane() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        backend.set_current(architect_facts(&cwd));
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let group = "staleness-research";
        let keys = ["research-a", "research-b", "research-c"];

        let mut lanes = Vec::new();
        for key in keys {
            lanes.push(spawn_lane(&server, &backend, key, &cwd, Some(group)));
        }

        for (index, (key, (session, marker))) in keys.iter().zip(&lanes).enumerate() {
            backend.show_lane_screen(key, format!("findings from {key}\n{marker}"));
            let events = run_watcher(&server, std::slice::from_ref(session), root.path());
            let last = index + 1 == keys.len();
            if last {
                assert_eq!(events.len(), 1, "the final lane wakes once: {events:?}");
                assert_eq!(events[0]["event"], "completed");
            } else {
                assert!(
                    events.is_empty(),
                    "an early grouped lane must stay silent: {events:?}"
                );
            }
        }

        let wakes = backend
            .sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, text, _)| text.contains("grouped research"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(wakes.len(), 1, "exactly one combined wake: {wakes:?}");
        assert!(
            wakes[0].1.contains("all 3 lanes finished"),
            "the combined wake counts every member: {:?}",
            wakes[0].1
        );
        let combined =
            std::fs::read_to_string(root.path().join("groups").join(group).join("combined.md"))
                .expect("combined report written");
        for key in keys {
            assert!(
                combined.contains(&format!("findings from {key}")),
                "the combined report must carry {key}: {combined}"
            );
        }
        assert_eq!(
            combined.matches("## v1:kitty:spawn-").count(),
            3,
            "every spawned lane gets a section: {combined}"
        );
    }
    #[test]
    fn e2e_a_forked_architect_is_detached_and_never_bridgeable() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());

        let response = tool_call(
            &server,
            "session_fork",
            json!({
                "tool": "claude",
                "cwd": cwd,
                "key": "chase-lockfile",
                "model": "opus",
                "effort": "xhigh",
                "brief": "cargo metadata --locked fails on CI but passes locally",
            }),
        );
        assert_eq!(
            response["result"]["isError"], false,
            "fork failed: {response}"
        );
        let outcome: Value =
            serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(outcome["detached"], true);
        assert_eq!(outcome["model"], "opus");
        assert_eq!(outcome["effort"], "xhigh");
        assert!(
            outcome.get("completion_marker").is_none(),
            "a fork carries no round marker: {outcome}"
        );

        let launch = backend.spawn_launch.lock().unwrap().clone().unwrap();
        assert_eq!(launch.program, "claude");
        assert!(
            launch
                .args
                .windows(2)
                .any(|pair| pair == ["--model", "opus"]),
            "the fork launches at its own model: {:?}",
            launch.args
        );
        assert!(
            launch
                .args
                .windows(2)
                .any(|pair| pair == ["--effort", "xhigh"]),
            "the fork launches at its own effort: {:?}",
            launch.args
        );
        let prompt = launch.args.last().unwrap();
        assert!(prompt.contains("detached architect"), "{prompt}");
        assert!(
            !prompt.contains("Completion fragments"),
            "no completion marker may reach a fork: {prompt}"
        );

        let brief_path = outcome["brief"].as_str().unwrap();
        assert!(
            prompt.contains(brief_path),
            "the prompt names the brief file: {prompt}"
        );
        assert_eq!(
            std::fs::read_to_string(brief_path).unwrap(),
            "cargo metadata --locked fails on CI but passes locally"
        );

        let binding: SessionBinding = outcome["session"].as_str().unwrap().parse().unwrap();
        assert!(
            server.pending.pending_round(&binding).unwrap().is_none(),
            "a fork opens no round"
        );

        let refused = tool_call(
            &server,
            "session_bridge",
            json!({ "session": outcome["session"] }),
        );
        assert_eq!(refused["result"]["isError"], true);
        let message = refused["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            message.contains("detached fork") && message.contains("chase-lockfile"),
            "the refusal must name the fork: {message}"
        );

        let records = server.forks.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, "chase-lockfile");
        assert_eq!(records[0].session, outcome["session"].as_str().unwrap());
    }

    #[test]
    fn e2e_a_fork_refuses_lane_shaped_arguments() {
        let root = tempfile::TempDir::new().unwrap();
        let cwd = spawn_cwd(&root);
        let backend = Arc::new(
            FakeBackend::new(Vec::new(), false, false).with_id(BackendId::new("kitty").unwrap()),
        );
        backend.enable_spawner();
        let server = server_with_backend(backend.clone(), root.path().to_path_buf());

        for (argument, expected) in [("task", "no `task`"), ("group", "no `group`")] {
            let mut arguments = json!({
                "cwd": cwd,
                "key": format!("fork-{argument}"),
                "model": "opus",
                "brief": "a problem worth its own tree",
            });
            arguments[argument] = json!("whatever");
            let response = tool_call(&server, "session_fork", arguments);
            assert_eq!(
                response["result"]["isError"], true,
                "{argument}: {response}"
            );
            let message = response["result"]["content"][0]["text"].as_str().unwrap();
            assert!(message.contains(expected), "{argument}: {message}");
        }
        assert_eq!(backend.spawn_count.load(Ordering::Relaxed), 0);
    }

    type TestWriter = Arc<Mutex<BufWriter<Vec<u8>>>>;
    type ScriptedContext = (
        Arc<McpSessionServer>,
        TestWriter,
        Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
        Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
    );

    fn scripted_harness(server: McpSessionServer) -> ScriptedContext {
        let server = Arc::new(server);
        let writer = Arc::new(Mutex::new(BufWriter::new(Vec::new())));
        let cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let workers: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>> =
            Arc::new(Mutex::new(Vec::new()));
        (server, writer, cancellations, workers)
    }

    fn feed_line(
        server: &Arc<McpSessionServer>,
        writer: &TestWriter,
        cancellations: &Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
        workers: &Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
        line: &Value,
    ) {
        dispatch_line(
            server,
            writer,
            cancellations,
            workers,
            &serde_json::to_string(line).unwrap(),
        )
        .unwrap();
    }

    fn read_responses(writer: &TestWriter) -> Vec<Value> {
        let data = writer.lock().unwrap().get_ref().clone();
        String::from_utf8(data)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn wait_for_responses(writer: &TestWriter, count: usize, deadline: Duration) -> Vec<Value> {
        let started = Instant::now();
        loop {
            let responses = read_responses(writer);
            if responses.len() >= count {
                return responses;
            }
            assert!(
                started.elapsed() < deadline,
                "timed out after {:?} waiting for {count} responses; got {}: {:?}",
                deadline,
                responses.len(),
                responses
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn drain_workers(workers: &Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>) {
        for worker in workers.lock().unwrap().drain(..) {
            let _ = worker.join();
        }
    }

    fn pending_bridge_request(id: i64) -> Value {
        request(
            id,
            "tools/call",
            json!({ "name": "session_bridge", "arguments": { "session": token() } }),
        )
    }

    #[test]
    fn a_ping_is_answered_while_a_bridge_call_is_still_in_flight() {
        let (server, _backend) = server(Vec::new(), false, false);
        let (server, writer, cancellations, workers) = scripted_harness(server);
        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &request(
                1,
                "tools/call",
                json!({ "name": "session_submit", "arguments": { "session": token(), "task": "implement the bounded change" } }),
            ),
        );
        wait_for_responses(&writer, 1, Duration::from_secs(2));

        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &pending_bridge_request(2),
        );
        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &request(3, "ping", json!({})),
        );
        let responses = wait_for_responses(&writer, 3, Duration::from_secs(2));
        assert_eq!(
            responses[1]["id"], 3,
            "the ping must be answered while the bridge is still waiting: {responses:?}"
        );
        assert_eq!(responses[2]["id"], 2);
        drain_workers(&workers);
    }

    #[test]
    fn a_cancelled_bridge_releases_the_owner_and_leaves_the_round_open() {
        let (mut server, _backend) = server(Vec::new(), false, false);
        let binding: SessionBinding = token().parse().unwrap();
        server.round_timeout = Duration::from_secs(30);
        let (server, writer, cancellations, workers) = scripted_harness(server);

        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &request(
                1,
                "tools/call",
                json!({ "name": "session_submit", "arguments": { "session": token(), "task": "implement the bounded change" } }),
            ),
        );
        wait_for_responses(&writer, 1, Duration::from_secs(2));
        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &pending_bridge_request(5),
        );
        let started = Instant::now();
        let deadline = started + Duration::from_secs(5);
        while server.pending.owner_pid(&binding).is_none() {
            assert!(
                Instant::now() < deadline,
                "the bridge never attached to the round"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": 5 },
            }),
        );
        let responses = wait_for_responses(&writer, 2, Duration::from_secs(3));
        assert_eq!(
            responses[1]["id"], 5,
            "the cancelled bridge must respond promptly: {responses:?}"
        );
        assert_eq!(responses[1]["result"]["isError"], true);
        assert!(
            responses[1]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("cancelled"),
            "the cancellation must be named: {responses:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the cancelled bridge took {:?} to return",
            started.elapsed()
        );

        let round = server.pending.pending_round(&binding).unwrap().unwrap();
        assert!(
            !round.completed,
            "cancelling a bridge must not complete the round"
        );
        assert!(
            server.pending.owner_pid(&binding).is_none(),
            "the cancelled bridge must release its attach"
        );
        drain_workers(&workers);
    }

    #[test]
    fn a_cancelled_pi_bridge_releases_the_owner_and_leaves_the_round_open() {
        let root = tempfile::TempDir::new().unwrap();
        let backend = Arc::new(FakeBackend::new(Vec::new(), false, false));
        let pi_facts = SessionFacts {
            id: SessionId::new(BackendId::new("fake").unwrap(), "9").unwrap(),
            root_pid: 300,
            cwd: "/work/demo".to_owned(),
            title: "Pi REPL".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: vec!["pi".to_owned()],
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        };
        backend.add_lane(pi_facts);
        let mut server = server_with_backend(backend.clone(), root.path().to_path_buf());
        let binding: SessionBinding = "v1:fake:9:300".parse().unwrap();
        server.round_timeout = Duration::from_secs(30);
        let (server, writer, cancellations, workers) = scripted_harness(server);

        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &request(
                1,
                "tools/call",
                json!({ "name": "session_submit", "arguments": { "session": "v1:fake:9:300", "task": "implement the bounded change" } }),
            ),
        );
        let submitted = wait_for_responses(&writer, 1, Duration::from_secs(2));
        assert_eq!(
            submitted[0]["result"]["isError"], false,
            "submit failed: {submitted:?}"
        );

        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &request(
                6,
                "tools/call",
                json!({ "name": "session_bridge", "arguments": { "session": "v1:fake:9:300" } }),
            ),
        );
        let started = Instant::now();
        let deadline = started + Duration::from_secs(5);
        while server.pending.owner_pid(&binding).is_none() {
            assert!(
                Instant::now() < deadline,
                "the bridge never attached to the round"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        feed_line(
            &server,
            &writer,
            &cancellations,
            &workers,
            &json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": 6 },
            }),
        );
        let responses = wait_for_responses(&writer, 2, Duration::from_secs(3));
        assert_eq!(responses[1]["id"], 6, "{responses:?}");
        assert_eq!(responses[1]["result"]["isError"], true);
        assert!(
            responses[1]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("cancelled"),
            "{responses:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        let round = server.pending.pending_round(&binding).unwrap().unwrap();
        assert!(!round.completed);
        assert!(server.pending.owner_pid(&binding).is_none());
        drain_workers(&workers);
    }
}
