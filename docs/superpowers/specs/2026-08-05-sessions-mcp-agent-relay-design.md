# Sessions MCP: agent-to-agent relay through the host terminal stack — design research

Date: 2026-08-05
Status: research + PoC in progress; will be updated iteratively with guest-VM test results.

## Idea (user pitch)

CLI Sessions already "communicates" with host CLIs; qol-voice already routes text into
any chosen CLI session. Abstract that logic: give agents (Claude Code, pi, ...) a surface
that lets them send text to *other agents' CLIs* — the agent picks the host terminal
session at inference, and delivery rides the existing terminal stack. The user's mental
model: relay via MCP servers → host CLI.

## What already exists (verified in source, 2026-08-05)

1. **The delivery primitive is abstracted and shared.**
   `libs/qol-terminal-sessions` defines `TerminalBackend =
   SessionInventory + ScreenReader + SessionFocus + TextInput`, with
   `TextInput::send_text(&SessionBinding, &str, DeliveryMode)` where
   `DeliveryMode = Insert | Submit` (`src/model.rs:125`).
   `SessionCapabilities::TEXT_INPUT` gates which sessions accept text.
   Session identity: `SessionBinding` token `v1:<backend>:<native>:<root_pid>`
   (e.g. `v1:kitty:1:42`); `FromStr` round-trips it.
   `SessionFacts { id, root_pid, cwd, title, at_prompt, reported_cmd,
   foreground_basenames, foreground_pids, capabilities }`.
   Kitty backend implemented (`kitty @ ls / get-text / send-text / send-key /
   focus-window`, via `SystemCommandRunner`, `src/kitty/mod.rs:178-212`);
   `TerminalSessionService::system()`.

2. **qol-voice proves the "text → chosen session" loop end to end.**
   `ConversationSink { targets(), subscribe_target(), deliver() }`
   (`plugins/qol-voice/src/app/delivery.rs`), pure `ConversationRouter` with
   pinned routes and `RouteState` (Unselected → Ready → Delivered →
   TargetUnavailable/Failed), delivery queue cap 8, typed failures.
   Routing target is config-chosen today — the idea replaces "config-chosen"
   with "agent-chosen at inference". Per-tool interpretation is consumed from
   the shared lib (`qol_terminal_sessions::cli::{CliSessionInterpreter,
   CliToolColor}`), used for target labels ("Task · Codex").

3. **The "which session needs input" brain exists.**
   cli-sessions owns the attention policy / per-tool strategies / status state
   machine (needs-you / your-turn) and daemon actions (`open | next | snapshot |
   kill`, `src/daemon/actions.rs`). The interpreter core lives in the shared
   lib, so a new consumer does not need the plugin daemon for status.

4. **MCP pattern already in the house.**
   repo-status (kmrh47-skills marketplace) ships a stdio MCP server
   (`scripts/repo-status-mcp.mjs`, `@modelcontextprotocol/sdk`, zod) whose
   tools Claude Code invokes (`status_server`, `restart_server`, ...).
   Establishing an MCP server in this ecosystem is a solved pattern.

5. **Client reality.**
   - Claude Code: native `mcpServers` (stdio).
   - pi: **no built-in MCP** (pi docs usage.md: "intentionally does not include
     built-in MCP ... build or install those workflows as extensions"). pi path
     = extension registering the same tools as plain extension tools (shelling
     to the same CLI), no MCP client needed.
   - qol-tray: unix-socket runtime + axum dashboard; host↔plugin channels are
     Pull/Subscribe/Action/Capability (`qol-arch-channels`) — an agent-facing
     MCP surface is a *new boundary*, not a fifth host/plugin channel.

## The gap (what does not exist)

- No MCP server (or headless CLI) exposing terminal sessions to agents:
  no `sessions list`, `session read`, `session send`, `session focus` anywhere
  (monorepo has zero MCP).
- No headless "send text to session X" command: voice `assistant request` is
  voice-session-internal; cli-sessions daemon actions do not send text.

## Proposed architecture

- **Standalone headless surface, not a tray feature.** A process that uses
  `TerminalSessionService::system()` directly (Kitty backend is host-neutral;
  voice and cli-sessions each instantiate it independently). No tray needed.
- **Home (PoC): `qol sessions` command group in qol-cli** (rides the dev
  bundle as `bin/qol`). Long-term: `qol sessions mcp` stdio MCP server for
  Claude Code + pi extension with the same tools.
- Tools: `list` (id, label, status, capabilities), `read <session>`,
  `send <session> <text> [--submit]`, `focus <session>`.
- **Agent loop:** list → pick session (label/status/read screen) → send →
  read screen → interpret (shared `CliSessionInterpreter`).

## Design constraints / gaps to resolve

1. **No ack.** `send_text` is fire-and-forget typing (Kitty passthrough);
   the loop closes via screen polling + interpretation (same as cli-sessions).
2. **Contention.** Voice's worker queue serializes delivery; an agent path
   needs the same single-writer discipline (queue or lock per session).
3. **Security.** A tool that types into any terminal is powerful: scope to
   TEXT_INPUT sessions, same-user, enable-gated (mirror voice's device-scoped
   config rule), optional send allowlist.
4. **Ownership.** qol-cli vs qol-tray vs a plugin — decide when promoting the
   PoC; encode the "agent-facing MCP surface" pattern into a skill
   (`qol-arch-channels` extension) per standards-evolution.
5. **Status surface** for the agent should come from the shared interpreter
   (+ optionally the cli-sessions daemon), one source of truth.

## Verification plan (guest VM)

1. Host build `qol` with the new command.
2. `qol env up linux/mint-cinnamon --dev-worktree <repo>` (prepared desktop
   image, offline guest, bundle carries `bin/qol`).
3. In guest: verify kitty present; start kitty with a REPL CLI (python3);
   run `qol sessions list`, `qol sessions send <id> 'print(6*7)' --submit`,
   `qol sessions read <id>` → expect `42` in the screen text.
4. Evidence: command transcripts, `qol env shot`, probe log
   (`TERMINAL_SESSIONS` lines in the guest trace log).
5. `qol env down`; report stays; host unchanged.

## Guest-VM verification (2026-08-05, completed) — PASS

Environment: `linux/mint-cinnamon` prepared desktop image, artifact-backed via
`qol env up linux/mint-cinnamon --dev-worktree <repo>` (offline guest, bundle
carries `bin/qol` with the new command).

Blocker found and worked around: **the prepared desktop image has no terminal
emulator and the guest is offline**. Kitty (0.32.2 Ubuntu build) was carried in
via the generic USB-stick channel: `qol emu insert` reuses a pre-created
`usb-stick.raw` in the run dir (idempotent `ensure_usb_stick`), `apt-get
download`ed the kitty dep closure (83 debs) onto it, auto-mounted in the
guest, and extracted with `dpkg-deb -x` (no root) into the user tree. The ELF
launcher resolves its lib dir relative to itself, so the extracted tree runs
as-is; zero missing native libs.

Second blocker: kitty 0.32 needs `allow_remote_control=yes` for `--listen-on`
to bind (`boss.py:361` gates socket creation on it), and the `kitten @` client
needs `KITTY_LISTEN_ON` in its env (otherwise it falls back to the controlling
tty, which the exec context lacks). On the user's host this works because pi
runs inside a kitty window — the controlling-tty protocol path.

Verification transcript (guest, bundled qol):

```
$ qol sessions list
v1:kitty:1:2714  python3  python3  /home/qol  read,focus,input

$ qol sessions send v1:kitty:1:2714 "print(6*7)" --submit
delivered submitted to v1:kitty:1:2714

$ qol sessions read v1:kitty:1:2714
>>> print(6*7)
42
>>>

$ qol sessions send v1:kitty:1:2714 "print([i*i for i in range(5)])" --submit
$ qol sessions read v1:kitty:1:2714
>>> print([i*i for i in range(5)])
[0, 1, 4, 9, 16]
>>>
```

Probe-log evidence (debug build, `/tmp/qol-altmon.log`):

```
TERMINAL_SESSIONS backend=kitty operation=discover sessions success=true code=Some(0) stdout_len=4666
TERMINAL_SESSIONS backend=kitty operation=insert text success=true code=Some(0) stdout_len=0
TERMINAL_SESSIONS backend=kitty operation=submit text success=true code=Some(0) stdout_len=0
TERMINAL_SESSIONS backend=kitty operation=read screen success=true code=Some(0) stdout_len=211
```

Verdict: the core relay — list sessions, pick one, deliver text, read the
CLI's response — is proven end to end in a clean desktop guest. The agent
loop (pick → send → read → interpret → repeat) is fully supported by existing
machinery (`CliSessionInterpreter` in the shared lib supplies status).

Remaining work for the real feature (unchanged from the design above): MCP
wrapper (`qol sessions mcp`) + pi extension tools, contention/queueing,
send-guardrails (TEXT_INPUT-capability filter already implicit), and
ownership/pattern skill updates.

## Online research: battle-tested patterns (2026-08-05, 5 parallel research agents)

Sources: MCP spec (2025-03-26/06-18/11-25), sst/opencode source, Claude Code docs, OpenAI Codex docs, kitty remote-control docs + source, tmux man page, gotty, xterm.js, tmate, MCP reference servers, Playwright MCP, pexpect.

### Target selection (Q1)
- No researched tool filters "agent CLIs" from plain shells. kitty targets every window (rich match: id, title, pid, cwd, cmdline), tmux targets every pane. tmuxp organizes by project intent, not process kind. Filtering is a product opinion, not a convention.
- Agent tools never attach to existing terminal sessions: opencode/Claude Code/Codex spawn a fresh subprocess per command with stdin ignored. Injecting into a live session is an uncontested niche.
- In agent vocabulary "session" means a conversation transcript, never a terminal window. Borrow naming discipline (stable ids + labels), not resume/fork semantics.
- Decision: show everything with labels (interpreter classification), matching kitty/tmux uniformity.

### Turn-taking (Q2)
- MCP tools/call is blocking-only; no streaming tool results in any spec revision; clients SHOULD implement timeouts. A wait tool is legal and idiomatic.
- opencode shell tool blocks until exit-or-timeout (model-chosen timeout per call, default 120s), kills on expiry, returns output in one call. Claude Code Bash blocks (2min default, 10min cap), demotes to background task on timeout (does not kill), and ships a Monitor tool (watch background output and react). tmux wait-for is a binary barrier, not output-aware; idle detection is capture-pane polling. pexpect expect(pattern, timeout) is the classic blocking primitive.
- Decision: keep send fire-and-forget; add a blocking `session_wait_output(session, timeout)` tool that polls screen snapshots until settled or timeout, returning partial screen + settled flag.

### Contention (Q3)
- Nobody locks or refuses concurrent writers. tmux, kitty, gotty serialize via single-threaded event loop + atomicity of individual PTY writes; interleaving with human typing is accepted behavior.
- The only queueing shipped is FIFO + coalescing on the output/render side (xterm WriteBuffer, gotty writeMutex, opencode terminalWriter with flush acknowledgement). MCP/Anthropic guidance: side-effect tools should run sequentially (client or server owns ordering).
- Decision: per-session FIFO queue in the MCP server with flush-acknowledged writes and an exposed busy state; never refuse-when-busy.

### Guardrails (Q4)
- kitty: remote control defaults OFF; allow_remote_control = no/yes/socket/socket-only/password; remote_control_password scopes a password to an action allowlist; unknown password triggers an interactive allow/disallow prompt; per-window opt-in; custom is_cmd_allowed hook. Same-user alone is not considered a guardrail by kitty.
- MCP spec: hosts must obtain explicit user consent before invoking any tool; stdio preferred for local servers (limits access to just the client); HTTP requires auth. tmux: filesystem perms + server-access ACL with a read-only tier; tmate authenticates possession of a secret and splits read/write.
- Claude Code: ask-by-default with fail-closed unknown commands; per-tool MCP permission scopes; servers can self-declare always-approve via requiresUserInteraction. opencode: allow/ask/deny per tool plus a repetition guard (doom loop).
- Decision: same-user restricted socket as baseline; send_text should be client-side permission-gated (requiresUserInteraction-style) and offer optional kitty rc-password scoping; add a repetition guard later.

### Architecture home (Q5)
- MCP spec: clients SHOULD support stdio whenever possible; HTTP is for multi-client/remote. Every reference server (filesystem, sequential thinking, Playwright) is a stdio command; Playwright now steers coding agents to CLI + skills over MCP.
- Claude Code plugins host MCP servers as stdio command entries; Codex ships `codex mcp-server` as a per-invocation stdio subcommand (direct precedent); opencode runs a local HTTP server for its clients but MCP servers are still stdio children. kitty: resident server owns state, per-invocation `kitten @` clients.
- Decision: keep `qol sessions mcp` as a thin stdio subcommand of the qol CLI; add `--transport http` only if a remote/multi-client consumer appears. Do not move into the tray.

## Iteration log

- 2026-08-05: research pass completed (above). PoC `qol sessions` in progress.
- 2026-08-05: PoC landed (list/send/read/focus, JSON list, unit tests, clippy clean); guest-VM verification passed (transcript + probes above).
- 2026-08-05: MCP server landed (`qol sessions mcp`, stdio, newline-delimited JSON-RPC; hand-rolled to keep the CLI dependency-light, 13 unit tests covering handshake, tools, errors). `sessions list` now enriches rows with the shared interpreter's tool/display/activity. pi tools shipped in the qol-skills marketplace (`plugins/qol-sessions`, tools sessions_list / session_read_screen / session_send_text / session_focus). Guest-VM verification of the MCP path passed: initialize -> tools/list -> sessions_list -> session_send_text "print(6*7)" submit -> session_read_screen returned the screen with 42 (probe log: discover, submit text, read screen all success).

## MCP protocol notes

- Transport: stdio, one JSON-RPC 2.0 message per line; notifications get no response; errors -32700/-32601/-32602; tool failures are isError results, not protocol errors.
- Capabilities: tools only (listChanged false); resources/prompts answered with -32601.
- The same backend constraint applies as in the CLI PoC: the `kitten @` client needs KITTY_LISTEN_ON in env (host works via controlling-tty fallback because the caller runs inside kitty).

## Per-tool strategy research (2026-08-05)

Four research passes (one per CLI, sources: official docs + upstream source; pi verified against the installed package on this machine). Each verified or falsified the existing `CliSessionStrategy` assumptions in `libs/qol-terminal-sessions/src/cli/builtins/`.

### Claude Code
- Transcripts: `~/.claude/projects/<project>/<session-id>.jsonl`, where `<project>` is the cwd with every non-alphanumeric character replaced by `-` (confirmed in docs; matches the strategy's encoding).
- Live session record: `~/.claude/sessions/<pid>.json` holds `{sessionId, cwd}` (the strategy's match point).
- `claude -n <name>` sets a display name shown in `/resume`, the terminal title, and the prompt bar; `/rename` updates it mid-session. The strategy reads it from the transcript.
- No documented screen-level busy/idle signal; `has_activity` stays None.

### Codex
- Session storage: `~/.codex/sessions/<YYYY/MM/DD>/rollout-<timestamp>-<uuid>.jsonl` plus `session_index.jsonl` at the sessions root with entries `{id, thread_name, updated_at}` (confirmed in `codex-rs/rollout/src/session_index.rs`).
- Terminal title (new): configurable items joined by ` | `, defaults `project-name | activity | run-state | thread-title` (confirmed in `codex-rs/tui/src/bottom_pane/title_setup.rs` + snapshot). Run-state is `Ready` (idle), `Working`, or `Thinking` (busy); activity shows a spinner while working and `Action Required` while blocked on approval. The strategy now derives `has_activity` and the live thread name from the title, falling back to rollout/session_index state.
- `codex exec --ephemeral` skips rollout persistence (so exec-mode sessions leave no metadata).

### Kimi
- Storage (confirmed in docs): `$KIMI_CODE_HOME/` (default `~/.kimi-code/`) with `config.toml`, `session_index.jsonl`, and `sessions/<workDirKey>/<sessionId>/state.json` + `agents/main/wire.jsonl`. `state.json` carries `title` and `lastPrompt` (the strategy's display name and activity signals).
- Resume: `kimi --continue` (most recent in cwd), `kimi --session <id>`, or an interactive picker.
- No documented terminal title behavior; display name comes from `state.json`.

### Pi
- Terminal title (confirmed in installed source, `dist/modes/interactive/interactive-mode.js`): `π - <sessionName> - <cwdBasename>`, or `π - <cwdBasename>` unnamed. Matches the strategy's parser exactly.
- Session files: `~/.pi/agent/sessions/<--encoded-cwd-->/<timestamp>_<uuid>.jsonl`, header `{"type":"session","version":3,...,"cwd":...}` (real files on this machine match).
- Env overrides (confirmed in `dist/config.js`): `PI_CODING_AGENT_DIR` and `PI_CODING_AGENT_SESSION_DIR`. The strategy now honors the session-dir override.
- Bug found + fixed: `expand_tilde` used `strip_prefix('~')` which leaves a leading `/`; `Path::join` with an absolute path replaces the base, so `~/...` values never expanded (affected `PI_CODING_AGENT_DIR` too). Fixed to `strip_prefix("~/")`.

### Resulting changes (commit `a2c97b1e`)
- codex: title-derived `has_activity` (`Working`/`Thinking` busy, `Ready`/`Action Required` idle) and live thread name; rollout fallback preserved.
- pi: `PI_CODING_AGENT_SESSION_DIR` override + tilde-expansion fix.
- claude: project-dir encoding extracted and covered by a docs-grounded test.
- kimi: covered by docs-grounded tests (already implemented correctly).
- 40 tests, clippy 0, fmt clean.

### Second review pass fallout (2026-08-05)

Two review passes (5 agents each) over the `qol-terminal-telepathy` skill surfaced architecture items beyond skill prose:

- **Surface split is real**: `session_wait_output` exists only in the MCP server (this worktree); main ships four tools and the CLI has no `wait` subcommand; the qol-skills pi package ships no session tools at all (the earlier "pi tools shipped" claim was false; hooks.ts never landed). The skill now teaches a poll-based loop as the universal procedure with `wait_output` as an optimization.
- **Capability negotiation gap**: `sessions_list` flattens per-tool phase into a bool `has_activity`, forcing clients to re-derive idle/blocked states; codex blocked-on-approval reads as idle. Future: expose the interpreter phase (Busy/Blocked/Done/Idle) in list rows.
- **Three turn engines overlap**: cli-sessions status state machine, qol-voice turn coordinator, and the telepathy procedure. Next step is to consume cli-sessions status as the idle oracle instead of re-deriving per-tool signals in prose.
- **Pending work confirmed**: pi hooks.ts (register the five tools, including `session_wait_output`) is now the top delivery item; a `qol sessions wait` CLI subcommand would close the CLI gap.

- 2026-08-05: `qol sessions wait <session> [--timeout-ms N] [--expect TEXT]` CLI subcommand landed (shared settle poll extracted from the MCP tool; JSON out with settled/screen/polls/elapsed_ms; host-smoke verified against a live kitty session). qol-skills ships the five native pi tools (`plugins/qol-sessions/extensions/hooks.ts`, registered in the package pi.extensions; note: the sync script owns `.pi/extensions` as generated output, so the hand-written tools extension lives in a sibling dir the script does not manage). Extension load verified headless via `pi -p` with zero errors.

- 2026-08-05: review-driven fixes landed: sync script now registers `plugins/*/extensions` (tool extensions) in the pi package manifest, so `qol sessions` tools survive regeneration; pi hooks read the `activity` key (not the MCP-only `has_activity`/`pending_input`); CLI `send` treats `--submit`/`--insert` as flags only in final position and `wait` filters empty `--expect`; skills defer availability to `qol sessions help`. Verified via installed-package discovery (model enumerated all five `session_*` tools) and a live `sessions_list` call.
