# Architecture

`plugin-cli-sessions` watches the CLI sessions running in a terminal and shows
an always-on-top panel, one row per session, colored by how much each session
wants your attention.

Two things vary independently, and each is a strategy behind a trait:

- **where** the sessions live - the terminal (`host::TerminalHost`)
- **what** a session's on-screen state *means* - the tool running in it
  (`strategy::Strategy`)

Everything else (the registry, the reconciler, the panel) is written against the
universal middle vocabulary and never needs to know about kitty, Claude, or
Codex.

## Data flow

```
                  every poll tick (reconcile::tick)
                              |
        host.discover()  -> Vec<Pane>          (TerminalHost: the "where")
                              |
        tool::classify(pane) -> Tool           (which CLI is in the foreground)
                              |
        service_probe.is_service(pane)         (deterministic: does it hold a listener?)
                              |  -> Ctx.is_service (generic panes only)
        for_tool(tool).read(Ctx) -> Reading    (Strategy: the "what")
                              |  Reading { phase, label }
                              v
        Phase { Busy, Service, Blocked, Done, Idle }   universal terminal vocabulary
                              |
        status_for(prev, phase) -> Status      folds in memory (ack stickiness)
                              |
        Status { Working, Service, YourTurn, NeedsYou, Unknown, Acknowledged }
                              |
        registry (sorted by attention) -> SessionsView (panel)
```

## Axis 1 - terminal host (`src/host`)

`TerminalHost` is the only thing that talks to a concrete terminal:

```rust
trait TerminalHost {
    fn discover(&self) -> Vec<Pane>;          // enumerate live panes
    fn get_text(&self, window_id: u64) -> Option<String>; // scrollback snapshot
    fn focus(&self, window_id: u64) -> Result<()>;        // jump-to-session
}
```

`Pane` is the host-neutral description the rest of the code consumes (window id,
root pid, cwd, title, `at_prompt`, foreground process names/pids). `kitty` is the
only implementation today; nothing outside `src/host/kitty` references kitty. The
`host` config field is meant to select the implementation (only `kitty` is wired
- see Known gaps).

### Adding a terminal host

1. Implement `TerminalHost` in a new `src/host/<name>` module, producing `Pane`s.
2. Select it in `ui::run` where `Kitty` is constructed.

## Axis 2 - tool strategy (`src/strategy`)

A `Strategy` turns a `Ctx` (the pane + an optional scrollback snapshot + the
previous reading + now) into a `Reading { phase, label }`. The default impl
`Cli` encodes **standard terminal behavior**, with no tool knowledge:

- at a shell prompt -> `Idle`, or `Done` if a command had been running a while
- not at a prompt -> `Busy`, or `Blocked` if the screen shows a selection / input
  prompt that hasn't changed, or `Service` if it is a long-running service (see
  "Deterministic service detection")

Only the generic `Cli` strategy can reach `Service`. Agents (`Claude`, `Codex`)
override `read` and never consult `is_service`, so a thinking agent always keeps
its `Working` spinner - its busy -> your-turn lifecycle is the whole point.

`Claude` and `Codex` override `read`/`label`/`wants_screen` to detect the *same*
four phases from their own tells instead of the generic prompt heuristics:

| Phase   | `Cli` (generic)              | `Claude`                    | `Codex`                       |
|---------|------------------------------|-----------------------------|-------------------------------|
| Busy    | not at prompt                | `esc to interrupt` / spinner | `esc to interrupt`, title     |
| Blocked | selection/input prompt       | choice carets `❯ 1.`        | selection prompt markers      |
| Done    | returned to prompt after run | `✱ ... for <dur>` summary   | session file has >1 turn      |
| Idle    | otherwise                    | otherwise                   | welcome banner / untouched    |

The shared detectors live in `signal::screen` and `signal::title`
(`has_prompt_markers`, `has_input_request`, `claude_working`, `claude_done`,
`codex_banner`, `title_working`, ...). Strategies compose these; they don't
re-implement screen parsing.

`tool::classify` picks the `Tool` from the foreground process names, and
`for_tool` maps it to the strategy.

### Adding a tool strategy

1. Add a `Tool` variant and a rule in `tool::classify`.
2. Implement `Strategy` (override `read` for the phase detection, `label` for the
   row title, `wants_screen` if you need the scrollback).
3. Register it in `strategy::for_tool`.

## Deterministic service detection (`src/service`)

A dev server, watcher, or `qol dev` is "busy" forever and never hands a turn
back to you, so the `Working` spinner lies. `ServiceProbe` answers one question
without guessing from command-name substrings: **is this pane a long-running
service?** Two deterministic arms, OR'd:

- **OS fact** - a server is a process holding a listening socket. The probe
  snapshots all `LISTEN` pids (`lsof`) and the process tree (`ps`) once per tick,
  then walks the pane's subtree (root + foreground pids and their descendants).
  A match means it listens, so it is live. The listener is usually a child
  (`qol` -> `node`), which is why the walk follows the subtree, not just the top
  pid.
- **Explicit declaration** - the `service_commands` config list, matched exactly
  against the reported command or a foreground basename. This covers portless
  long-runners (`cargo watch`, `tail -f`) that hold no socket.

The probe is injected (`reconcile::tick` takes `&dyn ServiceProbe`), so tests use
`NoServiceProbe` / a fake and never shell out. The reconciler only consults it
for generic, non-`at_prompt` panes; everything else short-circuits to `false`.

## The Phase -> Status seam

`Phase` is what the terminal is *doing*; `Status` is what it *means to you*.
`status_for(prev, phase)` is the only place they connect, and it folds in one bit
of memory - an `Acknowledged` session stays acknowledged until it goes Busy
again, so a finished agent you've already seen doesn't keep nagging.

| prev          | phase   | -> Status      |
|---------------|---------|----------------|
| any           | Busy    | Working        |
| any           | Service | Service        |
| any           | Blocked | NeedsYou       |
| Acknowledged  | Done    | Acknowledged   |
| other         | Done    | YourTurn       |
| Acknowledged  | Idle    | Acknowledged   |
| other         | Idle    | Unknown        |

`running_since` (how long the current Busy stretch has run) is also derived from
the phase at this seam, in the reconciler (`running_since_for`), so it is tracked
uniformly for every tool rather than per-strategy.

`Status` drives both the row color and the sort order (most attention-worthy
first): NeedsYou -> YourTurn -> Working -> Service -> Unknown -> Acknowledged.
`Service` renders blue with a slow pulse (calm, not the spinner) and its counter
reads as uptime; it sinks below live agent work so background servers do not hog
the top rows.

## Known gaps

The `host` and `poll_secs` config fields are declared in `qol-config.toml` but
not yet consumed - the host is always kitty and the poll interval is a constant.
The `corner` field is honored (the panel parks in the configured screen corner).
`service_commands` (the explicit-declaration arm of service detection) is read
from the plugin config, but it has no `qol-config.toml` editor field yet - it is
a config-file-only knob, since the UI has no editable string-list type.
