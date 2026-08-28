# Phase 3 spec: launcher flow entries

Status: architect contract for the `flow-contract`, `flow-host`, `flow-launcher` and `flow-memory` lanes. Source plan: `docs/research/qol-memory/interface-plan.md` section 5. Scout reports (2026-08-28) verified every fact below against the tree at d34decd63; line numbers cite that tree.

Rules for every lane: edit only the paths in your ownership row; never run build, test, lint, format or git commands; add no code comments; never use the em-dash character anywhere. Every public signature below is exact; do not rename, reorder or widen it. Report changed files and lines plus conscious deviations, nothing else.

## 1. Goal

- A plugin declares `[launcher]` in `plugin.toml`. `kind = "flow"` names a runtime query that turns typed text into rows.
- The host writes every valid flow entry to one JSON file the launcher reads; the launcher lists flows next to apps.
- Typing `mem` in the launcher ranks `qol memory` first. Enter keeps the launcher open with the prompt `Ask memory`; typed text is sent to the plugin query after an 80 ms debounce; rows show the answer, recalled units and skill hits; Enter on a row copies its text (or runs the first declared row action); Escape returns to the normal list.
- `POST /api/plugins/{id}/queries/{query}` with a JSON body reaches `dispatch_query_with_input` with the 10 s MCP timeout.
- A manifest whose flow names an undeclared query fails validation with a message naming the query.

Deviation from the plan, decided by the architect: rows come pre-shaped from the plugin (`rows: [{ title, subtitle, copy }]`) instead of `row_title` and `row_subtitle` templates, because `ask` output carries recalled keys without text and the plugin is the only side that can render them. Row action input still uses `{field}` templates over the row object.

## 2. Ownership

| Lane | Owned paths |
|---|---|
| `flow-contract` | `libs/qol-plugin-api/src/manifest/schema.rs`, `libs/qol-plugin-api/src/manifest/validation/launcher_rules.rs` (new), `libs/qol-plugin-api/src/manifest/validation/mod.rs`, `libs/qol-plugin-api/src/manifest/validation/manifest_rules.rs`, `libs/qol-plugin-api/src/manifest/validation_tests.rs`, `libs/qol-plugin-api/src/manifest/mod.rs`, `libs/qol-plugin-api/src/launcher_flows/mod.rs` (new), `libs/qol-plugin-api/src/lib.rs`, `libs/qol-plugin-api/src/capability.rs`, `libs/qol-plugin-api/tests/capability_declarations_structural.rs`, `docs/plugin-contract.md` |
| `flow-host` | `apps/qol-tray/src/features/launcher_apps/mod.rs`, `apps/qol-tray/src/features/plugin_store/server/plugin_handlers.rs` |
| `flow-launcher` | everything under `plugins/launcher/src/`, `plugins/launcher/Cargo.toml` |
| `flow-memory` | `plugins/qol-memory/plugin.toml`, `plugins/qol-memory/qol-runtime.toml`, `plugins/qol-memory/src/ask/rows.rs` (new), `plugins/qol-memory/src/ask/mod.rs` (only the `pub mod rows;` line), `plugins/qol-memory/src/app/request.rs`, `plugins/qol-memory/src/cli.rs` |

No two lanes share a file. Lanes compile only after the fan-in; write against the signatures here, not against the current tree.

## 3. Lane `flow-contract`

### 3.1 `libs/qol-plugin-api/src/manifest/schema.rs`

Add after `ShortcutDeclaration` handling, before `PluginManifest`:

```rust
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LauncherKind {
    App,
    Flow,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct LauncherSpec {
    pub kind: LauncherKind,
    pub title: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub row_actions: Vec<qol_config::contract::RowActionSpec>,
}
```

`PluginManifest` (schema.rs:87-111) gains, after `shortcuts`:

```rust
    #[serde(default)]
    pub launcher: Option<LauncherSpec>,
```

`RowActionSpec` is re-exported from `qol_config::contract` (contract/mod.rs:15 `pub use v1::{...}`); verify the name is in that list and use the shortest existing public path.

### 3.2 `libs/qol-plugin-api/src/manifest/validation/launcher_rules.rs` (new)

```rust
pub(super) fn validate_optional_launcher(
    launcher: Option<&LauncherSpec>,
    executable_action_ids: &BTreeSet<String>,
) -> Result<()>

pub fn validate_launcher_runtime(
    manifest: &PluginManifest,
    runtime: Option<&qol_config::contract::RuntimeSpec>,
) -> Result<()>
```

`validate_optional_launcher` (anyhow `bail!`, same style as the sibling rule files):

- `None` is Ok.
- `title.trim()` empty: `launcher.title must not be empty`.
- `prompt` present and empty after trim: `launcher.prompt must not be empty`.
- `kind == Flow`: `query` missing or empty after trim: `launcher.query is required for kind = "flow"`; query failing `qol_config::contract::is_valid_runable_name` (runtime.rs:83; confirm the re-export path): `invalid launcher.query name: {query}`.
- `kind == App`: `query` present: `launcher.query is only valid for kind = "flow"`; `row_actions` non-empty: `launcher.row_actions are only valid for kind = "flow"`.
- every `row_actions[i].action` not in `executable_action_ids`: `launcher.row_actions references undeclared action: {action}`.

`validate_launcher_runtime` (the cross-file check; the host calls it with the parsed `qol-runtime.toml`):

- launcher `None` or `kind == App`: Ok.
- `runtime` is `None`: `launcher flow query {query} requires qol-runtime.toml`.
- `query` not in `runtime.queries`: `launcher flow query not declared: {query}`.
- the query's `input` is `None` or lacks the key `query`: `launcher flow query {query} must declare a query input`.

Register the module in `validation/mod.rs` the way the sibling rule modules are registered; call `super::launcher_rules::validate_optional_launcher(self.launcher.as_ref(), executable_action_ids)` from `PluginManifest::validate()` (manifest_rules.rs:4-33) after `validate_shortcuts`. Export `validate_launcher_runtime` from `manifest/mod.rs` next to the existing `pub use validation::{...}` list.

Tests in `validation_tests.rs` (use the existing `validate_toml` helper and `base_manifest()` style): `launcher_flow_requires_query`, `launcher_flow_rejects_invalid_query_name`, `launcher_app_rejects_query_and_row_actions`, `launcher_row_action_must_be_declared`, `launcher_flow_parses_and_validates` (a full `[launcher]` flow section with one `[[launcher.row_actions]]` naming a catalogued action). For `validate_launcher_runtime`: `launcher_runtime_rejects_undeclared_query` (error text contains `launcher flow query not declared: rows`), `launcher_runtime_requires_query_input`, `launcher_runtime_accepts_declared_query` (build the `RuntimeSpec` with `qol_config::contract::parse_runtime_spec_str`, runtime.rs:193).

### 3.3 `libs/qol-plugin-api/src/launcher_flows/mod.rs` (new), `lib.rs`

```rust
pub const FLOWS_FILE_NAME: &str = "launcher-flows.json";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct FlowEntry {
    pub plugin_id: String,
    pub title: String,
    pub prompt: String,
    pub query: String,
    #[serde(default)]
    pub row_actions: Vec<qol_config::contract::RowActionSpec>,
}

pub fn flows_path() -> Option<PathBuf>
pub fn write_flows(path: &Path, entries: &[FlowEntry]) -> std::io::Result<()>
pub fn read_flows(path: &Path) -> Vec<FlowEntry>
```

- `flows_path` = `qol_config::data_dir()` (libs/qol-config/src/lib.rs:38) joined with `FLOWS_FILE_NAME`.
- `write_flows` creates the parent directory, writes pretty JSON (an array) to `<path>.tmp` and renames it over `path`.
- `read_flows` returns an empty vector when the file is missing or unparseable; it never panics.
- `lib.rs` adds `pub mod launcher_flows;`.

Tests: `flows_round_trip_through_a_temp_file`, `read_flows_tolerates_missing_and_garbage` (use `tempfile`, adding it to dev-dependencies only if it is not already there; `std::env::temp_dir` with a unique subdirectory is acceptable instead).

### 3.4 `capability.rs` and `tests/capability_declarations_structural.rs`

Delete `LauncherProviderCapability` (capability.rs:54-58) and its module doc sentence about `launcher-provider`; delete the `launcher_provider_capability_is_a_marker` test and its import (tests file :12, :100-107). Nothing else references it.

### 3.5 `docs/plugin-contract.md`

In section 2.1 (line 76) add a `[launcher]` bullet after `[[shortcuts]]` documenting `kind` (`app` is the default host behavior, `flow` keeps the launcher open), `title`, `prompt`, `query`, `[[launcher.row_actions]]` (same shape as config row actions; `input` values may use `{field}` templates over the returned row object), and the flow query contract: the query must declare a `query` input; it receives `{ "query": "<typed text>" }` and returns an object with a `rows` array whose items carry `title` (required), `subtitle` and `copy` (optional strings) plus any other fields used by row action templates. Add the POST form of the query route wherever the GET query route is documented (search the file for `queries/`); if it is not documented, add one sentence under section 2.1's runtime bullet.

## 4. Lane `flow-host`

### 4.1 `apps/qol-tray/src/features/launcher_apps/mod.rs`

```rust
pub fn collect_flow_entries<'a>(
    plugins: impl IntoIterator<Item = &'a Plugin>,
) -> Vec<qol_plugin_api::launcher_flows::FlowEntry>
```

For each plugin whose `manifest.launcher` is `Some` with `kind == LauncherKind::Flow`: load the runtime contract with `crate::plugins::config::load_runable_contract_from_root(&plugin.path)` (config/mod.rs:990); on `Err` log with `log::error!` and skip; call `qol_plugin_api::manifest::validate_launcher_runtime(&plugin.manifest, runtime.as_ref())`; on `Err` log the error and skip; otherwise push `FlowEntry { plugin_id: plugin.id.to_string(), title, prompt: prompt.unwrap_or_else(|| title.clone()), query, row_actions }`.

`trigger_full_sync_with_manager` (mod.rs:103) collects flow entries under the same manager lock it already holds for settings entries and hands them to `sync_launcher_entries`; the spawned sync thread calls `sync_entries(entries, &bin)`, then `write_flow_entries(&flows)`, then `platform::publish_synced()`, in that order.

```rust
fn write_flow_entries(entries: &[qol_plugin_api::launcher_flows::FlowEntry])
```

`flows_path()` None: `log::warn!` and return; write error: `log::error!`.

Test: `collect_flow_entries_keeps_only_valid_flows`: two `Plugin::new` values over temp directories. Plugin `a` has `[launcher] kind = "flow", title = "a", query = "rows"` and a `qol-runtime.toml` with `[query.rows]` declaring `input = { query = "q" }`; plugin `b` has a flow naming `missing` with the same runtime file. The result holds exactly `a` with `prompt == "a"`. Reuse the fixture style of the existing tests in this file (mod.rs:157-273).

### 4.2 `apps/qol-tray/src/features/plugin_store/server/plugin_handlers.rs`

Route line :33 becomes `.route("/plugins/{id}/queries/{query}", get(query_plugin_handler).post(query_plugin_with_input_handler))`.

```rust
pub(super) async fn query_plugin_with_input_handler(
    Path((id, query)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(input): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)>
```

Same validation as `query_plugin_handler` (:57-79); dispatch with `crate::plugins::action_executor::dispatch_query_with_input(&state.plugin_manager, &id, &query, input, crate::plugins::action_executor::MCP_DISPATCH_TIMEOUT)`. Factor the shared contract check into one private helper so the two handlers do not duplicate it.

## 5. Lane `flow-launcher`

### 5.1 `plugins/launcher/src/flow/mod.rs` (new), `lib.rs` module line

Headless flow transport, no gpui:

```rust
pub use qol_plugin_api::launcher_flows::FlowEntry;

pub const MAX_ROWS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct FlowRow {
    pub title: String,
    pub subtitle: Option<String>,
    pub copy: Option<String>,
    pub raw: serde_json::Value,
}

pub fn parse_rows(payload: &serde_json::Value) -> Result<Vec<FlowRow>, String>
pub fn fetch_rows(entry: &FlowEntry, text: &str) -> Result<Vec<FlowRow>, String>
pub fn render_action_input(
    action: &qol_config::contract::RowActionSpec,
    row: &FlowRow,
) -> serde_json::Value
pub fn run_row_action(
    entry: &FlowEntry,
    action: &qol_config::contract::RowActionSpec,
    row: &FlowRow,
) -> Result<(), String>
```

- `parse_rows`: `payload["rows"]` must be an array, else `Err("flow response has no rows array")`; items that are objects with a string `title` become rows (`subtitle`, `copy` read when they are strings; `raw` is the item); other items are skipped; the result is truncated to `MAX_ROWS`.
- `fetch_rows`: body `{"query": text}`; `qol_plugin_api::host_exec::post_to_daemon(&qol_conventions::api_routes::plugin_query(&entry.plugin_id, &entry.query), &body)` (host_exec.rs:46, conventions lib.rs:92); io error -> its `to_string()`; status outside 200..300 -> `format!("host {status}: {body}")`; then `parse_rows` on the parsed body.
- `render_action_input`: an object whose keys are `action.input`'s keys; each value is the template with every `{name}` replaced by `row.raw[name]` when that is a string, otherwise left verbatim; `action.input` None -> `{}`.
- `run_row_action`: posts the rendered input to `api_routes::plugin_action(&entry.plugin_id, &action.action)`; non-2xx -> `Err(format!("host {status}: {body}"))`.

Tests: `parse_rows_keeps_titled_objects_and_caps`, `parse_rows_rejects_missing_array`, `render_action_input_substitutes_string_fields`.

### 5.2 `discovery/search.rs`, `discovery/entry_store.rs`, `discovery/mod.rs`

- `ResultItem` gains `Flow(&'a FlowEntry)`; `ResultSource` gains `Flow`.
- `filtered` and `filtered_from_candidates` gain a `flow_entries: &[FlowEntry]` parameter directly after `file_entries`. In `SearchMode::Apps` they also score flows through `fn score_flow(index: usize, title: &str, query: &PreparedQuery<'_>) -> Option<Scored>`: fuzzy match on the title; when the lowercased title starts with the lowercased query, any whitespace-separated word of the title starts with it, or it appears at a word boundary (`contains_at_word_boundary`), subtract `FLOW_PIN_BONUS` (`const FLOW_PIN_BONUS: i32 = 10_000;`) so an exact prefix sorts above every app; `match_kind` from `classify_lowered_match`; `frecency_bonus` and `manual_boost` zero. Flows are never scored in `SearchMode::Files`.
- `EntryStore` gains `flow_entries: Arc<Vec<FlowEntry>>`; `new(app_entries, file_entries, flow_entries)`; `replace_entries(app_entries, file_entries, flow_entries)` (the `Arc::ptr_eq` short-circuit covers all three); `name` and `item` handle `ResultSource::Flow` (name is the title).
- `PreloadedEntries` gains `flow_entries: Arc<Vec<FlowEntry>>`; `start` loads flows with `read_flows(&flows_path)` when `qol_plugin_api::launcher_flows::flows_path()` is `Some`, and the watch loop reloads them on every `WatchSignal::HostHint` before `publish`. Every `publish` call carries the current flows.

Test in search.rs: `flow_title_prefix_ranks_before_apps` (an app named `Memory Manager` and a flow titled `qol memory`; query `mem` puts the flow first; query `manager` leaves the flow out).

### 5.3 `ui/state.rs`, `ui/input.rs`

```rust
pub struct FlowSession {
    pub entry: FlowEntry,
    pub rows: Vec<FlowRow>,
    pub generation: u64,
    pub pending: bool,
}
```

`LauncherState` gains `pub flow: Option<FlowSession>` (`None` in `new`). Methods:

```rust
pub fn enter_flow(&mut self, entry: FlowEntry)
pub fn exit_flow(&mut self)
pub fn flow_result_count(&self) -> usize
```

`enter_flow` sets the session, clears `query`, `cursor`, the selection and the launch error, and resets `scroll_list`; `exit_flow` clears the session and does the same resets. The typed flow text lives in `state.query`, so `insert_char`, `backspace`, `delete_forward`, selection and clipboard helpers work unchanged.

`InputEffect` gains `FlowQueryChanged`, `FlowActivate`, `FlowExit`. `apply_key` (input.rs:15) starts with `if self.flow.is_some() { return self.apply_flow_key(key, secondary, control, shift, alt, result_count); }`.

```rust
fn apply_flow_key(&mut self, key: &str, secondary: bool, control: bool, shift: bool, alt: bool, result_count: usize) -> InputEffect
```

Mapping: `escape`/`esc` -> `FlowExit`; `enter` -> `FlowActivate`; `up`/`down` without `secondary` -> `move_up` / `move_down(result_count)` -> `Navigate`; `left`, `right`, `home`, `end` -> the same cursor moves as today -> `Navigate`; `backspace`/`delete` -> `FlowQueryChanged` when the text changed else `Navigate`; `a` with `secondary` -> `select_all` -> `Navigate`; `space` without modifiers -> insert -> `FlowQueryChanged`; printable via `key_to_input_char` -> `FlowQueryChanged`; `tab`, boost keys and every other modified key -> `Ignore`.

Tests (input.rs): `flow_escape_exits_and_enter_activates`, `flow_tab_is_ignored`, `flow_typing_reports_flow_query_changed`; (state.rs): `enter_flow_clears_query_and_exit_flow_clears_session`.

### 5.4 `ui/controller.rs`, `ui/mod.rs`

- `handle_key`: `result_count` is `self.state.flow_result_count()` when a flow is active, else `self.store.result_count()`; the `ensure_filtered` call is skipped while a flow is active.
- Effects: `FlowExit` -> `self.state.exit_flow(); trace::flow(self, "exited"); cx.notify();`. `FlowActivate` -> `self.activate_flow_row(window, cx)`. `FlowQueryChanged` -> `self.state.clear_launch_error(); self.state.reset_results_position(); self.schedule_flow_query(cx);`.
- `launch_selected` (:152-194): when `store.item(scored)` is `ResultItem::Flow(entry)`, call `self.state.enter_flow(entry.clone())`, `trace::flow(self, "entered")`, `cx.notify()` and return without hiding or recording frecency.

```rust
const FLOW_DEBOUNCE: Duration = Duration::from_millis(80);

fn schedule_flow_query(&mut self, cx: &mut Context<Self>)
fn activate_flow_row(&mut self, window: &mut Window, cx: &mut Context<Self>)
```

`schedule_flow_query`: bump `flow.generation`, set `pending = true`, capture `generation`, `entry.clone()` and `query.clone()`. Empty trimmed text: clear `rows`, `pending = false`, notify, return. Otherwise spawn with the `cx.spawn` + `async_cx.background_executor().timer(FLOW_DEBOUNCE).await` pattern of `start_entry_watch` (ui/mod.rs:213-241); after the timer, drop out if the view's current generation differs; `trace::flow(view, "queried")`; run `crate::flow::fetch_rows(&entry, &text)` inside `cx.background_spawn`; back on the entity, apply only when the generation still matches: `Ok(rows)` -> `flow.rows = rows`, `Err(message)` -> `flow.rows.clear()` and `set_launch_error(message)`; `pending = false`; `trace::flow(view, "rows")`; `cx.notify()`.

`activate_flow_row`: the selected row is `flow.rows[scroll_list.selected]`; no row -> return. With `entry.row_actions` non-empty run `crate::flow::run_row_action(&entry, &entry.row_actions[0], row)`; `Err` -> `set_launch_error` and notify, return. Otherwise copy `row.copy.clone().unwrap_or_else(|| row.title.clone())` with `cx.write_to_clipboard(ClipboardItem::new_string(text))` (controller.rs:119 shows the call). Then `trace::flow(self, "activated")` and `self.hide_to_ghost("flow", window)`.

`sync_entries_from_shared` (ui/mod.rs:134-147) passes `fresh.flow_entries.clone()` to `replace_entries`.

### 5.5 `ui/view.rs`, `ui/layout.rs`, `ui/render.rs`

- layout.rs: `pub const FLOW_ROW_HEIGHT: f32 = 44.0;` and `pub fn window_height_for(visible_rows: usize, row_height: f32) -> f32`; `window_height_for_rows(n)` becomes `window_height_for(n, ROW_HEIGHT)`.
- view.rs: `pub fn flow_row(row: &FlowRow, selected: bool, row_height: f32) -> Div` renders the letter tile from the title, the title on the first line and the subtitle (when present) on a second, smaller, muted line, inside `kit.row_selected`; `pub fn hint_bar_flow(entry: &FlowEntry) -> Div` lists Enter with the first row action's `label` (fallback `copy`), arrows `move`, `esc back`, a chip with `entry.title`, Spacer; `kind_label` returns `"flow"` for `ResultSource::Flow`.
- render.rs: while a flow is active the rows come from `flow.rows` through `flow_row`, the height from `window_height_for(visible, FLOW_ROW_HEIGHT)`, the hint bar from `hint_bar_flow`, and the header shows `entry.prompt` as placeholder text while `query` is empty and `entry.title` where the mode label chip is drawn today. Everything else keeps its current path.

### 5.6 `ui/trace.rs`

```rust
pub(super) fn flow(view: &LauncherView, event: &'static str)
```

Debug-gated like `input` (trace.rs:86-99): `qol_runtime::probe!("LAUNCHER_FLOW", "event={} plugin={} q=\"{}\" rows={} gen={} pending={}", ...)` with `plugin` from `flow.entry.plugin_id` (or `none`), the query through `quoted(.., 120)`, `rows` the row count, `gen` the generation, `pending` the flag.

### 5.7 `plugins/launcher/Cargo.toml`

Only if a needed crate is missing: `serde_json` and `qol-config` are already workspace dependencies; add nothing else.

## 6. Lane `flow-memory`

### 6.1 `plugins/qol-memory/qol-runtime.toml`

Add after `[query.continue]`:

```toml
[query.rows]
description = "Launcher rows for a question: the answer, recalled units and skill hits"
poll_interval_ms = 60000
input = { query = "question typed in the launcher" }
```

Not an agent tool.

### 6.2 `plugins/qol-memory/plugin.toml`

Add after `[capabilities]`:

```toml
[launcher]
kind = "flow"
title = "qol memory"
prompt = "Ask memory"
query = "rows"
```

### 6.3 `plugins/qol-memory/src/ask/rows.rs` (new), `ask/mod.rs`

```rust
pub const MAX_ROWS: usize = 8;
pub const TITLE_CHARS: usize = 140;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowRow {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub copy: String,
    pub key: String,
    pub kind: String,
}

pub fn title_of(text: &str) -> String
pub fn from_output(output: &AskOutput, units: &UnitsLayer, notes: &NotesLayer) -> Vec<FlowRow>
```

- `title_of`: whitespace collapsed to single spaces and trimmed; when longer than `TITLE_CHARS` chars, the first `TITLE_CHARS` chars followed by `...`.
- `from_output`, in order, stopping at `MAX_ROWS`:
  1. `output.answer` when present: `title_of(text)`, subtitle `format!("{} {} {}", output.verdict, answer.source_kind, date)` where `date` is the first 10 chars of `source_ts` (empty when absent), `copy` the full text, `key` the answer key, `kind` `answer`.
  2. Each `output.recalled` entry whose key was not used yet: text and kind from the unit with that key in `units.items` (`kind` = unit kind), else from the note with that key in `notes.items` (`kind` = note `cls`); skip when neither holds it; subtitle `format!("{} {}", kind, date)` from `source_ts`.
  3. Each `output.skills.hits` entry: title `format!("{}: {}", name, section)` (name falls back to the id, section to an empty string, trimmed), subtitle `format!("skill {}", id)`, `copy` = `content` flattened (`None` -> empty string), `key` = id, `kind` `skill`.
- `ask/mod.rs` gains only `pub mod rows;` next to the other module lines; nothing else in that file changes (parity oracle).

Tests in `rows.rs`: `title_of_collapses_and_caps`, `from_output_orders_answer_recalled_skills_and_caps` (build the `AskOutput` with `serde_json::from_value` over a JSON fixture; units and notes layers constructed directly).

### 6.4 `plugins/qol-memory/src/app/request.rs`

Add `"rows" => rows(state, &request.input)` to the dispatch match (request.rs:17-22).

```rust
fn rows(state: &Arc<Mutex<WarmState>>, input: &Value) -> Result<Value>
```

`query = string_field(input, "query", "rows")?`, trimmed; empty -> `bail!("rows: input.query must not be empty")`. Build `AskRequest { query, k: DEFAULT_ASK_K, brief: false, exclude_session: None }` and `LogOptions { source: "launcher".to_string(), cwd: None, fact: None, no_log: false }`; `let (store, aliases, units, notes) = warm.views()?;` then `crate::ask::run_and_log_with_layers(store, aliases, &req, &log, units, notes)?`; answer `json!({ "verdict": output.verdict, "confidence": output.confidence, "rows": crate::ask::rows::from_output(&output, units, notes) })`.

Test `request_rows_returns_the_answer_row_first`: same fixture as `request_capture_from_text_is_idempotent_and_recallable` (four filler units plus the 14-rare-word capture), then a `rows` request with that text answers `rows[0].kind == "answer"` and `rows[0].title == title_of(text)`; `request_rows_rejects_empty_query`.

### 6.5 `plugins/qol-memory/src/cli.rs`

New verb `rows`: usage `qol-memory rows "<query>" [--store PATH]`, about `Print the launcher rows for a question.`, detail `Rows are the answer, the recalled units and the skill hits, in that order.` Socket first exactly like `continue_payload` (cli.rs:682-694): `crate::app::send_request("rows", json!({ "query": query }))` when `--store` is absent, falling back in-process on an unreachable daemon: `Store::resolve`, `store.read_units()?`, `store.read_notes()?`, the alias map the way `ask` loads it, `crate::ask::run_with_layers` (ask/mod.rs:282) with the same `AskRequest` as 6.4, then `from_output`. Plain output: one line per row, `title` then a tab then the subtitle (empty when absent). JSON output: the same object 6.4 returns. Register the verb in the command list and `help`; usage errors follow the `ask` conventions (missing query -> usage, exit 64).

Tests: `rows_requires_a_query`, `rows_help_is_listed` (extend the existing help test if a list is asserted).

## 7. Gate and acceptance (architect)

1. `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings` for `-p qol-plugin-api -p qol-tray -p launcher -p qol-memory -p qol` on host, `x86_64-apple-darwin` and `x86_64-pc-windows-msvc` (launcher and qol-tray where they build for that target); `cargo test -p qol-plugin-api -p qol-tray -p launcher -p qol-memory`; `env -u QOL_TRAY_HTTP_TOKEN cargo run -q -p qol -- check`.
2. Parity `node docs/research/qol-memory/parity.mjs --bin target/debug/qol-memory --store <copy>` stays 73/73.
3. Recompile-self; `~/.local/share/qol-tray/launcher-flows.json` holds the qol-memory flow; `curl -X POST .../api/plugins/qol-memory/queries/rows -d '{"query":"LookPose"}'` returns rows; `qol-memory rows "LookPose"` prints them.
4. Launcher behavior (typing `mem`, Enter, rows, Escape) verified in a guest per `qol-project:qol-dev-environments` when a guest lane is available; otherwise recorded as not visually verified.
5. Squash to main as one commit with explicit paths; no push.

## 8. Out of scope

- Row templates in the manifest, multiple row actions per row, flow entries in `SearchMode::Files`, spinners, and any change to `ask` output.
