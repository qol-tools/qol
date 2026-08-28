# Phase 5 spec: fold `qol sessions mcp` onto the shared MCP lib

Status: architect contract for the `sessions-mcp-fold` lane. Source plan: `docs/research/qol-memory/interface-plan.md` section 2 row 5 and section 8 item 9. Facts from the 2026-08-28 scout of `tools/qol-cli/src/commands/sessions/` and `libs/qol-mcp`.

Rules: edit only the owned paths; never run build, test, lint, format or git commands; add no code comments; never use the em-dash character anywhere. Report changed files and lines plus conscious deviations, nothing else.

## 1. Decision

Phase 5 has two halves. The first, folding the JSON-RPC protocol layer of `qol sessions mcp` onto `libs/qol-mcp`, is delivered here. The second, serving the sessions tools from the tray `/api/mcp` endpoint, is closed as not viable and stays out:

- `session_bridge` blocks inside the call for up to 24 h (`round_timeout = bridge::TIMEOUT_MAX_MS`); the tray handles a tool call inside one `spawn_blocking` request and every HTTP MCP client applies a request timeout, so a hosted bridge would time out or pin tokio blocking threads for hours.
- The owner identity (`watch_owner.rs:30-48`) is the calling terminal (`terminals.is_current`); the tray is not in any caller's terminal, so every caller would collapse onto one `unknown-<pid>` key and race on `watch-owner-<key>.json` and its watcher child. A per-caller header would require threading an owner token through spawn, bridge, watch and the pi export; that is a redesign of the sessions loop, not a memory-interface change.

The stdio server stays the transport for Claude, Codex and kimi; pi keeps the generated tool extension. Nothing in qol-skills changes.

## 2. Ownership

Lane `sessions-mcp-fold`: `tools/qol-cli/src/commands/sessions/mcp.rs`, `tools/qol-cli/src/commands/sessions/contract.rs`, `tools/qol-cli/Cargo.toml`. The lane works in the `qm-flow` worktree.

## 3. Changes

### 3.1 `tools/qol-cli/Cargo.toml`

Add `qol-mcp.workspace = true` under `[dependencies]` (the workspace declares `qol-mcp = { path = "libs/qol-mcp" }` at root Cargo.toml:39).

### 3.2 `contract.rs`

Add next to `tool_specs()`:

```rust
pub(crate) fn mcp_tool_specs() -> Vec<qol_mcp::ToolSpec>
```

Maps every `ToolSpec { name, description, input_schema, .. }` to `qol_mcp::ToolSpec { name: name.to_string(), description: description.to_string(), input_schema }`. `label` stays on the local struct for the pi export. Existing tests stay; add `mcp_tool_specs_keeps_order_and_schemas`.

### 3.3 `mcp.rs`

- Delete the local constants `ERROR_PARSE`, `ERROR_INVALID_REQUEST`, `ERROR_METHOD_NOT_FOUND`, `ERROR_INVALID_PARAMS` (lines 17-20) and the local `result` (976-978) and `error` (980-986) helpers; use `qol_mcp::jsonrpc::{result_response, error_response, ErrorCode}` at every former call site (`ErrorCode::ParseError`, `InvalidRequest`, `MethodNotFound`, `InvalidParams`). `error_response` takes the id as a `Value`; pass `Value::Null` where the old helper took `None`.
- Delete `PROTOCOL_VERSION` (line 14). Implement:

```rust
impl qol_mcp::ToolHost for McpSessionServer {
    fn server_info(&self) -> qol_mcp::ServerInfo
    fn list(&self) -> Vec<qol_mcp::ToolSpec>
    fn call(&self, name: &str, arguments: serde_json::Value) -> qol_mcp::ToolResult
}
```

`server_info` returns `SERVER_NAME` and `env!("CARGO_PKG_VERSION")`; `list` returns `contract::mcp_tool_specs()`; `call` runs the existing per-tool dispatch (the body of `call_tool_with_cancel` after its name lookup) with no cancellation flag and wraps `Ok(text)` as `qol_mcp::ToolResult::text(text)` and `Err(message)` as `qol_mcp::ToolResult::error(message)`.

- `handle()` (148-181) delegates every method except `tools/call` to `qol_mcp::handle(self, message)`: `initialize` (protocol version now negotiated from `qol_mcp::PROTOCOL_VERSIONS`, falling back to `LATEST_PROTOCOL_VERSION`), `ping`, `tools/list`, the `notifications/*` methods (a `None` from the shared handler means no response is written) and unknown methods (`MethodNotFound`). `tools/call` keeps the worker-thread path in `dispatch_line` (789-845) with its cancellation map and progress notifications, and `call_tool_with_cancel` keeps producing the response through `result_response(id, serde_json::to_value(ToolResult)?)` so the wire shape stays `{"content":[{"type":"text","text":...}],"isError":...}`.
- Keep unchanged: the stdin loop, `write_response`, `parse_json_line` (its parse error now uses `ErrorCode::ParseError`), the watcher lifecycle, all tool functions, and every constant not named above.
- Existing tests in `mcp.rs` keep passing with at most these edits: an assertion on the exact error code constant becomes the `ErrorCode::X.code()` value; an `initialize` assertion expecting the old echoed prefix behavior expects the shared negotiation instead. Add `initialize_negotiates_protocol_through_qol_mcp` (requesting `2024-11-05` echoes it; requesting `1999-01-01` yields `LATEST_PROTOCOL_VERSION`) and `tools_list_through_shared_handler_matches_contract`.

## 4. Gate and acceptance (architect)

1. `cargo fmt --all --check`; `cargo clippy -p qol --all-targets -- -D warnings` on host, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`; `cargo test -p qol` (sessions tests included).
2. Stdio smoke against the freshly built binary: `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | target/debug/qol sessions mcp` answers both lines; the tools list matches `qol sessions mcp` from the installed binary.
3. Squash to main as one commit (`refactor(qol-cli): serve qol sessions mcp through qol-mcp`) with explicit paths; no push; the installed `qol` is not reinstalled by this phase.
