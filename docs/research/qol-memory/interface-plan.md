# qol-memory interface plan

Status: architect plan, 2026-08-27, grounded in the grouped scout `qm-plan-scout` (five read-only lanes over the tray server, the plugin contract, the launcher, the memory plugin, and the harness MCP surface).
Scope: everything after the MVP in `plugin-mvp-scope.md`.
Nothing in this document is implemented yet.

## 1. Shape

qol-memory is an engine with one contract; every human or agent surface is an adapter over it.
An AI harness is one consumer strategy, not a prerequisite.

Three human interfaces:

- CLI: `qol-memory ask|status|doctor` today, daemon-backed verbs in stage 2.
- Launcher: a `flow` entry kind that keeps the launcher open and routes typed text to the plugin.
- Agents: one Streamable HTTP MCP endpoint served by qol-tray, no per-session server process.

Two adapter families, both declared in manifests and mediated by the host:

- Sources feed units: harness transcripts (claude, pi), live capture from the pi extension, later notes and decisions.
- Consumers pull context: terminal, launcher, harness hook, MCP tool.

The engine never links to a harness, the launcher, or MCP code.

## 2. Phases in dependency order

| Phase | Deliverable | Repos | Depends on |
|---|---|---|---|
| 1 | Host MCP endpoint plus `agent_tool` contract flag | monorepo | none |
| 2 | qol-memory stage 2 daemon: watcher, ingest, warm index, socket queries | monorepo | 1 for tool exposure only |
| 3 | Launcher `flow` entry kind plus `[launcher]` manifest section | monorepo | 1 (query input transport) |
| 4 | Harness bridge consolidation: `qol-memory` plugin in qol-skills, http MCP entries | qol-skills | 1, 2 |
| 5 | Fold `qol sessions mcp` onto the shared MCP lib and host endpoint | monorepo, qol-skills | 1, owner identity decision |

Phases 1 and 2 can run as parallel worktrees; their file sets are disjoint.
Phase 3 starts after phase 1 lands because flows reuse the query-with-input transport.
Phase 5 is optional and blocked on a design decision (section 8, item 9).

## 3. Phase 1: host MCP endpoint

### 3.1 Facts the design rests on

- The tray is axum 0.8 bound to `127.0.0.1:42700` (`apps/qol-tray/src/features/plugin_store/server/mod.rs:56-97`, `libs/qol-conventions/src/lib.rs:8`).
- Token auth is `require_api_access` (`server/security.rs:47-62`): header `x-qol-token` or cookie fragment, applied to the `/api` nest; `require_local_host` wraps the whole app.
- Queries dispatch through `action_executor::dispatch_query` (`apps/qol-tray/src/plugins/action_executor/mod.rs:210-250`) with no input and a 750 ms timeout; actions go through `execute_action_with_input`.
- The daemon wire type already carries input: `DaemonRequest { action, input: Value }` (`libs/qol-runtime/src/protocol.rs:10-16`), one JSON line per request over the plugin socket.
- Query and action specs live in `libs/qol-config/src/contract/runtime.rs` (`QuerySpec` :33, `ActionSpec` :24); unknown fields are ignored, so new optional fields are backward compatible.
- No JSON-RPC or MCP code exists in `apps/` or `libs/`; `tools/qol-cli/src/commands/sessions/mcp.rs` hand-rolls JSON-RPC 2.0 over stdio with tool specs in `contract.rs`.
- Harness support for remote HTTP MCP: Claude Code (`type: "http"`, `url`, `headers` or `headersHelper`), Codex (`[mcp_servers.<name>] url`, `http_headers`, `env_http_headers`, `bearer_token_env_var`), pi via `pi-mcp-adapter` (`url`, `headers` with `!command` values, `requestHeadersCommand`), kimi (`url`, `headers`, `bearerTokenEnvVar`).

### 3.2 Contract change

File: `libs/qol-config/src/contract/runtime.rs`.

- `QuerySpec` gains `agent_tool: bool` (default false), `tool_description: Option<String>`, `input: Option<IndexMap<String, String>>` (same shape as `ActionSpec.input`: parameter name to description).
- `ActionSpec` gains `agent_tool: bool` and `tool_description: Option<String>`.
- Validation in `validate_runtime_spec`: `agent_tool = true` requires a non-empty description; input parameter names must satisfy `is_valid_runable_name`.
- Docs: `docs/plugin-contract.md` section 2.3 gains the three fields and the sentence "an agent tool is any query or action with `agent_tool = true`; the host exposes it on its MCP endpoint".

The exposure gate is the flag on the runable, not a `[capabilities]` key; capabilities stay for host-side surfaces the plugin must support.

### 3.3 Shared protocol lib

New crate `libs/qol-mcp`, transport-agnostic, no tokio, no axum.

- `jsonrpc`: `Request`, `Response`, `Notification`, `ErrorCode` (`-32700`, `-32600`, `-32601`, `-32602`), one message per value.
- `ToolSpec { name, description, input_schema: Value }` and `ToolResult { content: Vec<Content>, structured: Option<Value>, is_error: bool }`.
- `trait ToolHost { fn list(&self) -> Vec<ToolSpec>; fn call(&self, name: &str, args: Value) -> ToolResult; }`.
- `fn handle(host: &dyn ToolHost, message: Value) -> Option<Value>` routing `initialize`, `ping`, `tools/list`, `tools/call`, and swallowing `notifications/*`; `initialize` echoes a supported protocol version from the set `{2024-11-05, 2025-03-26, 2025-06-18}` and advertises `tools.listChanged = false`.
- `fn input_schema(params: &IndexMap<String, String>) -> Value` producing `{type: object, properties: {name: {type: string, description}}, required: [all]}`.
- Unit tests for every method and error path, plus a fixture round-trip of `initialize -> tools/list -> tools/call`.

`qol sessions mcp` is not migrated in this phase (section 8, item 9).

### 3.4 Host endpoint

New feature module `apps/qol-tray/src/features/mcp/` following the feature-router pattern (`features/task_runner/mod.rs:11`), nested at `/api/mcp` inside `assemble_app` (`server/mod.rs:344-350`) with `require_api_access`.

- `POST /api/mcp`: parse the JSON-RPC body, call `qol_mcp::handle`, respond `application/json`; notifications and responses from the client return `202` with an empty body.
- `GET /api/mcp`: `405`; the host never opens server-initiated streams in this phase.
- `DELETE /api/mcp`: `204`; the server is stateless and issues no `Mcp-Session-Id`.
- `ToolHost` impl: enumerate installed plugins from `AppState.plugin_manager`, load each runable contract (`load_runable_contract`), collect every `agent_tool` runable as `ToolSpec { name: "<plugin_id>__<runable>", description: tool_description or description, input_schema }`.
- `call`: split the name on `__`, then dispatch a query with `dispatch_query_with_input(plugin, name, args, MCP_DISPATCH_TIMEOUT)` (new function beside `dispatch_query`, timeout 10 s) or an action with `execute_action_with_input`; `Handled { data }` becomes `structured = data` and `content = [text(JSON pretty)]`; `Fallback` and errors become `is_error = true` with the message.
- Trace target `TRAY_MCP` with events `tools_listed`, `tool_called`, `tool_failed`.

### 3.5 CLI helpers

New command family in `tools/qol-cli/src/commands/mcp/` (disjoint from `commands/sessions/`):

- `qol mcp url`: prints `http://127.0.0.1:<port>/api/mcp` using the stable port.
- `qol mcp token`: prints the tray token from the token file (`qol_plugin_api::host_exec::read_auth_token`).
- `qol mcp headers`: prints `{"x-qol-token": "<token>"}` for Claude `headersHelper` and pi `requestHeadersCommand`.
- `qol mcp configure <claude|codex|pi|kimi>`: writes or updates the harness entry named `qol` (Claude and pi use the helper command; Codex and kimi get a static `x-qol-token` header value because they have no helper facility); prints the diff and exits non-zero when the harness config is missing.

### 3.6 Gate and acceptance

Gate: `cargo fmt --check`, `cargo clippy --all-targets -D warnings` for `qol-mcp`, `qol-config`, `qol-tray`, `qol`; `cargo test -p qol-mcp -p qol-config -p qol-tray -p qol`; cross-target clippy for `qol-mcp` and `qol-config` on macOS and Windows targets; `qol check`.

Acceptance:

1. `curl -X POST -H "x-qol-token: $(qol mcp token)" -d '<initialize>' http://127.0.0.1:42700/api/mcp` returns a valid `initialize` result.
2. `tools/list` lists exactly the runables flagged `agent_tool` across installed plugins with generated schemas.
3. `tools/call` on a flagged query with input reaches the plugin daemon with `input` populated and returns `structuredContent`.
4. A request without the token gets `401`; a request with a foreign `Host` header is rejected.
5. Claude Code with the `qol mcp configure claude` entry lists the tools under `mcp__qol__*` with zero extra processes in `ps`.

## 4. Phase 2: qol-memory stage 2 daemon

### 4.1 Facts the design rests on

- The Rust crate is read-only apart from `idx-*.json(.meta)` and `retrievals.jsonl`; `ask::run` (`src/ask/mod.rs:276`) and `status` (:1020) are pure functions over `Store`.
- Write path is JS: `snapshot.mjs` walks `~/.pi/agent/sessions` and `~/.claude/projects` (`snapshot.mjs:16-17`), `lib/merge.js` dedupes by `unitKey` and rewrites `units.jsonl` sealed, `notes.mjs` is a pure trigger extractor, `decisions.mjs` is the only LLM consumer.
- Detection is not incremental: every run re-walks and re-parses; `ingest.jsonl` records size, mtime, and sha256 per file.
- The pi extension appends live units to `units.jsonl` itself and triggers `decisions.mjs --live` on compaction with a 15 min debounce.
- The daemon pattern to copy is qol-voice for stateful request handling (`run_stateful_request_listener`, `plugins/qol-voice/src/app/mod.rs:37-58`) and cli-sessions for lifecycle (`plugins/cli-sessions/plugin.toml:34-38`, `daemon/actions.rs:5-8`, socket via `QOL_TRAY_DAEMON_SOCKET`).
- No surveyed daemon implements idle exit; the host-death watchdog and `kill` are the lifecycle contract.

### 4.2 Manifest and contract

`plugins/qol-memory/plugin.toml`:

- `[daemon] enabled = true, command = "qol-memory", socket = "/tmp/qol-memory.sock"`.
- `[capabilities] doctor = true` unchanged; no gpui.

New `plugins/qol-memory/qol-runtime.toml`:

- `[query.ask] agent_tool = true`, description "Retrieve settled facts from agent session history", `input = { query = "question in plain words", cwd = "optional working directory for scoping", exclude_session = "optional session id to exclude" }`.
- `[query.status] agent_tool = true`, description "Store size, index freshness, pending candidates".
- `[query.continue]` (not a tool) with `input = { cwd = "...", session = "..." }` returning the units landed since the per-cwd marker, replacing the hook's own store parsing.
- `[action.capture] input = { unit = "one unit as JSON" }`: append one redacted unit; the daemon is the single writer.
- `[action.reindex]`: drop `idx-*` and rebuild.

### 4.3 Crate layout

```
plugins/qol-memory/src/
  app/            daemon lifecycle: listener, request router, warm index cache, shutdown
  ingest/         transcript walkers (claude, pi), unit key, dedupe, sealed rewrite, ingest-state offsets
  watch/          qol-watch subscription over the transcript roots with debounce
  continue_recall/ marker file, gate, delta selection (port of inject-qol-memory-continue.cjs)
```

`cli.rs` gains verbs `run` (daemon), `capture`, `continue`, `reindex`; `ask` and `status` first try the socket and fall back to in-process execution when it is absent, mirroring cli-sessions `open`.

### 4.4 Behaviour

- Startup: build or load the pool, user, and notes indexes through the existing `build_or_load` path so `idx-*.meta` stays parity compatible with the JS side, then hold them in memory.
- Watcher: subscribe to the two transcript roots; on a settled change (250 ms debounce) run incremental ingest for the changed files using per-file byte offsets from `ingest-state.json`; a shrunk or rewritten file falls back to full re-parse with `unitKey` dedupe.
- Ingest appends to `units.jsonl` under the existing `.distill.lock` protocol so JS `mergeStep` and the daemon never interleave; after each append the warm index applies the merge-tail path instead of a rebuild.
- `ask` over the socket answers from the warm index and still appends to `retrievals.jsonl`; the 10 s MCP timeout and the 750 ms web query timeout are both honoured because the warm path is sub-100 ms.
- `continue` computes the same delta the hook computes today and rewrites `continue.marker.json`.
- Distill stays JS and pi-triggered in this phase; the daemon does not spawn `decisions.mjs`.

### 4.5 Gate and acceptance

Gate: `cargo fmt --check`, `cargo clippy -p qol-memory --all-targets -D warnings`, `cargo test -p qol-memory`, `node docs/research/qol-memory/parity.mjs` at 73/73 with 0 mismatches, plugin manifest test, `qol check`.

Acceptance:

1. `qol-memory ask "<q>"` returns byte-identical output with and without the daemon running.
2. Writing a fixture transcript into a temp `QOL_MEMORY_CLAUDE_DIR` yields new units within 2 s of the write and `status` reflects them.
3. Two concurrent `capture` calls plus one JS `mergeStep` leave `units.jsonl` parseable with no duplicate keys.
4. `continue --cwd X --session Y` prints the same lines the current hook prints for the same marker state (fixture comparison).
5. The daemon exits when qol-tray dies and on `kill`.

## 5. Phase 3: launcher flow entries

### 5.1 Facts the design rests on

- Entries are `AppEntry` and `FileEntry` unified by `ResultItem`/`ResultSource` (`plugins/launcher/src/discovery/search.rs:64-76`); ranking ends in `sort_by_score` with `manual_boost` in the score (`search.rs:230-248`).
- State is fields on `LauncherState` (`src/ui/state.rs:43-59`); keys map to `InputEffect` (`src/ui/input.rs:6-14`); Enter runs `launch_selected` (`src/ui/controller.rs:152-194`).
- The host already contributes entries to the launcher: `apps/qol-tray/src/features/launcher_apps/mod.rs` (`LauncherEntry` :12, `sync_entries` :97) delivered through `RuntimeEventKind::LauncherAppsSynced`.
- The launcher reaches the host through `qol_plugin_api::host_exec` (`libs/qol-plugin-api/src/host_exec.rs:38-46`) with the token file.
- Window height adapts per render without an OS resize (`src/ui/render.rs:220-232`).
- A dead `LauncherProviderCapability` marker exists in `libs/qol-plugin-api/src/capability.rs:58` and is consumed nowhere.

### 5.2 Contract change

File: `libs/qol-plugin-api/src/manifest/schema.rs`, new `launcher: Option<LauncherSpec>` on `PluginManifest`.

```toml
[launcher]
kind = "flow"
title = "qol memory"
prompt = "Ask memory"
query = "ask"
row_title = "{fact}"
row_subtitle = "{provenance}"

[[launcher.row_actions]]
label = "Copy"
action = "copy_fact"
```

- `kind = "app"` is the shim: the entry launches the plugin's settings or window exactly as the host app export does today.
- `kind = "flow"` requires `query` to exist in the plugin's runtime contract with an `input` that has a `query` parameter; validated in a new `validate_launcher` called from `PluginManifest::validate()`.
- `row_actions` reuse `RowActionSpec` from `libs/qol-config/src/contract/v1.rs:141-148`.
- `LauncherProviderCapability` is removed in the same change; the manifest section replaces it.
- Docs: `docs/plugin-contract.md` section 2.1.

### 5.3 Host side

- `LauncherEntry` gains `kind: LauncherEntryKind { App, Flow { plugin_id, query, prompt, row_title, row_subtitle, row_actions } }`; `sync_entries` collects flow entries from installed manifests.
- Route `POST /api/plugins/{id}/queries/{query}` with a JSON body reaches `dispatch_query_with_input` from phase 1; no launcher-specific route.

### 5.4 Launcher side

- `FlowEntry` and `ResultSource::Flow`, fed from the synced entries; `score_flow` pins flows above fuzzy results on exact title prefix and otherwise scores like apps.
- `flow_session: Option<FlowSession { entry, text, rows, pending }>` on `LauncherState`; Enter on a flow entry sets it instead of `hide_to_ghost`; `reset_for_show` and `cycle_mode` clear it.
- While a session is active, printable keys edit `text` and schedule a host query after an 80 ms debounce; navigation keys stay in the launcher; Escape clears the session first and dismisses on the second press.
- Rows render title and subtitle through `result_row` with a subtitle line; Enter on a row runs its first action through `host_exec::post_to_daemon`; `hint_bar` shows the flow prompt.
- Trace target `LAUNCHER_FLOW` with `entered`, `queried`, `rows`, `exited`.

### 5.5 Gate and acceptance

Gate: workspace fmt and clippy for `qol-plugin-api`, `qol-tray`, `launcher`; `cargo test` for the three; `qol check`; visual verification in a guest (`qol env up <environment> --dev-worktree <worktree>`), never on the host session.

Acceptance:

1. Typing `mem` ranks "qol memory" first; Enter keeps the launcher open with the prompt "Ask memory".
2. Typing a question shows memory rows within one query round trip; Escape returns to the normal list.
3. A plugin with `kind = "app"` behaves exactly as before the change.
4. A manifest whose flow names an undeclared query fails validation with a message naming the query.

## 6. Phase 4: harness bridge consolidation (qol-skills)

- New plugin `plugins/qol-memory` in qol-skills mirroring `qol-sessions`: `.mcp.json` `{ "type": "http", "url": "http://127.0.0.1:42700/api/mcp", "headersHelper": "qol mcp headers" }`, `mcp.json` for Codex, kimi and pi entries, `hooks/hooks.json` SessionStart running `qol-memory continue --cwd ... --session ...`, `.pi/extensions/qol-memory-tool.ts` reduced to live capture via `qol-memory capture` and the distill trigger.
- Remove the memory hook, script, and extension from `qol-project`; bump both plugins and run `sync-plugin-manifests.cjs`, then commit the root marketplace file.
- The `qol_memory_retrieve` pi tool is removed once pi reaches the tools through `pi-mcp-adapter`.

Acceptance: a fresh Claude Code session lists `mcp__qol__qol-memory__ask`, the SessionStart continue line still appears for a store with new units, and `ps` shows no memory-specific node process after startup.

## 7. Lane plan

Two lanes never share a file.

Phase 1, round 1 (parallel):

- `mcp-lib`: `libs/qol-mcp/**`, workspace `Cargo.toml` member line.
- `mcp-contract`: `libs/qol-config/src/contract/runtime.rs`, its tests, `docs/plugin-contract.md`.

Phase 1, round 2:

- `mcp-host`: `apps/qol-tray/src/features/mcp/**`, `apps/qol-tray/src/features/mod.rs`, `server/mod.rs` nest line, `plugins/action_executor/mod.rs` (`dispatch_query_with_input`), `apps/qol-tray/Cargo.toml`.
- `mcp-cli`: `tools/qol-cli/src/commands/mcp/**`, `commands/mod.rs` registration.

Phase 2 (parallel with phase 1, own worktree):

- `qm-daemon`: `plugins/qol-memory/src/app/**`, `src/cli.rs`, `src/lib.rs`, `plugin.toml`, `qol-runtime.toml`, `Cargo.toml`.
- `qm-ingest`: `plugins/qol-memory/src/ingest/**`, `src/watch/**`, `src/continue_recall/**`, fixtures under `tests/fixtures/**`.

Phase 3, round 1: `flow-contract` (`libs/qol-plugin-api/src/manifest/schema.rs`, validation, `capability.rs`, docs).
Phase 3, round 2 (parallel): `flow-host` (`features/launcher_apps/**`, `plugin_handlers.rs` body route) and `flow-launcher` (`plugins/launcher/src/**`).

Each lane brief carries the role word, the owned paths, this document's path, the prohibitions (edit only owned paths, no build, test, lint, format, or git commands, no code comments, no em-dash), and the report shape.

## 8. Open decisions and recommendations

1. Tool naming: `<plugin_id>__<runable>`; the plugin id charset already fits harness tool-name rules, and the double underscore is the separator Claude Code itself uses.
2. Input schema: all parameters are required strings in this version; richer types come with a typed input table later.
3. Token delivery: helper command for Claude and pi, static header written by `qol mcp configure` for Codex and kimi; the token is file-stable per host.
4. Endpoint auth: `require_api_access`, never the looser host-only check; the MCP inspector is configured with the header like any client.
5. `[launcher]` lives in `plugin.toml` (install-scoped identity), not in the runtime contract.
6. Flows are an overlay state entered by Enter, not a `SearchMode` variant; Tab keeps cycling apps and files.
7. Query timeout: keep 750 ms for web polling, 10 s for MCP and flows, passed as a parameter to the dispatcher.
8. Distill stays JS and pi-triggered until a Rust port is scoped on its own.
9. `qol sessions mcp` migration onto `qol-mcp` and the host endpoint waits for a per-caller owner identity (the `watch-owner-<token>.json` key is derived from the calling terminal today); do not start it while the concurrent qol-sessions loop-close work is open.
10. No idle exit for the memory daemon; the host watchdog is the contract.
11. Single writer: the pi extension routes capture through the daemon with a direct-append fallback when the socket is absent.

## 9. Non-goals

Dense or embedding rerank, query rewriting, Rust distillation, a gpui memory panel, streams over MCP, server-initiated MCP streams, MCP resources or prompts, and any change to `qol sessions mcp` behaviour.
