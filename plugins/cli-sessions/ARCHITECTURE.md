# Architecture

`plugin-cli-sessions` watches the CLI sessions running in a terminal and shows
an always-on-top panel, one row per session, colored by how much each session
wants your attention.

Two things vary independently:

- **where** the sessions live - the shared `qol-terminal-sessions` capability
- **what CLI tool** owns a session - the shared `CliSessionInterpreter`
- **what** that tool's on-screen state means to this dashboard -
  `strategy::Strategy`

Everything else (the registry, the reconciler, the panel) is written against the
universal middle vocabulary and never needs to know about Kitty transport or
tool-specific metadata storage.

## Data flow

```
                  every poll tick (reconcile::tick)
                              |
        host.discover() -> Vec<Pane>           (shared terminal capability)
                              |
        cli_interpreter.describe(pane)
          -> CliSessionDescriptor              (shared generic/tool enrichment)
                              |
        Tool::from_cli_session(descriptor)      (dashboard attention policy)
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

## Axis 1 - terminal sessions

`qol-terminal-sessions` owns backend-neutral session identity, discovery, screen
reading, focus, text input, and extensible CLI-session interpretation. Its
session binding combines an opaque backend/session id with the root process id,
so commands can reject a vanished or reused target.

`CliSessionInterpreter` is a registry of `CliSessionStrategy` implementations.
It always has a generic fallback, so every terminal works as a normal CLI
session. Registered strategies enrich recognized tools with a stable tool id,
display name, external session id, and optional activity evidence. Codex,
Claude, Pi, and Kimi are built-ins; future tools are added at this shared seam
rather than reimplemented in every consumer.

The plugin-local `TerminalHost` is a narrow compatibility adapter for the
dashboard:

```rust
trait TerminalHost {
    fn discover(&self) -> Vec<Pane>;
    fn get_text(&self, window_id: u64, root_pid: i32) -> Option<String>;
    fn focus(&self, window_id: u64, root_pid: i32) -> Result<()>;
}
```

`Pane` is the shared `SessionFacts` model. The Kitty parser and command adapter
live in the shared crate and are consumed independently by CLI Sessions and
Voice. The same is true of CLI interpretation: both consumers create their own
shared interpreter and neither brokers the other. This plugin owns only the
numeric-window compatibility and attention policy needed by its existing
registry and presentation model.

### Adding a terminal host

1. Implement the segregated terminal capability traits in
   `libs/qol-terminal-sessions`.
2. Register the backend in `TerminalSessionService`.
3. Keep dashboard-specific selection and attention behavior in this plugin.

## Axis 2 - tool strategy (`src/strategy`)

A `Strategy` turns a `Ctx` (the pane + an optional scrollback snapshot + the
previous reading + now) into a `Reading { phase, label }`. The default impl
`Cli` encodes **standard terminal behavior**, with no tool knowledge:

- at a shell prompt -> `Idle`, or `Done` if a command had been running a while
- not at a prompt -> `Busy`, or `Blocked` if the screen shows a selection / input
  prompt that hasn't changed, or `Service` if it is a long-running service (see
  "Deterministic service detection")

Only the generic `Cli` strategy can reach `Service`. Agents (`Claude`, `Codex`,
`Pi`, `Kimi`) override `read` and never consult `is_service`, so a thinking
agent always keeps its `Working` spinner - its busy -> your-turn lifecycle is
the whole point.

`Claude`, `Codex`, `Pi`, and `Kimi` override `read`/`wants_screen` to detect the
*same* four phases from their own tells instead of the generic prompt
heuristics. Labels and session activity metadata arrive through the shared
descriptor:

| Phase   | `Cli` (generic)              | `Claude`                    | `Codex`                       | `Pi`                            | `Kimi`                  |
|---------|------------------------------|-----------------------------|-------------------------------|---------------------------------|-------------------------|
| Busy    | not at prompt                | `esc to interrupt` / spinner | `esc to interrupt`, title     | braille spinner + `...` loader  | moon-phase spinner line |
| Blocked | selection/input prompt       | choice carets `❯ 1.`        | selection prompt markers      | selector hint (`↑↓ navigate …`) | numbered choice prompt |
| Done    | returned to prompt after run | `✱ ... for <dur>` summary   | session file has >1 turn      | session file has a message      | session has a prompt    |
| Idle    | otherwise                    | otherwise                   | welcome banner / untouched    | startup help / untouched        | untouched / fresh       |

The shared detectors live in `signal::screen` and `signal::title`
(`has_prompt_markers`, `has_input_request`, `claude_working`, `claude_done`,
`codex_banner`, `title_working`, ...). Strategies compose these; they don't
re-implement screen parsing.

`Tool::from_cli_session` maps the shared tool id into the dashboard's known
attention policies. Unrecognized registered tools retain generic dashboard
behavior, so adding shared interpretation never requires this plugin to support
a specialized state machine.

### Adding a tool strategy

1. Implement and register `CliSessionStrategy` in `qol-terminal-sessions` for
   shared detection and semantic metadata. All consumers immediately gain a
   useful label while preserving generic CLI behavior.
2. Only when the dashboard needs specialized attention semantics, add a local
   `Tool` variant and implement `Strategy`.
3. Register that optional attention strategy in `strategy::for_tool`.

## Deterministic service detection (`src/service`)

A dev server, watcher, or `qol dev` is "busy" forever and never hands a turn
back to you, so the `Working` spinner lies. `ServiceProbe` answers one question
without guessing from command-name substrings: **is this pane a long-running
service?** Two deterministic arms, OR'd:

- **OS fact** - a server is a process holding a listening socket. The probe
  snapshots all `LISTEN` pids (`lsof`) and the process tree (`ps`) once per
  reconcile pass, then walks the pane's subtree (root + foreground pids and
  their descendants). A failed probe is scoped to that pass and is retried on
  the next pass, so process transitions cannot be hidden by stale or negative
  cache entries.
  A match means it listens, so it is live. The listener is usually a child
  (`qol` -> `node`), which is why the walk follows the subtree, not just the top
  pid.
- **Explicit declaration** - the `service_commands` config list, matched exactly
  against the reported command or a foreground basename. This covers portless
  long-runners (`cargo watch`, `tail -f`) that hold no socket.

The probe is injected (`reconcile::tick` takes `&dyn ServiceProbe`), so tests use
`NoServiceProbe` / a fake and never shell out. The reconciler only consults it
for generic, non-`at_prompt` panes; everything else short-circuits to `false`.

Kitty discovery produces one shared terminal snapshot per reconcile pass. Screen
reads validate against that snapshot and reuse a screen result when the same
target is requested again during the pass, avoiding a second Kitty discovery
process for every pane.

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

## Notifications and navigation

A transition *into* an attention status (`NeedsYou`/`YourTurn`) is an event the
panel cannot convey when you are not looking at it. `tick` returns the `Notice`s
those transitions produced (`notify::announces_attention` decides; edge-triggered,
so a status that merely persists never re-fires), and the *caller* fires them via
`notify::send` (push-first through the runtime push channel, falling back to a
best-effort `osascript`/`notify-send` shellout when the host is unreachable).
Keeping the I/O in the caller leaves `tick` pure-ish and lets the startup
populate pass drop its batch, so launching the panel is quiet rather than a
notification storm.

Jumping to the next session that wants you must work when the panel is *not*
focused (you are in an editor or another terminal), so it is not an in-view key -
it is a qol-tray-bound action. The manifest declares a `next` catalog action plus
a bindable `[[shortcuts]]` entry; a hotkey fires
`cli-sessions next`, which forwards to the running daemon over its socket
(`Command::NextAttention`). The daemon focuses the next attention session's
terminal window via the host and advances its selection cursor, so repeated
presses cycle through just the rows that want you (`nav::next_attention`, pure and
ordered with attention on top), skipping calm ones - from anywhere, no panel
focus required.

A third action, `snapshot`, is bound the same way with a catalog action plus a
`[[shortcuts]]` entry. It is the user-as-oracle capture: see "Self-healing".

## Self-healing: anomaly capture (`src/anomaly`)

A misread status (a false `NeedsYou`, say) is rare and content-specific, so it is
caught in the act rather than guessed at. The reconciler feeds every observed
frame to an env-gated recorder. The tell it watches for is a *flap*: a session
that enters `NeedsYou` and releases it on its own within a few seconds. A real
prompt persists until you answer it; one that clears itself was almost certainly
a misread. On a flap the recorder dumps a small ring buffer of the surrounding
frames (screen text, title, phase, status) to `paths::anomalies_dir`, so the
review has the transition, not just one frame.

Recording is on automatically in the dev (debug) build - the daemon calls
`anomaly::enable()` at startup under `cfg(debug_assertions)`, so a
launcher-started dev build records with nothing to set. The release build (the
portable one) stays off and writes nothing, host left as found, unless
`CLI_SESSIONS_RECORD_ANOMALIES=1` opts in; `CLI_SESSIONS_ANOMALY_DIR` overrides
the location either way. The gating lives in the *binary* (`ui::run`), not the
library, so the recorder is off by default for the test build too - the
reconcile tests drive `tick` directly and never enable it. The state machine
(`AnomalyRecorder::note`) is pure and unit-tested; the disk dump is a separate
`dump` so the decision is testable without I/O.

`tools/heal-from-anomalies.mjs` closes the loop: it replays each captured frame
through the real classifier (the `classify` example, the same code the daemon
runs) and stages the frames that still read `NeedsYou` as candidate fixtures under
`tests/fixtures/candidates` (git-ignored). The true label stays human-gated - a
real prompt answered within the flap window also flaps - so a confirmed
false-positive is moved into `tests/fixtures/corpus` with an expected status,
turning the once-live misread into a permanent regression test. The corpus and
the `examples/classify` harness let you pin behavior against real captured CC
screens instead of hand-written strings.

The flap recorder only sees one direction: a *false positive* (a NeedsYou that
self-clears). A *false negative* - a session that genuinely wants you but reads
idle/working - has no temporal tell, so the daemon, whose own classification is
the thing that is wrong, cannot detect it. Only you can. The `snapshot` action
(`cli-sessions snapshot`, bindable to a hotkey) is that escape hatch: one press
dumps every live session's frame in the moment - screen, title, and the status
the panel is currently showing - to `paths::snapshots_dir` in the same
corpus-fixture shape (`snapshot::capture_all`). When the panel is wrong in any
direction, you snap, then promote the offending frame into the corpus with the
right `expect`. It always records (user intent, not autonomous), independent of
the dev-build recorder gating.

## Known gaps

`service_commands` (the explicit-declaration arm of service detection) is read
from the plugin config and exposed in `qol-config.toml` as a `string_array`
field - the contract's editable string-list kind, rendered as add/remove rows of
one command each in the settings UI. The terminal host is always kitty and the
reconcile interval is a fixed constant; no `host` or `poll_secs` config fields
are declared for either.
