# plugin-claude-sessions: always-on-top Claude session overview

- Date: 2026-06-17
- Status: draft v4 (revised after third review)
- Author: brainstormed with Claude
- Topic: a qol-tray plugin showing every live Claude Code session with glanceable status and jump-to-session

## Summary

A new qol-tray plugin, `plugin-claude-sessions`, renders a minimal always-on-top
floating panel. One row per Claude Code session currently alive on the machine.
Each row is colored by status (green = working, yellow = your turn, red = needs
permission/input). Pressing Enter (or clicking) a row jumps to the terminal
hosting that session via a pluggable terminal-host strategy. Kitty is the first
host implementation; the strategy is shaped so iTerm2, WezTerm, tmux, or an IDE
can be added later without touching the rest of the plugin.

The plugin reuses two pieces from the monorepo's git history (both removed in
commit `86305fa5`, "chore(plugins): remove archived kitty and claude-sessions"):
the libproc session resolver from the old `plugin-claude-sessions`, and the kitty
IPC adapter from `plugin-kitty`.

## Review revisions

### First review

All verified against the codebase and current Claude Code hook docs:

1. **Daemon launch contract.** qol-tray spawns daemons with no argv
   (`apps/qol-tray/src/plugins/daemon_lifecycle/spawn.rs:64`,
   `Command::new(daemon_path)`) and validates `daemon.command` as a single
   basename (`libs/qol-plugin-api/src/manifest/validation/command_rules.rs:22`).
   No-arg execution means daemon mode; the shim is a subcommand of the same binary
   (see second review for why it is not a second binary).
2. **Hook binary path.** Hooks must not rely on PATH resolution, so the installed
   absolute path is used, derived at runtime from `QOL_TRAY_PLUGIN_DIR` /
   `std::env::current_exe()`.
3. **Typed permission signal.** Claude Code exposes a dedicated `PermissionRequest`
   event and a typed `Notification.notification_type` (`permission_prompt`,
   `idle_prompt`, ...). Status mapping is type-aware, not "every Notification is red".
4. **Sockets are streams.** qol-tray uses Unix STREAM sockets throughout
   (`libs/qol-plugin-daemon/src/daemon.rs` `UnixListener`,
   `apps/qol-tray/src/plugins/action_transport/platform/unix_common.rs`
   `UnixStream`). Two stream sockets: the manifest action socket and a separate
   hook-ingest socket.
5. **Hotkey is a host binding.** Hotkeys are host-managed
   `HotkeyBinding{plugin_id, action}` mapped to a manifest menu action
   (`apps/qol-tray/src/hotkeys/types.rs:12`). The panel declares an `open` action;
   the user binds a hotkey to it. No manifest field exists for a plugin-declared
   default, so v1 documents manual assignment.

### Second review

1. **Single binary, not two.** Release automation only handles the first declared
   binary: `plugin_matrix.py:34` builds `binaries[0]` and `release.yml` stages one
   artifact, while the store requires an asset for every declared binary
   (`apps/qol-tray/src/features/plugin_store/github/releases.rs:75`). A second
   binary would fail store release discovery. The hook is therefore a `hook`
   subcommand of the one binary (matching `86305fa5^:plugins/plugin-kitty/src/main.rs`).
   A separate lean hook binary remains a future optimization, gated on teaching the
   release pipeline to build and stage all `dependencies.binaries`.
2. **Cleanup guarantee weakened to silent-orphan + manual.** There is no host
   pre-uninstall or disable lifecycle: `uninstall` is a bare `remove_dir_all`
   (`apps/qol-tray/src/features/plugin_store/installer/operations.rs:93`) and
   `stop_daemon` is an undifferentiated terminate that also fires on
   recompile/shutdown (`apps/qol-tray/src/plugins/daemon_lifecycle/mod.rs:18`). The
   daemon cannot tell uninstall from a restart, so it must not remove hooks on stop.
   Instead the hook is installed as a self-guarding shell command that exits 0 when
   the binary is gone (inert orphan), with a manual `cleanup` action for true
   removal. A host pre-uninstall lifecycle is proposed as the real fix (future).
3. **Namespace literal fixed.** The config/data namespace is `qol-tray`
   (`qol_config::NAMESPACE`), not `qol-tools`. Per the auto-loaded qol-config
   path-convention rule, the path is resolved through qol-config /
   `QOL_TRAY_PLUGIN_DIR` / `current_exe`, never by re-deriving `dirs::config_dir()`
   and joining a literal.

### Third review

No blockers; two refinements and a wording fix:

1. **Shell escaping.** The guarded hook command POSIX single-quote escapes every
   interpolated path (not just spaces), neutralizing quotes, `$`, backticks, `;`,
   and newlines. The test plan asserts this.
2. **Schema-compatible marker.** The managed block is identified by command
   signature using only Claude-Code-schema fields; no JSON comments or unknown keys.
3. **Wording.** "Not PATH-resolved" softened to "must not rely on PATH resolution"
   (exec-form commands can be PATH-resolved; this design uses an absolute path
   regardless).

## Context and motivation

The user runs several Claude Code sessions at once across kitty terminals and
loses track of which ones need attention. Today there is no single place to see
"which session is waiting on me right now."

Prior art found during brainstorming:

- `plugins/plugin-claude-sessions` (deleted at `86305fa5`): mapped a `claude` PID
  to its active session `.jsonl` via libproc fd-walking. Built for restore claims,
  not a UI. Recoverable with `git show 86305fa5^:plugins/plugin-claude-sessions/...`.
- `plugins/plugin-kitty` (deleted at `86305fa5`): kitty IPC, named-workspace
  lifecycle, snapshot/restore. Its `src/kitty.rs` IPC adapter is the reference for
  the kitty host strategy.
- A friend's tool, https://github.com/Nicsilver/claude-sessions, solves a similar
  problem but is IntelliJ-focused and not terminal/kitty-aware.

Neither archived plugin exists as a standalone repo in the `qol-tools` GitHub org;
they only ever lived in this monorepo.

## Goals

- A persistent, always-on-top, minimal panel listing all live Claude sessions.
- At-a-glance status color: green (working), yellow (your turn), red (needs you).
- Keyboard-first: a hotkey shows/focuses the panel; arrow keys select; Enter jumps
  to the session; Esc blurs.
- Jump-to-session through a pluggable terminal-host strategy (kitty first).
- Self-healing hook registration that is inert when orphaned and removable on demand.

## Non-goals

- Session lifecycle management (spawning or killing sessions) - deferred.
- Reading or rendering transcript content beyond a one-line status summary.
- Windows support in v1 (libproc and kitty paths are macOS/Linux).
- Restore-claim integration with qol-tray workspace restore (the archived plugin's
  original purpose) - out of scope.

## Key decisions

| Decision | Choice | Rationale |
|---|---|---|
| Status signal | Claude Code hooks, typed | Hooks are the only reliable signal for "needs permission"; `PermissionRequest` + `Notification.notification_type` give a clean typed red/yellow. |
| Hook coverage gap | Idempotent self-heal at user level | The plugin ensures a managed block in `~/.claude/settings.json`; Claude's file watcher applies it to running sessions (restart only if missed). |
| Host reversibility | Silent-guarded orphan + manual cleanup | The hook command guards on the binary's existence and exits 0 when gone; a `cleanup` action removes the block. Auto-removal needs a host lifecycle that does not exist yet. |
| Packaging | Single plugin, one binary | No-arg = daemon + UI + actions; `hook` subcommand = shim; `open`/`cleanup` = actions. One binary keeps store release discovery working. |
| Strategy scope | `focus()` + `discover()` | Jump-to-session plus host enumeration to enrich rows and catch sessions hooks have not reported yet. |
| Join key | claude PID | The shim (via PPID walk) and host discovery both resolve to the claude PID; the libproc resolver bridges PID to session. |
| Presentation | Persistent floating panel | Matches "minimal window, always on top"; a glanceable monitor, not a summoned picker. |
| Row layout | Two-line, tinted | Dot + project + branch on line 1; summary + elapsed on line 2. |
| IPC | Two Unix STREAM sockets | Manifest action socket (menu actions) + a separate hook-ingest socket. Matches qol-tray's stream-socket convention. |
| Toggle | Menu action `open` + host hotkey | Hotkeys are host-managed bindings to menu actions; the user assigns the key. |
| Name | `plugin-claude-sessions` | Revives the freed name; the new plugin supersedes the old one's intent. |

## Architecture overview

One Rust plugin crate, `plugins/plugin-claude-sessions`, producing one binary,
`plugin-claude-sessions`, that dispatches on argv (the kitty precedent):

- no argv - daemon mode (launched by qol-tray). Owns the registry, reconciler, the
  GPUI panel, the action socket, and the hook-ingest socket.
- `hook` - the shim Claude Code invokes per event. Does minimal work and never
  initializes GPUI, so per-event cost stays low.
- `open` / `cleanup` - menu actions, dispatched to the running daemon over the
  action socket (or run directly when the daemon is down).

```
claude session ─hook event─> `plugin-claude-sessions hook` ─stream(JSON line)─> daemon.ingest ─┐
                                                                                                │
qol-tray ─menu action "open"/"cleanup"─> daemon.action socket ──────────────────────────────────┤
kitty host ─discover()/focus()─> daemon.reconciler <─ periodic tick ─────────────────────────────┤
                                                                  registry: SessionId -> SessionState
                                                                                                │ render
                                                                  always-on-top GPUI panel (rows)
                                                                                                │ Enter / click
                                                                          host.focus(pane)
```

## Components

### Binary `plugin-claude-sessions`, argv dispatch

- no argv: daemon mode (below).
- `hook`: read hook JSON from stdin (`session_id`, `transcript_path`, `cwd`,
  `hook_event_name`, and event-specific `tool_name` / `notification_type` /
  `message`); resolve the claude PID via `getppid()` then libproc `parent_pid`
  walk until the exe basename is `claude`; map to status + summary; connect the
  hook-ingest stream socket (absolute path passed via `--socket`), write one JSON
  line, close; exit 0 unconditionally. No GPUI in this path.
- `open`: tell the daemon to show/focus the panel.
- `cleanup`: remove the managed hook block from `~/.claude/settings.json` (works
  daemon-up over the socket, or daemon-down as a direct edit).

### Daemon mode (no argv)

Long-running. Concerns run on separate threads, with background threads posting
updates into the GPUI app over a channel (the plugin-launcher controller pattern):

- ingest: binds the hook-ingest stream socket (a second `UnixListener`);
  deserializes shim events; updates the registry.
- actions: the manifest action socket via the qol-plugin-daemon `start_listener`
  helper; handles `open`, `cleanup`, and lifecycle pings.
- reconciler: periodic tick that
  - prunes sessions whose PID is no longer alive (libproc / `kill(0)`),
  - runs `host.discover()` to pick up cold sessions and attach pane location,
  - refreshes the git branch per cwd (cached),
  - re-asserts the managed hook block only if missing or drifted (never rewrites a
    correct block, to avoid thrashing Claude's settings file watcher).
- UI: the always-on-top GPUI panel, rendering rows from the registry.

### `src/host/` - terminal host strategy

```rust
pub struct Pane { pub pid: i32, /* host-specific locator */ }

pub trait TerminalHost {
    fn discover(&self) -> Vec<Pane>;
    fn focus(&self, pane: &Pane) -> anyhow::Result<()>;
}
```

`kitty` implementation:

- `discover()` parses `kitten @ ls` to map each window/pane to its foreground PID.
- `focus()` runs `kitten @ focus-window --match pid:<N>` (raising the OS window as
  needed).

Host selection comes from config; the trait keeps the rest of the plugin host-agnostic.
The kitty IPC details are lifted from `86305fa5^:plugins/plugin-kitty/src/kitty.rs`.

### `src/resolver/` - libproc session resolver

Lifted from `86305fa5^:plugins/plugin-claude-sessions/src/resolver/`. Given a PID,
confirms the process is `claude` and returns its active session `.jsonl` under
`~/.claude/projects/<encoded-cwd>/<uuid>.jsonl`. Used for:

- host-discovered cold sessions (a kitty pane running claude that has not fired a
  hook since the daemon started),
- PID liveness during pruning.

Hook-reported sessions already carry `transcript_path`, so the resolver is not on
their path.

## Session model and status mapping

```rust
struct SessionState {
    session_id: String,
    pid: i32,
    project: String,       // cwd basename
    cwd: PathBuf,
    branch: Option<String>,
    status: Status,        // Working | YourTurn | NeedsYou
    summary: String,       // one line
    last_activity: Instant,
    pane: Option<Pane>,    // host locator, when known
}
```

Registered hook events and their mapping (matchers keep the set tight):

| event (and type) | status | summary |
|---|---|---|
| UserPromptSubmit | Working (green) | "working" |
| PreToolUse | Working (green) | `tool_name` (e.g. "Bash") |
| SessionStart | Working (green) | "started" |
| Notification, `notification_type = permission_prompt` | NeedsYou (red) | "permission" |
| PermissionRequest | NeedsYou (red) | matcher (e.g. "ExitPlanMode") |
| Notification, `notification_type = idle_prompt` | YourTurn (yellow) | "your turn" |
| Stop / SubagentStop | YourTurn (yellow) | "your turn" |
| Notification, other types | (no status change) | - |
| SessionEnd, or PID dead at tick | (row removed) | - |

- elapsed = now - last_activity, rendered compact ("12s", "2m", "now").
- branch via `git rev-parse --abbrev-ref HEAD` in cwd, cached per cwd, refreshed on tick.
- Sort order: red, then yellow, then green; within a color, most-recent activity first.

## Hook registration, self-healing, and cleanup

The daemon owns an idempotent, sentinel-marked block in `~/.claude/settings.json`
under `hooks`, covering the events above, each running this binary by absolute path.

- Absolute path: resolved from `std::env::current_exe()` / `QOL_TRAY_PLUGIN_DIR`
  (config namespace is `qol-tray`), to avoid relying on PATH resolution.
- Invocation form: a self-guarding shell command so an orphaned block (post
  uninstall or crash) is silent rather than logging a per-event hook error:

  ```sh
  test -x '<abs-bin>' && '<abs-bin>' hook --socket '<abs-socket>' || true
  ```

  Shell form is chosen over exec form specifically for this guard. Every
  interpolated path is POSIX single-quote escaped (wrap in single quotes, and
  replace each embedded `'` with `'\''`), which neutralizes spaces, `$`, backticks,
  `;`, double quotes, and newlines, not only spaces. (Exec-form `args` are supported
  by current Claude Code but cannot express the existence guard.)
- Identification: managed entries are recognized by their command signature - the
  binary's absolute path plus a stable sentinel token carried in the standard
  `command`/`args` (e.g. a `--marker qol-claude-sessions` flag the shim ignores).
  Detection, replacement, and removal use only Claude-Code-schema fields
  (`matcher`, `hooks[].type`, `command`, `args`); no JSON comments or unknown keys,
  which Claude Code may strip or reject. Merge by reading the whole `hooks` object,
  editing only entries that match the signature, and writing back.
- Self-heal: re-asserted on daemon start and on tick only when missing/drifted.
  Claude's file watcher applies settings changes to running sessions automatically
  (restart only if it misses the change).
- Removal: there is no host pre-uninstall/disable lifecycle, and the daemon's stop
  signal is indistinguishable from recompile/shutdown, so the daemon never removes
  hooks on stop. Removal happens via the manual `cleanup` action. An orphaned block
  is inert in the meantime thanks to the guard. Honoring the qol-tray mission ("host
  left exactly as found") fully would require a host pre-uninstall lifecycle action;
  that is proposed as a follow-up (see open questions).
- Known limit: a session started before first install reports only after Claude
  reloads settings; host `discover()` surfaces such sessions meanwhile (without rich
  status until they next fire a hook).

## Window and UX

- qol-gpui `popup_window`, `NSPopUpMenuWindowLevel` (always on top), borderless,
  small (about 330px wide), parked in a configurable screen corner.
- Toggle: the manifest declares an `open` menu action; the user binds a host hotkey
  to it in qol-tray's hotkey settings. The host dispatches `open` to the daemon's
  action socket, which shows/focuses the panel. (No qol-runtime keybind, no hotkey
  in the plugin's own config; auto-seeding a default is not supported by the host
  manifest today.)
- In-panel keys (handled by the GPUI window once focused): arrow keys move
  selection, Enter focuses the selected session, Esc blurs, optional `1..9`
  quick-jump.
- Mouse click on a row also focuses that session.
- Two-line tinted rows; the panel title bar shows the live session count.

## Configuration (`qol-config.toml`)

- panel corner and pixel offset,
- host target (default `kitty`),
- reconciler poll interval,
- max rows shown.

(The toggle hotkey is not here; it lives in qol-tray's host hotkey config bound to
the `open` action.)

## Code reuse from history

| Lift | Source (`86305fa5^`) |
|---|---|
| libproc session resolver + structural tests | `plugins/plugin-claude-sessions/src/resolver/*`, `tests/*` |
| encoded-cwd helpers | `plugins/plugin-claude-sessions/src/encoding.rs` |
| `parent_pid` PID-walk | already present in `plugins/plugin-alt-tab` discovery / `libs/qol-app-icon` |
| kitty IPC adapter (ls, focus) | `plugins/plugin-kitty/src/kitty.rs` |
| daemon argv dispatch (no-arg = daemon) | `plugins/plugin-kitty/src/main.rs`, `plugin.toml` |
| plugin scaffold (plugin.toml, daemon wiring, `ui/` structure) | `plugin-launcher` + `plugin-kitty` |

The `hook` subcommand and the settings.json manager are new.

## Cross-platform and build

- `plugin.toml` `platforms = ["macos", "linux"]` (matches the archived plugin).
- libproc and kitty paths are cfg-gated so the crate compiles clean under
  `RUSTFLAGS=-D warnings` on macOS, Linux, and Windows (Windows builds an inert
  stub, as other plugins do). The auto-loaded qol-tray rule treats warnings as
  errors on every backend.
- Config/data paths go through `qol-config` (namespace `qol-tray`); the plugin never
  re-derives `dirs::config_dir()` and joins a literal (qol-config path-convention rule).
- AF_UNIX path limit: macOS caps `sun_path` near 104 bytes, and the plugin install
  dir (`.../qol-tray/plugins/plugin-claude-sessions/`) is long. Place both sockets
  in a short runtime directory (the qol-tray runtime socket location), not the
  plugin dir.

## Error handling and edge cases

- shim: best-effort; socket absent means drop and exit 0. Never blocks claude.
- orphaned hooks (post-uninstall or crash): the existence guard exits 0, so no
  per-event error noise; `cleanup` removes the block when the user chooses.
- dead or zombie PID: pruned on the next reconciler tick.
- non-git cwd: no branch field rendered.
- kitty or `kitten` missing: `discover()` returns empty, `focus()` returns Err; the
  row still renders, the jump no-ops with a logged warning (and optional toast).
- two sessions in worktrees of the same repo (same cwd basename): disambiguated by
  `session_id` and the branch field.
- duplicate or rapid hook events: registry updates are last-writer-wins keyed by
  `session_id`.

## Testing strategy

Table-driven where possible:

- hook event JSON (incl. `notification_type` variants) to status/summary mapping,
- `~/.claude/settings.json` merge idempotency (absent -> inserted; present -> no-op;
  drifted -> repaired; `cleanup` -> removed; unrelated hooks preserved),
- guarded hook-command construction with POSIX single-quote escaping (absolute path
  + socket arg + existence guard; paths containing spaces, single/double quotes,
  `$`, backticks, `;`, and newlines),
- managed-block identification by command signature (matches our entries, ignores
  the user's other hooks, uses only schema fields),
- resolver path matcher (reuse archived structural tests),
- registry prune logic (alive vs dead PID),
- row sort order,
- `kitten @ ls` output parse to pane/PID,
- `plugin.toml` manifest structural test (one binary; `open` + `cleanup` actions; daemon).

## Open questions

- Exact `Notification` payload shape in the installed Claude Code version
  (`notification_type` field name and values vs a legacy free-text `message`).
  Capture against a live session before wiring red vs yellow; design tolerates both
  by checking `notification_type` first, `message` second.
- `PermissionRequest` does not fire in non-interactive `-p` runs; acceptable here
  since the panel targets interactive sessions, with `Notification`/`permission_prompt`
  as the primary red signal.
- A host pre-uninstall/disable lifecycle action in qol-tray would let the daemon
  remove its hook block automatically (true mission compliance). Worth proposing as
  a separate host change; until then, silent-orphan + `cleanup` is the contract.
- Whether to add a negative-result cache for cold-session resolution to avoid
  re-walking PIDs every tick.
- Should the panel auto-show (steal focus) when a session flips to red, or only
  recolor silently.

## Out of scope / future

- Additional host strategies (iTerm2, WezTerm, tmux, IntelliJ).
- A separate lean hook binary, gated on the release pipeline building and staging
  all `dependencies.binaries` (today only `binaries[0]` is built/staged).
- A host pre-uninstall lifecycle for automatic hook removal.
- Session spawn/kill from the panel.
- Surfacing recent transcript context or token/cost per session.
- Windows support.
- Plugin-seeded default hotkey (needs a host manifest capability that does not exist yet).
