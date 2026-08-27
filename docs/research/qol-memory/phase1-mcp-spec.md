# Phase 1 spec: host MCP endpoint and agent_tool contract flag

Status: architect spec, 2026-08-27. Implements section 3 of `interface-plan.md`.
Worktree: `/media/kmrh47/WD_SN850X/Git/worktrees/qol-mcp/qol-monorepo`, branch `qol-mcp`.
Every path below is relative to that worktree root.

## 1. Goal

qol-tray serves one stateless Streamable HTTP MCP endpoint at `POST /api/mcp` behind the existing token auth.
It exposes every plugin query or action whose runtime contract entry says `agent_tool = true` as an MCP tool named `<plugin_id>__<runable>`.
The `qol` CLI prints the URL, the token, a headers JSON object, and can write the harness config entry.
No per-session MCP process exists anywhere after this phase.

## 2. Lanes and ownership

Four lanes run in parallel. Two lanes never touch the same file.

| Lane | Role | Owned paths |
|---|---|---|
| `mcp-lib` | implement | `libs/qol-mcp/**` (new crate), root `Cargo.toml` (one line under `[workspace.dependencies]`) |
| `mcp-contract` | implement | `libs/qol-config/src/contract/runtime.rs`, `docs/plugin-contract.md` |
| `mcp-host` | implement | `apps/qol-tray/src/features/mcp/**` (new), `apps/qol-tray/src/features/mod.rs`, `apps/qol-tray/src/features/plugin_store/server/mod.rs`, `apps/qol-tray/src/plugins/action_executor/mod.rs`, `apps/qol-tray/src/plugins/action_transport/mod.rs`, `apps/qol-tray/Cargo.toml` |
| `mcp-cli` | implement | `tools/qol-cli/src/commands/mcp/**` (new), `tools/qol-cli/src/commands/mod.rs`, `tools/qol-cli/src/main.rs`, `tools/qol-cli/src/cli/contract.rs` |

`Cargo.lock` is never edited by a lane; the architect regenerates it at the gate.

Lane rules, unconditional: edit only owned paths; never run build, test, lint, format, or git commands (no cargo build, cargo test, cargo clippy, cargo fmt, no commit, stage, stash, or push); add no code comments; never use the em-dash character anywhere; no `#[allow(...)]` attributes; no `unwrap()` outside tests.
Lane report shape: the list of files changed with line ranges, plus any conscious deviation from this spec with one sentence of reason, and nothing else.

The host and CLI lanes write against the signatures in sections 3 and 4 exactly, even though the crate and fields do not exist yet in their view; the architect compiles the whole round once.

## 3. Crate `libs/qol-mcp` (lane `mcp-lib`)

Transport-agnostic MCP tool protocol. No tokio, no axum, no I/O.

### 3.1 `libs/qol-mcp/Cargo.toml`

```toml
[package]
name = "qol-mcp"
version = "0.1.0"
edition = "2021"
description = "Transport-agnostic JSON-RPC handler for the MCP tools surface"
license = "PolyForm-Noncommercial-1.0.0"

[dependencies]
indexmap = { version = "2.12", features = ["serde"] }
serde.workspace = true
serde_json.workspace = true
```

Root `Cargo.toml`: add `qol-mcp = { path = "libs/qol-mcp" }` under `[workspace.dependencies]` next to the other internal crates (alphabetical position after `qol-hotkeys`). The `members` glob already includes `libs/*`.

### 3.2 Layout

```
libs/qol-mcp/src/
  lib.rs        re-exports only
  jsonrpc.rs    message shapes and error codes
  tool.rs       ToolSpec, Content, ToolResult, input_schema
  handler.rs    ToolHost trait and handle()
```

`lib.rs`:

```rust
pub mod handler;
pub mod jsonrpc;
pub mod tool;

pub use handler::{handle, ServerInfo, ToolHost};
pub use jsonrpc::{ErrorCode, LATEST_PROTOCOL_VERSION, PROTOCOL_VERSIONS};
pub use tool::{input_schema, Content, ToolResult, ToolSpec};
```

### 3.3 `jsonrpc.rs`

```rust
pub const PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];
pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
}

impl ErrorCode {
    pub fn code(self) -> i64   // -32700, -32600, -32601, -32602, -32603
}

pub fn result_response(id: serde_json::Value, result: serde_json::Value) -> serde_json::Value
pub fn error_response(id: serde_json::Value, code: ErrorCode, message: impl Into<String>) -> serde_json::Value
```

Responses are `{"jsonrpc":"2.0","id":<id>,"result":...}` or `{"jsonrpc":"2.0","id":<id>,"error":{"code":<n>,"message":"..."}}`.

### 3.4 `tool.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolResult {
    pub content: Vec<Content>,
    #[serde(rename = "structuredContent", skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    #[serde(rename = "isError")]
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self            // one text block, no structured, is_error false
    pub fn structured(value: serde_json::Value) -> Self      // content = [text(serde_json::to_string_pretty(&value))], structured = Some(value)
    pub fn error(message: impl Into<String>) -> Self         // one text block, is_error true
}

pub fn input_schema(params: &indexmap::IndexMap<String, String>) -> serde_json::Value
```

`input_schema` returns `{"type":"object","properties":{<name>:{"type":"string","description":<description>}},"required":[<every name in map order>]}`; an empty map yields `{"type":"object","properties":{},"required":[]}`.

### 3.5 `handler.rs`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInfo {
    pub name: String,
    pub version: String,
}

pub trait ToolHost {
    fn server_info(&self) -> ServerInfo;
    fn list(&self) -> Vec<ToolSpec>;
    fn call(&self, name: &str, arguments: serde_json::Value) -> ToolResult;
}

pub fn handle(host: &dyn ToolHost, message: serde_json::Value) -> Option<serde_json::Value>
```

`handle` rules, in order:

1. A JSON array (batch) or a non-object returns `Some(error_response(Null, InvalidRequest, ...))`.
2. An object without `method` but with `result` or `error` is a client response: return `None`.
3. An object without `method` and without `id` returns `None`; without `method` but with `id` returns `InvalidRequest` for that id.
4. An object with `method` and no `id` is a notification: return `None` for every method, including unknown ones.
5. `initialize`: read `params.protocolVersion`; if it is one of `PROTOCOL_VERSIONS` echo it, else use `LATEST_PROTOCOL_VERSION`. Result: `{"protocolVersion":..., "capabilities":{"tools":{"listChanged":false}}, "serverInfo":{"name":<info.name>,"version":<info.version>}}`.
6. `ping`: result `{}`.
7. `tools/list`: result `{"tools":[<ToolSpec>...]}` from `host.list()`.
8. `tools/call`: `params.name` missing or not a string returns `InvalidParams`; a name that `host.list()` does not contain returns `InvalidParams` with message `unknown tool: <name>`; otherwise `params.arguments` (default `{}`) is passed to `host.call` and the `ToolResult` is the result value.
9. Any other method returns `MethodNotFound`.

### 3.6 Tests (inside each module, `#[cfg(test)]`)

A `FakeHost` with two tools (`echo` returning `ToolResult::structured(arguments)`, `fail` returning `ToolResult::error("boom")`).
Cover every rule in 3.5, both branches of `initialize`, the serialized shape of `ToolSpec` (`inputSchema` key), `ToolResult` (`structuredContent` omitted when None, `isError` always present), `input_schema` with two parameters and with zero, `ErrorCode::code` values, and one round trip `initialize -> notifications/initialized -> tools/list -> tools/call echo` asserting the exact JSON of each response.

## 4. Contract change `libs/qol-config` (lane `mcp-contract`)

### 4.1 `libs/qol-config/src/contract/runtime.rs`

```rust
pub struct ActionSpec {
    pub description: String,
    #[serde(default)]
    pub confirm: Option<String>,
    #[serde(default)]
    pub input: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub agent_tool: bool,
    #[serde(default)]
    pub tool_description: Option<String>,
}

pub struct QuerySpec {
    pub description: String,
    pub poll_interval_ms: u64,
    #[serde(default)]
    pub agent_tool: bool,
    #[serde(default)]
    pub tool_description: Option<String>,
    #[serde(default)]
    pub input: Option<IndexMap<String, String>>,
}
```

Add to both structs:

```rust
pub fn tool_description(&self) -> &str   // tool_description if Some and non-blank, else description
```

Validation: `validate_runtime_spec` calls a new `validate_agent_tools(spec)` after `validate_initial_queries`. For every action and query with `agent_tool == true`:

- `tool_description()` trimmed must be non-empty, else `Validation(format!("agent tool {kind} {name} needs a description"))`.
- every key of `input` (when present) must satisfy `is_valid_runable_name`, else `Validation(format!("agent tool {kind} {name} has invalid input name: {param}"))`.

Entries with `agent_tool == false` are not validated further; existing behaviour is unchanged for them.

Tests to add in the existing `tests` module: a runtime with a flagged query carrying two inputs parses and exposes the fields; `agent_tool = true` with blank description and blank tool_description fails with the message above; an input key `Bad-Name` fails with the invalid input name message; a query without the new keys still parses with `agent_tool == false`, `tool_description == None`, `input == None`; `tool_description()` prefers the override and falls back to `description`.

### 4.2 `docs/plugin-contract.md` section 2.3

Replace the two bullets for `[action.<name>]` and `[query.<name>]` with:

- `[action.<name>]`: `description`, optional `confirm`, optional `input` map, optional `agent_tool` (bool), optional `tool_description`.
- `[query.<name>]`: `description`, `poll_interval_ms`, optional `agent_tool` (bool), optional `tool_description`, optional `input` map.

Add one paragraph after the names bullet: an agent tool is any query or action with `agent_tool = true`; the host exposes it on its MCP endpoint at `/api/mcp` as `<plugin_id>__<name>` with `tool_description` (falling back to `description`) and a JSON schema built from the `input` map, where every parameter is a required string. Flagged entries must have a non-empty description and input names matching the runable name charset.

## 5. Host endpoint `apps/qol-tray` (lane `mcp-host`)

### 5.1 Dependencies

`apps/qol-tray/Cargo.toml`: add `qol-mcp.workspace = true` next to `qol-config.workspace = true`.

### 5.2 Transport and dispatcher

`apps/qol-tray/src/plugins/action_transport/mod.rs`: add

```rust
pub fn dispatch_daemon_action_with_input_and_timeout(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch
```

delegating to the private `dispatch_daemon_action_request`.

`apps/qol-tray/src/plugins/action_executor/mod.rs`:

```rust
pub const MCP_DISPATCH_TIMEOUT: Duration = Duration::from_secs(10);

pub fn dispatch_query_with_input(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &str,
    query_name: &str,
    input: serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, ActionExecutionError>
```

Move the body of `dispatch_query` into `dispatch_query_with_input` (same initial dispatch, readiness recovery, and retry, using `dispatch_daemon_action_with_input_and_timeout` with the given input and timeout); `dispatch_query` becomes a one-line call with `serde_json::Value::Null` and `QUERY_DISPATCH_TIMEOUT`. Existing trace calls stay.

### 5.3 Feature module `apps/qol-tray/src/features/mcp/`

```
mod.rs        pub fn router(plugin_manager: Arc<Mutex<PluginManager>>) -> axum::Router
handlers.rs   axum handlers and router_with_host
tool_host.rs  PluginToolHost implementing qol_mcp::ToolHost
```

`apps/qol-tray/src/features/mod.rs`: add `pub mod mcp;` in alphabetical position.

`mod.rs`:

```rust
pub fn router(plugin_manager: Arc<Mutex<PluginManager>>) -> Router {
    handlers::router_with_host(Arc::new(tool_host::PluginToolHost::new(plugin_manager)))
}
```

`handlers.rs`:

```rust
pub(super) type SharedHost = Arc<dyn qol_mcp::ToolHost + Send + Sync>;

pub(super) fn router_with_host(host: SharedHost) -> Router {
    Router::new()
        .route("/", post(post_message).get(reject_get).delete(end_session))
        .with_state(host)
}
```

- `post_message(State(host), body: axum::body::Bytes) -> Response`: parse with `serde_json::from_slice`; on failure respond `400` with body `qol_mcp::jsonrpc::error_response(Null, ParseError, <error text>)` as JSON. Otherwise run `qol_mcp::handle(host.as_ref(), message)` inside `tokio::task::spawn_blocking` (tool calls block on the daemon socket for up to 10 s). `Some(value)` responds `200` `application/json` with the value; `None` responds `202` with an empty body. A join error responds `500` with `error_response(Null, InternalError, ...)`.
- `reject_get` responds `405`.
- `end_session` responds `204`.

`tool_host.rs`:

```rust
pub(super) struct PluginToolHost {
    plugin_manager: Arc<Mutex<PluginManager>>,
}

impl PluginToolHost {
    pub(super) fn new(plugin_manager: Arc<Mutex<PluginManager>>) -> Self
}

struct ToolBinding {
    plugin_id: String,
    runable: String,
    kind: RunableKind,      // enum RunableKind { Query, Action }
    spec: qol_mcp::ToolSpec,
}

fn tool_name(plugin_id: &str, runable: &str) -> String            // format!("{plugin_id}__{runable}")
fn bindings(plugin_manager: &Arc<Mutex<PluginManager>>) -> Vec<ToolBinding>
```

`bindings`: lock the manager, iterate `manager.plugins()`, for each plugin call `crate::plugins::config::load_runable_contract_from_root(&plugin.path)`; `Err` or `Ok(None)` skips the plugin (trace `contract_skipped`); for every query with `agent_tool` push a Query binding and for every action with `agent_tool` push an Action binding, `spec.description = entry.tool_description().to_string()`, `spec.input_schema = qol_mcp::input_schema(entry.input.as_ref().unwrap_or(&IndexMap::new()))`. Release the lock before returning. The plugin id string comes from `plugin.id` (use its existing string accessor or `to_string()`).

`impl qol_mcp::ToolHost for PluginToolHost`:

- `server_info`: name `qol-tray`, version `env!("CARGO_PKG_VERSION")`.
- `list`: `bindings(...)` mapped to specs; trace `tools_listed count=<n>`.
- `call`: find the binding by name (rebuild `bindings` for the lookup, then drop it); missing returns `ToolResult::error("unknown tool: <name>")`. Query: `dispatch_query_with_input(&self.plugin_manager, &plugin_id, &runable, arguments, MCP_DISPATCH_TIMEOUT)`; `Ok(value)` becomes `ToolResult::structured(value)`, `Err(error)` becomes `ToolResult::error(error.to_string())`. Action: `try_execute_action_with_input_result(&self.plugin_manager, &plugin_id, &runable, arguments)`; `Ok(Some(value))` becomes `ToolResult::structured(value)`, `Ok(None)` becomes `ToolResult::structured(json!({"status":"ok"}))`, `Err` becomes `ToolResult::error`. Trace `tool_called plugin=<id> runable=<name> kind=<query|action>` before dispatch and `tool_failed plugin=<id> runable=<name> error=<text>` on error.

Traces use `qol_runtime::probe!("TRAY_MCP", "event=<name> ...")`, the same macro `action_executor/mod.rs` uses; it is release-safe.

### 5.4 Mounting

`apps/qol-tray/src/features/plugin_store/server/mod.rs`, in `assemble_app`, before `api_router` consumes `app_state`:

```rust
let mcp = super::super::mcp::router(app_state.plugin_manager.clone()).layer(
    middleware::from_fn_with_state(http_security.clone(), security::require_api_access),
);
```

and add `.nest("/api/mcp", mcp)` right after `.nest("/api/task-runner", task_runner)`. Nothing else in that file changes.

### 5.5 Tests

`handlers.rs` tests with a `FakeHost` (one `echo` tool) and `tower::ServiceExt::oneshot` (tower with `util` is already a dev-dependency):

- POST `initialize` returns 200 and a body whose `result.protocolVersion` equals the request's version.
- POST `notifications/initialized` returns 202 with an empty body.
- POST invalid JSON returns 400 and a `-32700` error body.
- GET returns 405; DELETE returns 204.

`tool_host.rs` tests: `tool_name` joins with a double underscore; a `ToolBinding` built from a parsed `RuntimeSpec` string (use `qol_config::contract::parse_runtime_spec_str`) exposes the flagged query only, with the override description and the generated schema. Extract the per-contract collection into `fn bindings_for_contract(plugin_id: &str, runtime: &RuntimeSpec) -> Vec<ToolBinding>` so this test needs no `PluginManager`.

## 6. CLI `tools/qol-cli` (lane `mcp-cli`)

### 6.1 Commands

`qol mcp url` prints `http://<LOCAL_HOST>:<DEFAULT_PORT>/api/mcp` and a newline.
`qol mcp token` prints the tray token and a newline.
`qol mcp headers` prints `{"<HTTP_AUTH_HEADER>":"<token>"}` and a newline (compact JSON).
`qol mcp configure <claude|codex|pi|kimi>` writes or replaces the `qol` MCP entry in that harness's user config, prints `updated <path>` followed by the entry it wrote, and exits non-zero naming the path when the config file does not exist.
`qol mcp help`, `qol mcp`, and unknown subcommands print the usage and, for unknown, fail.

All literals come from `qol_conventions::{DEFAULT_PORT, LOCAL_HOST, HTTP_AUTH_HEADER}`; the port number and header name are never spelled out in code or tests (a literal guard forbids it), so tests build expected strings from the constants.

Token source: `qol_config::http_auth_token_path()` read with `std::fs::read_to_string`, trimmed; a missing path or file fails with `qol-tray HTTP token not found at <path>; start qol-tray first`.

### 6.2 Layout

```
tools/qol-cli/src/commands/mcp/
  mod.rs        pub(crate) fn run(args: &[OsString]) -> Result<()>, subcommand dispatch, usage text, url and token and headers
  configure.rs  harness entry builders and file updates
```

`configure.rs` pure functions, each unit tested on in-memory strings:

```rust
pub(crate) enum Harness { Claude, Codex, Pi, Kimi }
impl Harness { pub(crate) fn parse(name: &str) -> Option<Self>; pub(crate) fn config_path(&self) -> Result<PathBuf>; }

pub(crate) fn json_entry(harness: Harness, url: &str, token: &str) -> serde_json::Value
pub(crate) fn apply_json_entry(document: &str, entry: serde_json::Value) -> Result<String>
pub(crate) fn apply_codex_entry(document: &str, url: &str, token: &str) -> Result<String>
```

Entries:

- Claude (`~/.claude.json`, key `mcpServers.qol`): `{"type":"http","url":<url>,"headersHelper":"qol mcp headers"}`.
- pi (`~/.pi/agent/mcp.json`, key `mcpServers.qol`): `{"url":<url>,"headers":{<HTTP_AUTH_HEADER>:"!qol mcp token"}}`.
- kimi (`$KIMI_CODE_HOME/mcp.json`, else `~/.kimi-code/mcp.json`, key `mcpServers.qol`): `{"url":<url>,"headers":{<HTTP_AUTH_HEADER>:<token>}}`.
- Codex (`~/.codex/config.toml`, table `[mcp_servers.qol]`): `url = <url>` and `http_headers = { <HTTP_AUTH_HEADER> = <token> }`, edited with `toml_edit::DocumentMut` so every other table and comment survives.

`apply_json_entry` parses the document with `serde_json`, creates `mcpServers` when absent, replaces `mcpServers.qol`, and serializes with `serde_json::to_string_pretty` plus a trailing newline. Home directories come from `dirs::home_dir()` (already a dependency of qol-config; if `dirs` is not a direct qol-cli dependency, use `std::env::var_os("HOME")` and fail when unset).

### 6.3 Registration

- `tools/qol-cli/src/commands/mod.rs`: `pub(crate) mod mcp;` in alphabetical position.
- `tools/qol-cli/src/main.rs`: `"mcp" => commands::mcp::run(rest),` next to the `sessions` arm.
- `tools/qol-cli/src/cli/contract.rs`: add a `command("mcp", ...)` entry with subcommands `url`, `token`, `headers`, `configure` so `qol help mcp` and `qol mcp help` render through the contract like `sessions` does; no `run_json` handlers (no `--json` support in this phase).

Tests: `Harness::parse` accepts the four names and rejects others; `json_entry` for each harness matches the shapes above; `apply_json_entry` on `{}` and on a document with an existing unrelated server preserves that server and replaces `qol`; `apply_codex_entry` on a document with another `[mcp_servers.other]` table and a top-level comment keeps both and adds `[mcp_servers.qol]`; the url helper output equals a string built from the constants.

## 7. Gate (architect only)

`cargo fmt --all --check`; `cargo clippy -p qol-mcp -p qol-config -p qol-tray -p qol --all-targets -- -D warnings`; `cargo test -p qol-mcp -p qol-config -p qol-tray -p qol`; `cargo clippy -p qol-mcp -p qol-config --target x86_64-apple-darwin` and `--target x86_64-pc-windows-msvc`; `qol check`.

## 8. Acceptance

1. `curl -s -X POST -H "x-qol-token: $(qol mcp token)" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' http://127.0.0.1:42700/api/mcp` returns `result.protocolVersion == "2025-06-18"` and `serverInfo.name == "qol-tray"`.
2. `tools/list` returns exactly the flagged runables across installed plugins, with `inputSchema` built from their `input` maps.
3. `tools/call` on a flagged query reaches the daemon with `input` populated and returns `structuredContent`.
4. A request without the token gets `401`; GET gets `405`; DELETE gets `204`.
5. `qol mcp configure claude` writes the entry and a fresh Claude Code session lists `mcp__qol__*` tools with no extra process in `ps`.
