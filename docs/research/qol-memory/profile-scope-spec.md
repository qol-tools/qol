# Agent homes: one registry for every harness profile

Revision 3 (after the review board). Changes from revision 2 are marked
"(r3)"; every lane reads this file as the contract.

## Goal

qol tools keep meeting the same fact in different places: where an agent
harness (Claude Code, codex, kimi, pi) keeps its config and transcripts, and
which env var points a process at an alternative home (`CLAUDE_CONFIG_DIR`
for `clauded`, `claudedw`, `claudedwb`). Today that knowledge is copied into
qol-terminal-sessions builtins, `qol mcp configure`, qol-memory ingest, and
the qol-skills bridge scripts, and each copy differs.

This spec makes one crate the source of truth for agent homes, one host file
the source of truth for the homes a machine declares, and one store the
source of truth for memory data, tagged by home. qol-memory then partitions
memory by home: a personal Claude Code instance never reads work transcripts
and the other way round, on the same tray, with one store.

Threat model (r3): this is a privacy partition inside one user account and
one tray token. Every process running as the user can name any home; the
partition prevents accidental cross-home recall, not a hostile local
process. Per-home credentials are out of scope and recorded as a follow-up.

Naming (r3): the concept is an agent home everywhere. The word "profile"
never appears in new code, inputs, fields, flags, headers, or docs, because
the tray already owns a profile feature (settings sync). The wire argument,
the unit field, the CLI flag, the request input and the ask output key are
all `agent_home`; the qol-memory module is `src/agent_home/mod.rs`.

## Layer 1: crate `libs/qol-agent-homes` (lane qm-homes)

Workspace member, `[workspace.dependencies]` entry `qol-agent-homes = { path = "libs/qol-agent-homes" }`.
Dependencies: `serde`, `toml`, `dirs`, `qol-config` (for `config_dir()`).
No dependency on qol-terminal-sessions.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness { Claude, Codex, Kimi, Pi }

impl Harness {
    pub const ALL: [Harness; 4];
    pub fn id(self) -> &'static str;                 // "claude" "codex" "kimi" "pi"
    pub fn parse(text: &str) -> Option<Harness>;
    pub fn home_env_var(self) -> &'static str;       // CLAUDE_CONFIG_DIR, CODEX_HOME, KIMI_CODE_HOME, PI_CODING_AGENT_DIR
    pub fn default_home(self, user_home: &Path) -> PathBuf;  // .claude, .codex, .kimi-code, .pi/agent
    pub fn transcripts_dir(self, home: &Path) -> Option<PathBuf>;  // (r3) claude: projects; codex, pi: sessions; kimi: None (no transcript directory is known)
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AgentHome {
    pub harness: Harness,
    pub id: String,        // normalized absolute path, the identity used everywhere
    pub path: PathBuf,
    pub shared: bool,      // every caller may read this home's memory
    pub default: bool,     // the home assumed for this harness when a request names none
    pub declared: bool,    // (r3) true for entries from agents.toml, false for builtin implicit entries
}

pub struct Registry { .. }

impl Registry {
    pub fn load() -> Registry;   // config_dir()/agents.toml, HOME, real env
    pub fn load_from(file: Option<&Path>, user_home: &Path, env: &dyn Fn(&str) -> Option<OsString>) -> Registry;
    pub fn homes(&self) -> &[AgentHome];
    pub fn current(&self, harness: Harness) -> AgentHome;
    pub fn default_for(&self, harness: Harness) -> &AgentHome;
    pub fn is_shared(&self, id: &str) -> bool;
    pub fn is_registered(&self, id: &str) -> bool;          // (r3) some home in homes() has this id
    pub fn is_partitioned(&self) -> bool;                   // (r3) some harness has more than one home
    pub fn env_home(&self, harness: Harness) -> Option<&str>; // (r3) the normalized env var value when non-empty
    pub fn load_error(&self) -> Option<&str>;               // (r3) the parse error of an unreadable or malformed agents.toml
    pub fn resolve_caller(&self, explicit: Option<&str>) -> String;
    pub fn transcript_roots(&self) -> Vec<(AgentHome, PathBuf)>;
}

pub fn normalize(text: &str, user_home: &Path) -> String;
pub const REGISTRY_FILE_NAME: &str = "agents.toml";
```

Rules:

- `normalize`: trim, expand a leading `~` or `~/` with `user_home`, join a
  relative path onto `user_home` (r3), drop every trailing separator (r3,
  `/` everywhere and `\` on Windows) unless the result would be empty or a
  bare root, keep everything else verbatim. Ids compare as strings.
- File format, `~/.config/qol-tray/agents.toml` (the qol-config
  `config_dir()`; one machine-level file, never install-scoped; when
  `config_dir()` is None the registry holds only implicit defaults):

```toml
[[home]]
harness = "claude"
path = "~/.claude-work"
shared = false
default = true
```

  `shared` and `default` are optional (false). Unknown harness names and
  blank paths are skipped. Missing file means no declared homes. A file that
  exists but cannot be read or parsed also means no declared homes, and
  `load_error()` carries the error text (r3).
- Implicit defaults (r3): every harness always has its builtin default home
  in `homes()`, marked `declared = false`. A declared entry whose id equals
  the builtin path replaces the implicit entry (its flags win). The builtin
  entry is `default = true` unless a declared entry of that harness carries
  `default = true`; when several declared entries claim default, the first
  wins and the others are cleared. Implicit pi is `shared = true`, the other
  implicit entries `shared = false`. Declared order is kept, implicit entries
  come last.
- `current(harness)`: the harness env var when non-empty (normalized; if it
  matches a home in `homes()` that home is returned, otherwise an ad hoc
  `AgentHome { shared: false, default: false, declared: false }`), else
  `default_for`.
- `resolve_caller(explicit)`: explicit value normalized when non-empty,
  else `current(Harness::Claude).id`.
- `transcript_roots`: every home whose harness has a transcripts dir, paired
  with `harness.transcripts_dir(&path)`; pi additionally honours the env var
  `PI_CODING_AGENT_SESSION_DIR` (r3): when non-empty, the current pi home's
  root is that directory instead of `<home>/sessions`. Kimi contributes no
  root.
- Tests for each rule with a temp file, a fake env closure, and a fake user
  home. `Registry::load` is the only function that touches the real
  environment. Required r3 tests: adding a declared claude home keeps the
  builtin claude home in `homes()` and in `transcript_roots()`; a declared
  entry at the builtin path replaces it; `is_partitioned` is false with only
  implicit homes and true with one extra claude home; a malformed file sets
  `load_error`; a relative declared path joins onto the user home; trailing
  `//` normalizes like `/`.

Lane qm-homes also moves the harness home rules in
`libs/qol-terminal-sessions/src/cli/builtins/{claude,codex,kimi,pi}/` onto the
crate: `claude/environment.rs` and `claude/metadata.rs` derive the home from
`Registry::load().current(Harness::Claude).path`, `pi/environment.rs`
`agent_dir()` and `session_dir_override()` (r3: the override comes from the
registry's pi transcript root), `kimi/environment.rs` `kimi_home()`,
`kimi/metadata.rs` `kimi_subscription_home()`, and `codex/environment.rs`
`session_index_path()` all call the crate; local `expand_tilde` helpers that
become unused are deleted. The registry home replaces the old
`$HOME/.claude` prefix entirely: `claude/environment.rs` joins `sessions`
and `projects` directly onto `current(Harness::Claude).path`, never a second
`.claude` segment (r3), with a registry-aware test. Existing tests keep passing.

## Layer 2: `qol agents` CLI (lane qm-agents-cli)

Owned: `tools/qol-cli/src/commands/agents/mod.rs`, `tools/qol-cli/src/commands/mod.rs`, `tools/qol-cli/src/cli/contract.rs`, `tools/qol-cli/Cargo.toml`, `docs/agent-homes.md`.

- `qol agents list [--json]`: every home from `Registry::load()`, plus, for
  each harness whose `env_home` is set but not registered, one row for that
  ad hoc home (r3). Plain rows are tab-separated (r3): `harness`, `id`,
  `shared` or `-`, `default` or `-`, `declared`, `implicit` or
  `unregistered`. JSON is `{"homes": [AgentHome...]}` (r3, same shape as
  `GET /api/agents`), with unregistered rows carried as `AgentHome` values
  with `declared = false`. When `load_error()` is set, plain output prints a
  warning line to stderr naming the file and the error, and JSON adds
  `"error": "<text>"`.
- `qol agents current <harness> [--json]`: prints `current(harness).id`
  (JSON: the `AgentHome`). Scripts call this.
- `qol agents add <harness> <path> [--shared] [--default]`: appends or
  updates the `[[home]]` entry with `toml_edit`, creating the file;
  `--default` clears `default` on that harness's other entries; prints the
  resulting row so a flag change is visible (r3).
- `qol agents remove <path>`: normalize, then remove every entry with that
  id regardless of harness (r3); error listing nothing when absent.
- `docs/agent-homes.md` (r3 additions): the implicit-entry rule above, the
  work PC recipe (`qol agents add claude ~/.claude-personal` when the work
  instance owns `~/.claude`, the reverse otherwise), pre-registration
  semantics (an undeclared env home is never ingested and sees only shared
  units until declared), why pi is shared by default (pi lanes are
  subordinate to the session that spawned them and pi has no per-instance
  home convention) and how to make it private, the upgrade note (codex and
  kimi units become private to their own homes), the re-run rule for
  `qol mcp configure codex|kimi` after re-homing (their headers are static),
  paths as identity (moving a home orphans its units until re-added), the
  fail-closed behaviour of Layer 3 and 5, and the sessions limitation: the
  sessions builtins resolve one claude home per observing process (the
  tray's or watcher's own env), while memory ingests every registered home.

## Layer 3: identity on the wire (lane qm-caller)

Owned: `libs/qol-mcp/src/handler.rs`, `libs/qol-mcp/src/lib.rs`, `libs/qol-conventions/src/lib.rs`, `apps/qol-tray/src/features/mcp/handlers.rs`, `apps/qol-tray/src/features/mcp/tool_host.rs`, `apps/qol-tray/src/features/agents/mod.rs`, `apps/qol-tray/src/features/mod.rs`, `apps/qol-tray/src/features/plugin_store/server/mod.rs`, `apps/qol-tray/src/features/plugin_store/server/plugin_handlers.rs` (r3), `apps/qol-tray/Cargo.toml`, `tools/qol-cli/src/commands/mcp/mod.rs`, `tools/qol-cli/src/commands/mcp/configure.rs`, `tools/qol-cli/src/commands/sessions/mcp.rs`, `docs/plugin-contract.md`.

- `qol_conventions::HTTP_AGENT_HOME_HEADER: &str = "x-qol-agent-home"` next to `HTTP_AUTH_HEADER`.
- `qol_mcp`: `pub struct Caller { pub agent_home: Option<String> }`;
  `ToolHost::call(&self, name, arguments, caller: &Caller)`;
  `handle(host, message, caller)`; export `Caller`.
- Reserved argument (r3): the runable input name `agent_home` is host
  managed. `PluginToolHost` never publishes it: the tool spec's
  `properties` and `required` omit it, and the binding remembers
  `accepts_agent_home`. On `call`, when the binding accepts it, the host
  sets `arguments["agent_home"]` from the caller (a caller-supplied value is
  overwritten); a null `arguments` value is treated as `{}`.
- Fail closed on MCP (r3): when the binding accepts `agent_home`, the caller
  has none, and `Registry::load().is_partitioned()` is true, the host returns
  a tool error "caller identity missing: run qol mcp configure <harness>"
  without calling the plugin. With a single home per harness the plugin
  resolves its default as today.
- Tray `post_message` takes `HeaderMap`, builds the `Caller` from the header
  (trimmed, non-empty), passes it to `handle`. The plain routes
  `/api/plugins/{id}/queries/{query}` and `/actions/{action}` (r3) copy the
  same header into `input["agent_home"]` when present and the runable
  accepts it; without the header they forward the body unchanged.
- Tray `GET /api/agents`: `{"homes": [AgentHome...]}` from `Registry::load()`,
  token-protected, registered from `features/agents/mod.rs`, merged next to
  the other feature routes. One handler test.
- `qol mcp headers` output adds `x-qol-agent-home` = `Registry::load().current(Harness::Claude).id`.
- `qol mcp configure` (r3): the local `Harness` enum, `profile_for`,
  `env_or_home` and the env reads are deleted; `qol_agent_homes::Harness`
  is used directly. Config file paths: claude is
  `<env_home>/.claude.json` when `env_home(Claude)` is set else
  `~/.claude.json`; codex `current(Codex).path/config.toml`; pi
  `current(Pi).path/mcp.json`; kimi `current(Kimi).path/mcp.json`. Headers:
  claude keeps `headersHelper` only; pi gets the command header value
  `"!qol agents current pi"` next to `"!qol mcp token"`; codex and kimi get
  the static value `current(harness).id` and the command prints a line
  naming the baked id with the re-run rule. Shape tests updated; no test
  mutates the process environment.
- `docs/plugin-contract.md`: one paragraph in the MCP section: the header,
  the reserved `agent_home` input, that it is never published to agents,
  the fail-closed rule, and that plugins must not declare `agent_home` for
  any other meaning.

## Layer 4: qol-memory (lanes qm-roots and qm-visibility)

Data stays in one store; every unit carries `agent_home: Option<String>`
holding the home id that produced it. Legacy units without it map by source:
`pi` to the default pi home, anything else (including the pre-r3 `agent`
captures) to the default claude home; that mapping is a documented decision.

### Lane qm-roots

Owned: `plugins/qol-memory/Cargo.toml`, `plugins/qol-memory/qol-runtime.toml`, `plugins/qol-memory/src/lib.rs`, `plugins/qol-memory/src/agent_home/mod.rs` (r3, renamed from `profile`), `plugins/qol-memory/src/store/mod.rs`, `plugins/qol-memory/src/ingest/mod.rs`, `plugins/qol-memory/src/ingest/transcript.rs`, `plugins/qol-memory/src/watch/mod.rs`, `plugins/qol-memory/src/app/mod.rs`, `plugins/qol-memory/src/doctor/mod.rs`.

- `src/agent_home/mod.rs` (r3 rename): `unit_home(unit, registry) -> &str`,
  `visible(unit, caller, registry) -> bool`, `cache_slug(caller) -> String`
  (8 lowercase hex of sha256). No `profile` identifier remains anywhere in
  the plugin.
- `Unit.agent_home: Option<String>` with serde default and skip-if-none (r3 rename).
- `IngestRoot { path, source, agent_home }`; `IngestRoots::from_registry`
  builds one root per `transcript_roots()` entry whose harness has a parser
  (r3: only claude and pi, `SUPPORTED_SOURCES`); `source` is the harness id.
  Env overrides `QOL_MEMORY_PI_DIR` and `QOL_MEMORY_CLAUDE_DIR` replace that
  harness's roots with one root whose home id is
  `registry.current(harness).id` (r3). `source_of(path)` returns source and
  home id; every ingested unit carries it.
- `qol-runtime.toml`: `ask`, `continue`, `rows`, `capture` declare the input
  `agent_home` ("Agent home id of the caller; supplied by the host, never by
  the agent") (r3 rename).
- doctor `agent_homes` (r3): iterates `IngestRoots::from_registry` roots;
  a home whose path does not exist is listed as absent inside the ok text;
  warn when a home path exists but its root is not a directory; warn when
  `registry.load_error()` is set (naming the file); warn when
  `current(harness)` for claude or pi is not registered ("register it with
  qol agents add"); the ok text states the visibility rule in one clause.
- doctor `index_cache` (r3): checks the default caller's layer
  `user-<slug>` over visible units and names that home.

### Lane qm-visibility

Owned: `src/ask/mod.rs`, `src/ask/rows.rs`, `src/retrieval/cache.rs`, `src/retrieval_log/mod.rs` (r3), `src/continue_recall/mod.rs`, `src/app/request.rs`, `src/app/warm.rs` (r3), `src/cli.rs`, `docs/research/qol-memory/parity.mjs` (r3), `docs/research/qol-memory/test-e2e.mjs` (r3: its index-file assertions match the slugged names, for example `idx-pool-*.json`).

- `AskRequest.agent_home: Option<String>`; `run_with_layers` loads
  `Registry::load()` once, resolves the caller with `resolve_caller`, filters
  every unit list that feeds an index or the answer pool with `visible`; the
  output JSON gains `"agent_home": <caller id>`; `RetrievalEvent` gains
  `agent_home` (r3).
- Index caches are per caller: layer names get the suffix `-<cache_slug>`;
  `status_with_layers` and `warm::reindex` use the default caller's layers (r3).
- Notes: visible when the unit named by `source_key` is visible; notes whose
  `source_key` names no unit stay global (documented decision).
- `ContinueRequest.agent_home`; candidates filtered with `visible`; the
  continue marker is keyed by cwd and caller id (r3): one watermark per
  caller per cwd, an existing cwd-only marker seeds the first per-caller entry.
- `capture` (request.rs and cli.rs) force-stamps `unit.agent_home` with the
  resolved caller on every path (r3); a value inside the unit object is
  overwritten.
- `rows` passes `agent_home` through to ask.
- `cli.rs` (r3): `--agent-home <dir>` on ask, continue, rows, capture; when
  the flag is absent the CLI resolves `Registry::load().resolve_caller(None)`
  in its own process and always sends an explicit value over the socket, so
  a shell with `CLAUDE_CONFIG_DIR` set answers from that home.
- `parity.mjs` (r3): strips the `agent_home` key from the Rust output before
  diffing; `ask.mjs` stays untouched.
- Tests: ask returns only the caller's and shared units; continue filters
  the same and keeps per-caller watermarks; capture stamps and overwrites;
  the slug suffix appears in the index file name; the CLI sends an explicit
  home when the flag is absent.

## Layer 5: bridge scripts (lane qm-bridge2, repo qol-skills)

Owned: `plugins/qol-memory/bin/inject-qol-memory-continue.cjs`, `plugins/qol-memory/bin/agent-home.cjs`, `plugins/qol-memory/.pi/extensions/qol-memory-tool.ts`, `plugins/qol-memory/test/*`, `plugins/qol-memory/.claude-plugin/plugin.json` (version 0.1.0 to 0.1.1 only).

- `bin/agent-home.cjs` exports `agentHome(harness, timeoutMs)`: runs
  `qol agents current <harness>` with `execFile`, returns the trimmed stdout
  on exit 0, `null` on any failure or timeout. No env rule in JavaScript.
- Hook (r3): the continue body is `{cwd, session, agent_home}`; when
  `agentHome('claude', 800)` returns null the hook posts nothing, writes one
  stderr line "qol-memory: caller identity unavailable (qol agents current
  claude failed); continue skipped" and exits 0.
- pi extension (r3): `agentHome('pi', 800)`; when available the capture
  POST sends `{unit, agent_home}` and the unit carries `agent_home` for the
  direct-append fallback; when null the POST is skipped and the direct
  append writes the unit without a home.
- Tests: the helper with a fake `qol` on PATH returning a value, a non-zero
  exit, and a timeout; the hook body with the value and the skipped post
  without it.

## Prohibitions (every lane)

Edit only the owned paths. Never run cargo build, cargo test, cargo clippy,
cargo fmt, npm test, node scripts, or any git command. Add no code comments.
Use no em-dash character anywhere.

## Accepted risks and follow-ups (r3)

- Same-user processes can name any home (threat model above).
- Static codex and kimi headers go stale after re-homing (documented).
- `units.jsonl` mode, per-home purge on `qol agents remove`, redaction
  breadth, the JS store-path copy in the bridge scripts, per-corpus BM25
  gate calibration, and a doctor report of units whose home matches no
  registered home are follow-ups, not part of this round.
- Kimi and codex transcripts are not ingested until parsers exist.

## Acceptance (architect)

- monorepo: `cargo fmt --all --check`; clippy `-D warnings` for qol-agent-homes, qol-terminal-sessions, qol-mcp, qol-conventions, qol-tray (with `--features dev`), qol, qol-memory; tests for the same; `qol check`; darwin and windows clippy for the ring-free crates; `node docs/research/qol-memory/parity.mjs` stays green.
- Live: `qol agents list` shows the implicit defaults; `qol agents add claude ~/.claude-work` keeps `~/.claude` listed; `CLAUDE_CONFIG_DIR=~/.claude-work qol agents current claude` prints that id; `qol mcp headers` prints both headers; `tools/list` on `/api/mcp` publishes no `agent_home`; two claude homes with one fixture transcript each: ask under each home's env returns only its own plus shared units; `qol-memory --json doctor` lists `agent_homes`.
- qol-skills: `npm test`, `node scripts/sync-plugin-manifests.cjs`.
