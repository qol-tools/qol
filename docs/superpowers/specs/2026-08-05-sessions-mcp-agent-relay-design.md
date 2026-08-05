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

## Iteration log

- 2026-08-05: research pass completed (above). PoC `qol sessions` in progress.
- 2026-08-05: PoC landed (list/send/read/focus, JSON list, unit tests, clippy clean); guest-VM verification passed (transcript + probes above).
