# Phase 2 spec: qol-memory stage 2 daemon

Status: architect contract for the `qm-daemon` and `qm-ingest` lanes. Source plan: `docs/research/qol-memory/interface-plan.md` section 4. Phase 1 (host MCP endpoint, `agent_tool` contract flag) is on main as 91e7b174b; this phase makes `tools/list` non-empty by giving qol-memory a resident daemon that answers `ask`, `status`, `continue`, `capture` and `reindex` over the plugin socket.

Rules for every lane: edit only the paths in your ownership row; never run build, test, lint, format or git commands; add no code comments; never use the em-dash character anywhere. Every public signature below is exact; do not rename, reorder or widen it. Report changed files and lines plus conscious deviations, nothing else.

## 1. Goal

- `qol-memory` runs as a qol-tray daemon (no args, `QOL_TRAY_DAEMON_SOCKET` set) and answers the actions in section 2 through `qol_plugin_daemon::daemon::run_stateful_request_listener`.
- The daemon watches the two transcript roots and ingests new units incrementally into `units.jsonl`, replacing the run-based `snapshot.mjs` + `mergeStep` walk for day-to-day capture.
- `qol-memory ask` and `qol-memory status` from a terminal print byte-identical stdout with and without the daemon.
- `qol-memory continue --cwd X --session Y` prints exactly what `inject-qol-memory-continue.cjs` prints for the same store and marker state.
- The MCP endpoint lists `qol-memory__ask` and `qol-memory__status`.

## 2. Contract files (lane `qm-daemon`)

### 2.1 `plugins/qol-memory/plugin.toml`

Add after `[runtime]`:

```toml
[daemon]
enabled = true
command = "qol-memory"
socket = "/tmp/qol-memory.sock"
```

Everything else unchanged. Version stays 0.1.0 (release bump is a separate step).

### 2.2 `plugins/qol-memory/qol-runtime.toml` (new)

```toml
schema_version = 1

[query.ask]
description = "Answer a question from the settled agent session history"
agent_tool = true
tool_description = "Retrieve settled facts from the user's agent session history. Ask in plain words; the answer names the matching prior sessions and skills."
input = { query = "question in plain words", cwd = "optional working directory for scoping", exclude_session = "optional session id to exclude" }

[query.status]
description = "Store size, index freshness and pending candidates"
agent_tool = true
tool_description = "Report the qol-memory store size, index freshness and pending distillation candidates."

[query.continue]
description = "Units landed since the per-cwd continuation marker"
input = { cwd = "working directory of the session", session = "session id being continued" }

[action.capture]
description = "Append one redacted unit to the store"
input = { unit = "one unit as a JSON object" }

[action.reindex]
description = "Drop the persisted BM25 indexes and rebuild them"
```

The runtime contract validator (`libs/qol-config/src/contract/runtime.rs`) requires a `tool_description` when `agent_tool = true` and input names that pass `is_valid_runable_name`.

### 2.3 `plugins/qol-memory/Cargo.toml`

Add dependencies `qol-plugin-daemon.workspace = true`, `qol-runtime.workspace = true`, `qol-watch.workspace = true`. Nothing removed.

## 3. Lanes and ownership

| Lane | Owns (exclusive) |
|---|---|
| `qm-daemon` | `plugins/qol-memory/plugin.toml`, `plugins/qol-memory/qol-runtime.toml`, `plugins/qol-memory/Cargo.toml`, `plugins/qol-memory/src/lib.rs`, `plugins/qol-memory/src/main.rs`, `plugins/qol-memory/src/cli.rs`, `plugins/qol-memory/src/ask/mod.rs`, `plugins/qol-memory/src/app/mod.rs`, `plugins/qol-memory/src/app/request.rs`, `plugins/qol-memory/src/app/warm.rs`, `plugins/qol-memory/src/watch/mod.rs` |
| `qm-ingest` | `plugins/qol-memory/src/store/mod.rs`, `plugins/qol-memory/src/store/lock.rs`, `plugins/qol-memory/src/ingest/mod.rs`, `plugins/qol-memory/src/ingest/redact.rs`, `plugins/qol-memory/src/ingest/transcript.rs`, `plugins/qol-memory/src/ingest/state.rs`, `plugins/qol-memory/src/continue_recall/mod.rs`, `docs/research/qol-memory/lib/merge.js` |

`src/lib.rs` (qm-daemon) declares `pub mod app; pub mod continue_recall; pub mod ingest; pub mod watch;` alongside the existing modules, in alphabetical order. `src/store/mod.rs` (qm-ingest) declares `pub mod lock;` next to `pub mod seal;`.

The qm-daemon lane calls into qm-ingest modules only through the signatures in sections 5 and 6. The qm-ingest lane never touches `ask`, `cli`, `app` or `watch`.

## 4. Daemon (lane `qm-daemon`)

### 4.1 Entry and CLI

`src/main.rs` mirrors qol-voice: when `args.is_empty()` and `std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some()`, call `qol_memory::app::run_daemon()` and map `Ok` to `ExitCode::SUCCESS`, `Err` to stderr `{error:#}` plus `ExitCode::FAILURE`; otherwise `qol_memory::cli::exit_code(args)`. The existing manifest tests stay; `live_manifest_declares_the_headless_contract` additionally asserts `manifest.daemon.as_ref().map(|d| d.enabled) == Some(true)` and the socket equals `/tmp/qol-memory.sock`.

`src/cli.rs` adds four commands to the `HeadlessApp` (register in this order after `status`): `run`, `capture`, `continue`, `reindex`.

- `run`: about "Run the resident memory daemon."; alias `daemon`; `run_result` calls `crate::app::run_daemon()` and maps errors to a failed `Execution`; exit behaviour "Runs until stopped; exits non-zero if daemon startup fails."
- `capture`: usage `qol-memory capture --unit '<json>' [--store PATH]`; `--unit` is required and must parse as a JSON object; plain output `appended: <n>`; `--json` returns `{"appended": n}`. Socket first, then in-process `crate::ingest::append_units` with a freshly loaded `KeySet`.
- `continue`: usage `qol-memory continue --cwd PATH --session ID [--store PATH]`; both flags required; plain output is `outcome.block` when `outcome.stage == "injected"` else empty stdout; exit 0 in every non-usage case; `--json` returns `serde_json::to_value(outcome)`. Socket first, then `crate::continue_recall::run`.
- `reindex`: usage `qol-memory reindex [--store PATH]`; plain output `reindexed: <layers>`; `--json` returns `{"layers": [..]}`. Socket first, then in-process `crate::app::warm::reindex(&store)`.

Socket first means: `crate::app::send_request(action, input)` returning `Ok(Some(value))` is rendered; `Err` whose source is `io::ErrorKind::NotFound` or `ConnectionRefused` falls back to in-process; any other `Err` is a runtime failure. `ask` and `status` gain the same socket-first path; their in-process branch is unchanged. Usage parsing for `ask`/`status` is unchanged; `--store` disables the socket path (an explicit store is always in-process).

The socket `ask` input is `{"query", "k", "brief", "exclude_session", "log_source", "log_cwd", "log_fact", "no_log"}` built from the parsed invocation; `status` sends `{}`.

Tests to keep green in `cli.rs`: all existing ones. Add `run_help_lists_the_daemon_alias`, `capture_requires_a_json_object`, `continue_requires_cwd_and_session`, and extend `doctor_and_help_never_invoke_operational_handlers` so `help capture`, `help continue`, `help reindex` also call no handler (the sentinel app grows four no-op handlers for the new commands; keep `app_with_handlers` as the single injection point).

### 4.2 `src/app/mod.rs`

```rust
pub mod request;
pub mod warm;

pub const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

pub fn run_daemon() -> anyhow::Result<()>;
pub fn send_request(action: &str, input: serde_json::Value) -> anyhow::Result<Option<serde_json::Value>>;
```

`run_daemon`:
1. `Store::resolve(None)`, `crate::aliases::embedded()`, `IngestRoots::resolve()`.
2. Build `warm::WarmState::open(store, aliases)?` (loads layers and the key set once).
3. Wrap in `Arc<Mutex<WarmState>>`; start `crate::watch::spawn(roots, Arc::clone(&state))` and keep the returned `WatchHandle` alive for the daemon lifetime; a watch failure is logged to stderr with the `qol_runtime::probe!("QOL_MEMORY_DAEMON", ...)` trace and the daemon keeps serving.
4. Run an initial `ingest::ingest_all` on a background thread so startup does not block the socket.
5. `run_stateful_request_listener(&DAEMON_CONFIG, state, request::handle)` with `.context("qol-memory daemon listener failed")`.

`send_request` mirrors qol-voice: `core_daemon::send_request(&DAEMON_CONFIG, action, input, Duration::from_secs(10))` mapped `Handled { data } => Ok(data)`, `Fallback => bail!("qol-memory daemon declined action `{action}`")`, `Error { message } => bail!(message)`. The io error is preserved as the anyhow source so the CLI can classify NotFound / ConnectionRefused.

### 4.3 `src/app/request.rs`

```rust
pub fn handle(state: &mut Arc<Mutex<WarmState>>, request: &DaemonRequest) -> ReadResult<()>;
```

Action table, all payloads `serde_json::Value`:

| action | input | result |
|---|---|---|
| `ping`, `kill` | ignored | `ReadResult::Handled` |
| `ask` | see 4.1; missing `k` defaults 5, missing `log_source` defaults `"daemon"` | `serde_json::to_value(AskOutput)` from `ask::run_and_log_with_layers` |
| `status` | `{}` | `ask::status_with_layers` |
| `continue` | `{cwd, session}` | `serde_json::to_value(ContinueOutcome)` |
| `capture` | `{unit}` object | `{"appended": n}` after `ingest::append_units` with the warm key set, then `WarmState::push_units` |
| `reindex` | `{}` | `{"layers": [..]}` from `warm::reindex` followed by `WarmState::invalidate_layers` |
| anything else | | `ReadResult::Fallback` |

Errors become `ReadResult::Error(format!("{error:#}"))`. Missing or wrongly typed required inputs are errors with the field named, for example `capture: input.unit must be a JSON object`.

### 4.4 `src/app/warm.rs`

```rust
pub struct WarmState { .. }

impl WarmState {
    pub fn open(store: Store, aliases: AliasMap) -> anyhow::Result<WarmState>;
    pub fn store(&self) -> &Store;
    pub fn aliases(&self) -> &AliasMap;
    pub fn keys(&mut self) -> &mut crate::ingest::KeySet;
    pub fn layers(&mut self) -> anyhow::Result<(&UnitsLayer, &NotesLayer)>;
    pub fn push_units(&mut self, units: &[serde_json::Value]);
    pub fn invalidate_layers(&mut self);
}

pub fn reindex(store: &Store) -> anyhow::Result<Vec<String>>;
```

`layers` returns the cached `UnitsLayer` and `NotesLayer`, re-reading from disk only when `units.jsonl` metadata (len, mtime) or the newest notes run name differ from the cached fingerprint. `push_units` appends parsed `Unit`s to the cached layer and updates the fingerprint to the post-append metadata so the next `layers` call is a cache hit; a parse failure of an appended value falls back to `invalidate_layers`. `reindex` removes every `idx-*.json` and `idx-*.meta` under the store root, rebuilds `pool`, `user` and `notes` through `cache::build_or_load` and returns the layer names rebuilt.

### 4.5 `src/ask/mod.rs` refactor (exact)

Split the readers out of the pure functions without changing any output:

```rust
pub fn run(store: &Store, aliases: &AliasMap, req: &AskRequest) -> Result<AskOutput>;
pub fn run_with_layers(store: &Store, aliases: &AliasMap, req: &AskRequest, units: &UnitsLayer, notes: &NotesLayer) -> Result<AskOutput>;
pub fn run_and_log(store: &Store, aliases: &AliasMap, req: &AskRequest, log: &LogOptions) -> Result<AskOutput>;
pub fn run_and_log_with_layers(store: &Store, aliases: &AliasMap, req: &AskRequest, log: &LogOptions, units: &UnitsLayer, notes: &NotesLayer) -> Result<AskOutput>;
pub fn status(store: &Store) -> Result<Value>;
pub fn status_with_layers(store: &Store, units: &UnitsLayer, notes: &NotesLayer) -> Result<Value>;
```

`run` becomes `let units = store.read_units()?; let notes = store.read_notes()?; run_with_layers(..)`; the body of today's `run` moves into `run_with_layers` untouched apart from using the passed layers. Same for `status`. `run_and_log` delegates to `run_and_log_with_layers` after reading. `UnitsLayer` and `NotesLayer` are the existing `crate::store` types. No other line in `ask/mod.rs` changes.

### 4.6 `src/watch/mod.rs`

```rust
pub struct WatchHandle { .. }

pub fn spawn(roots: IngestRoots, state: Arc<Mutex<WarmState>>) -> Result<WatchHandle, qol_watch::WatchError>;
```

Uses `qol_watch::settled(&[WatchRoot::deep(roots.pi.clone()), WatchRoot::deep(roots.claude.clone())], Duration::from_millis(250))`. A thread drains the receiver; each batch is filtered to paths ending in `.jsonl` that `crate::ingest::is_ignored(&roots, path)` rejects, then `ingest::ingest_paths(state.store(), &roots, &paths, state.keys())` runs under the state lock and its appended units are pushed with `push_units`. Errors go to stderr through `probe!("QOL_MEMORY_WATCH", ...)`; the thread never panics out. Dropping `WatchHandle` stops the watcher (drop the `Watch`, the receiver closes, the thread exits).

## 5. Ingest (lane `qm-ingest`)

### 5.1 `src/store/mod.rs` additions

```rust
impl Store {
    pub fn ingest_state_path(&self) -> PathBuf;
    pub fn continue_marker_path(&self) -> PathBuf;
    pub fn distill_lock_path(&self) -> PathBuf;
}
```

Paths: `ingest-state.json`, `continue.marker.json`, `.distill.lock` under the root. `Unit` gains `#[derive(serde::Serialize)]` with `#[serde(skip_serializing_if = "Option::is_none")]` on the optional fields so a round trip keeps snapshot.mjs field order (`key, source, file, session, cwd, kind, ts, text`). Nothing else in the file changes.

### 5.2 `src/store/lock.rs`

Port of `docs/research/qol-memory/lib/distill-lock.js`:

```rust
pub const STALE_AFTER: Duration = Duration::from_secs(600);

pub struct DistillLock { .. }

impl DistillLock {
    pub fn acquire(store: &Store, mode: &str) -> anyhow::Result<Option<DistillLock>>;
    pub fn acquire_wait(store: &Store, mode: &str, wait: Duration) -> anyhow::Result<DistillLock>;
}

impl Drop for DistillLock { .. }
```

`acquire` creates the file with `create_new`, writes `{"pid","started_at","mode"}` plus newline, and returns `Some`; on `AlreadyExists` it reads the file, deletes it when `started_at` is older than `STALE_AFTER` and retries once, otherwise returns `None`. `acquire_wait` polls `acquire` every 20 ms up to `wait` and errors with `qol-memory: store is locked by another writer` on timeout. `Drop` removes the file.

### 5.3 `src/ingest/mod.rs`

```rust
pub mod redact;
pub mod state;
pub mod transcript;

pub use redact::redact;

pub struct IngestRoots { pub pi: PathBuf, pub claude: PathBuf }

impl IngestRoots {
    pub fn resolve() -> IngestRoots;
    pub fn source_of(&self, path: &Path) -> Option<&'static str>;
}

pub struct KeySet { .. }

impl KeySet {
    pub fn load(store: &Store) -> anyhow::Result<KeySet>;
    pub fn contains(&self, key: &str) -> bool;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}

#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IngestReport { pub files: usize, pub appended: usize, pub duplicates: usize, pub reparsed: usize }

pub fn unit_key(source: &str, file: &str, ts: Option<&str>, text: &str) -> String;
pub fn is_ignored(roots: &IngestRoots, path: &Path) -> bool;
pub fn append_units(store: &Store, units: &[serde_json::Value], keys: &mut KeySet) -> anyhow::Result<usize>;
pub fn ingest_paths(store: &Store, roots: &IngestRoots, paths: &[PathBuf], keys: &mut KeySet) -> anyhow::Result<IngestReport>;
pub fn ingest_all(store: &Store, roots: &IngestRoots, keys: &mut KeySet) -> anyhow::Result<IngestReport>;
```

- `IngestRoots::resolve`: `QOL_MEMORY_PI_DIR` else `~/.pi/agent/sessions`; `QOL_MEMORY_CLAUDE_DIR` else `~/.claude/projects` (home via `std::env::var_os("HOME")`). `source_of` returns `"pi"` or `"claude"` by prefix match, `None` otherwise.
- `unit_key` is `sha256([source, file, ts.unwrap_or(""), text].join("|"))` hex, first 16 chars, byte-exact with `snapshot.mjs` `unitKey` (JavaScript `join` renders `null` as an empty string). Use the `sha2` dependency already present.
- `is_ignored` ports `isIgnored` with the default rules `**/*secret*`, `**/*token*`, `**/.env`, `**/.env.*`, `**/memory/` plus the optional `<store>/ignore` file (one rule per line, `#` comments); relative form replaces the pi root with `~pi` and the claude root with `~claude` before matching; a rule with `*` becomes an anchored regex with `.*` per star, otherwise substring match.
- `KeySet::load` reads `units.jsonl` through `Store::read_units` when present (sealed prefix included) and collects every `key`.
- `append_units`: skip values that are not objects or lack a string `key`, skip keys already in the set, then under `DistillLock::acquire_wait(store, "append", Duration::from_secs(2))` open `units.jsonl` in append mode, write each remaining value as one compact `serde_json` line, flush, insert the keys, and return the count appended. The file is created if absent, and `idx-*` files are left alone (the persisted index validates itself by prefix proof).
- `ingest_paths`: for each path, `source_of` must be `Some` and `is_ignored` false, else skip; load the per-file `state::FileState`; if the file shrank below the stored offset or its inode changed, reparse from byte 0 and count `reparsed`; otherwise parse from the stored offset with the stored `session` and `cwd` carried in; collect the units, `append_units`, write the new state (offset = file length after the parse, size, mtime, inode, session, cwd). Partial trailing lines (no newline) are not consumed; the offset stops before them.
- `ingest_all`: walk both roots exactly like `snapshot.mjs` `walk` (sorted names, depth limit 8, symlinks skipped, ignored dirs and files skipped, `.jsonl` only) and call `ingest_paths` with the full list.

The in-run normalized-text dedupe of `snapshot.mjs` (`pushUnit` `fam`) is deliberately not ported; `ask` already dedupes user units by normalized text at read time.

### 5.4 `src/ingest/redact.rs`

`pub fn redact(text: &str) -> String` applying the six `lib/redact.js` replacements in order with the `regex` crate: 32+ char `[A-Za-z0-9_-]` words to `[REDACTED]`; `(?i)(Bearer|Token|api[_-]?key|password|passwd|secret|private[_-]?key)\s*[:=]\s*\S+` to `$1=[REDACTED]`; `sk-[A-Za-z0-9]{20,}` to `[REDACTED-KEY]`; `-----BEGIN[\s\S]*?END [A-Z ]*-----` to `[REDACTED-PEM]`; emails to `[EMAIL]`; `\.env[\s\S]*` to `.env [REDACTED]`. Regexes are compiled once with `std::sync::OnceLock`. Word boundaries: Rust `regex` supports `\b`; the JavaScript `\b` is ASCII-based, so use `(?-u:\b)`.

### 5.5 `src/ingest/transcript.rs`

```rust
pub struct ParseCursor { pub offset: u64, pub session: Option<String>, pub cwd: Option<String> }

pub struct Parsed { pub units: Vec<serde_json::Value>, pub cursor: ParseCursor }

pub fn parse_file(path: &Path, source: &str, cursor: ParseCursor) -> anyhow::Result<Parsed>;
```

Ports `processFile` for both sources; the produced objects are byte-compatible with `snapshot.mjs` (same field set and order: `key, source, file, session, cwd, kind, ts, text`, plus `filesRead` and `filesModified` for pi compactions). `file` is the basename. Timestamps: numbers become ISO millis through a local helper in this module producing `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC, the `Date#toISOString` shape; `text::now_iso` shows the formatting used elsewhere), strings pass through, missing becomes `null`. Assistant units are never emitted (snapshot default). Tool results are skipped. `kind` values: `user`, `compaction`, `branch`. Unparseable lines are skipped. Reading starts at `cursor.offset` and stops at the last complete line.

### 5.6 `src/ingest/state.rs`

```rust
pub const SCHEMA: &str = "qol-memory-ingest-state-v1";

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileState { pub offset: u64, pub size: u64, pub mtime_ms: u64, pub inode: u64, pub session: Option<String>, pub cwd: Option<String> }

pub struct IngestState { .. }

impl IngestState {
    pub fn load(store: &Store) -> IngestState;
    pub fn get(&self, path: &Path) -> Option<&FileState>;
    pub fn set(&mut self, path: &Path, state: FileState);
    pub fn save(&self, store: &Store) -> anyhow::Result<()>;
}
```

JSON shape `{"schema": SCHEMA, "files": {"<absolute path>": FileState}}`; a missing or foreign-schema file loads as empty; `save` uses `qol_fs::atomic_write`. `inode` is `std::os::unix::fs::MetadataExt::ino` behind `#[cfg(unix)]` with 0 elsewhere, kept inside this module.

## 6. Continue recall (lane `qm-ingest`)

### 6.1 `src/continue_recall/mod.rs`

Port of `qol-skills/plugins/qol-project/bin/inject-qol-memory-continue.cjs` (path on this machine: `/media/kmrh47/WD_SN850X/Git/qol-skills/plugins/qol-project/bin/inject-qol-memory-continue.cjs`), identical constants (`SCHEMA`, `MIN_TEXT` 40, `MIN_DELTA` 2, `K` 3, caps user 2 / compaction 1, the four boilerplate markers via `crate::store::BOILERPLATE_MARKERS`).

```rust
pub const SCHEMA: &str = "qol-memory-continue-v1";

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ContinueRequest { pub cwd: String, pub session: String }

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContinueOutcome { pub stage: String, pub reason: Option<String>, pub count: usize, pub block: Option<String> }

pub fn run(store: &Store, request: &ContinueRequest) -> anyhow::Result<ContinueOutcome>;
```

Behaviour, in the hook's order: empty `cwd` or `session` gives `abstain` / `no-cwd`; `QOL_MEMORY_CONTINUE_DISABLE=1` gives `disabled` / `env`; `<store>/continue.disabled` gives `disabled` / `flag-file`; read the marker (`continue.marker.json`, foreign schema treated as empty); read `units.jsonl` raw (missing gives `abstain` / `read-error`); line count below the marker's `units_count` rewrites the marker and gives `gate-miss` / `store-reset`; text comes from `store::seal::try_sealed_text` else lossy UTF-8; `pick_units` filters and sorts exactly as the hook (kind user or compaction, trimmed text at least 40 chars, not boilerplate, not the same session, parseable ts newer than the marker ts, sort by ts desc then key asc, per-session caps, stop at K); the marker is rewritten (`{"ts","session","units_count","updated"}` under `cwds[cwd]`, pretty JSON two spaces plus newline, tmp + rename); `count >= MIN_DELTA` gives `injected` with `block` built by the hook's `block()` (header line with `anchorTs`, then `  NEW <ts> <kind> <session[..8]> <key[..8]> "<snippet 140 chars>"`), else `gate-miss` / `below-min-delta`. Every `hook.log` append the hook performs is reproduced (`{"ts", "stage", ...}` lines) because the hook's own tests read it. Marker `ts` uses `text::now_iso()`.

### 6.2 `docs/research/qol-memory/lib/merge.js`

`mergeUnits` acquires `acquireDistillLock(storeRoot, "merge")` from `./distill-lock.js` before reading `units.jsonl`; when the lock is held it retries every 50 ms for up to 5 s and then throws `qol-memory: store is locked by another writer`; the lock is released in a `finally`. No other behaviour changes.

## 7. Tests (each lane, in its own files, `#[cfg(test)]`)

qm-ingest: `unit_key_matches_snapshot_formula` (known vector: `unit_key("pi", "a.jsonl", Some("2026-01-01T00:00:00.000Z"), "hello")` equals the sha256 prefix computed in the test with the same join), `null_ts_joins_as_empty`, `redact_matches_js_vectors` (one input per rule), `pi_transcript_yields_user_and_compaction_units`, `claude_transcript_skips_tool_results`, `offset_resume_only_parses_new_lines`, `shrunk_file_reparses_from_zero`, `append_units_skips_duplicate_keys_and_holds_lock`, `lock_stale_after_ten_minutes_is_stolen`, `ignore_rules_match_snapshot_semantics`, `continue_injected_block_matches_fixture` (fixture store with three units, expected block text literal), `continue_gate_miss_below_min_delta`, `continue_store_reset_rewrites_marker`.

qm-daemon: `request_ask_uses_warm_layers`, `request_capture_appends_and_reports_count`, `request_unknown_action_is_fallback`, `warm_layers_refresh_after_units_change`, `push_units_keeps_cache_hit`, `reindex_rebuilds_three_layers`, the CLI tests in 4.1, and the manifest assertions in main.rs. Tests use a temp store under `std::env::temp_dir()` with a nanos suffix like the existing cli tests, and set `QOL_MEMORY_PI_DIR`/`QOL_MEMORY_CLAUDE_DIR` only through `IngestRoots { .. }` literals, never the process env.

## 8. Gate (architect only)

From the worktree with the shared target dir: `cargo fmt --all --check`; `cargo clippy -p qol-memory --all-targets -- -D warnings`; `cargo test -p qol-memory`; `node docs/research/qol-memory/parity.mjs --bin target/debug/qol-memory` at 73/73 with 0 mismatches; `env -u QOL_TRAY_HTTP_TOKEN qol check`; cross-target `cargo clippy -p qol-memory --target x86_64-apple-darwin` and `x86_64-pc-windows-msvc`.

## 9. Acceptance

1. `qol-memory ask "<q>"` and `qol-memory status` print byte-identical stdout with the daemon running and with it stopped (diff of both captures empty).
2. Writing a fixture transcript under a temp `QOL_MEMORY_CLAUDE_DIR` while the daemon runs yields the new units in `units.jsonl` within 2 s and `status` reflects the new count.
3. Two concurrent `qol-memory capture` calls plus one `mergeStep` leave `units.jsonl` parseable with no duplicate keys and no lost line.
4. `qol-memory continue --cwd X --session Y` prints the same text as the hook for the same store and marker fixture, and both leave the same marker file.
5. The daemon exits on `kill` over the socket and when qol-tray dies (host-death watchdog from the daemon library).
6. `POST /api/mcp tools/list` on the dev tray lists `qol-memory__ask` and `qol-memory__status`, and `tools/call qol-memory__ask` returns the ask output as structured content.
