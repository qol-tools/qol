# Architecture

`plugin-cli-sessions` watches the CLI sessions running in a terminal and shows
an always-on-top panel, one row per session, colored by how much each session
wants your attention.

Two things vary independently:

- **where** the sessions live - the shared `qol-terminal-sessions` capability
- **what CLI tool** owns a session - the shared `CliSessionInterpreter`
  (`describe` + `classify_screen` evidence)
- **what** that evidence means to this dashboard - `attention::reduce`

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
          -> CliSessionDescriptor              (semantic metadata + descriptor evidence)
        cli_interpreter.classify_screen(pane, screen)
          -> CliScreenEvidence                 (pure screen evidence: runtime + viewport)
                              |
        Tool::from_cli_session(descriptor)      (dashboard identity mapping)
        service_probe.is_service(pane)         (deterministic: does it hold a listener?)
                              |
        attention::reduce(prev, evidence, now) -> Reduction
          |  Reduction { attention { status, working_since, settled_since },
          |              phase, transition { reason } }
                              v
        Phase { Busy, Service, Blocked, Done, Idle, Hold }   universal vocabulary
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

## Axis 2 - attention policy (`src/attention`)

The dashboard owns one monotonic reducer (`attention::reduce`) and nothing
else. It never parses tool screens: semantic evidence comes from the shared
descriptor (`CliSessionEvidence.runtime`, `.activity`) and screen evidence from
the shared `classify_screen` (`CliScreenEvidence.runtime`, `.viewport`). The
plugin's per-harness strategies are gone; a new CLI tool is added in
`qol-terminal-sessions` and this dashboard picks it up unchanged.

`reduce(prev, evidence, now)` is pure and total over the previous attention
state (status + monotonic timers) and the current evidence, with one explicit
precedence order:

1. **Strong live work wins.** Descriptor `Working` (e.g. the Codex title) is
   live regardless of the viewport. Screen `Working` (spinner in the recent
   tail) is live while the screen is moving or the transcript is fresh; a
   settled, non-fresh spinner is a stale leftover, not live work. Descriptor
   `Ready` (the harness's own runtime state) wins over a settled, fresh
   screen spinner, so a Codex "Ready" title never stays green on weak
   freshness.
2. **Historical viewport holds.** `viewport == Historical` (startup chrome)
   preserves the prior status and can never create attention. It is checked
   before any awaiting/blocked short-circuit, so a stale questionnaire in
   scrollback holds instead of alerting.
3. **Strong NeedsInput alerts immediately.** Descriptor `NeedsInput` (Codex
   "Action Required") is always strong, from any prior status. Screen
   `NeedsInput` is strong unless the transcript is confirmed stale, the
   session is mid-turn with fresh activity (a picker-looking tail while the
   agent works is scrollback), or the evidence is missing and the session was
   just working: with unknown freshness and a moving viewport the plugin
   cannot distinguish a real picker from scrollback residue, so it waits for
   the picker to settle and hold through the grace before alerting. Sustained
   scroll is never distinguished without evidence; it simply never confirms.
4. **Weak freshness is only negative evidence.** `file_fresh` never proves
   Working and never proves turn-taken; it only blocks completion while the
   agent is demonstrably still writing, and never overrides an authoritative
   descriptor `Ready`.
5. **Completion needs settle + grace.** A prior `Working` turn completes to
   `YourTurn` only after a settled screen (stable normalized hash) and a
   monotonic grace window (`GRACE_SECS`, time-based - poll counts never
   debounce). Weak file freshness never overrides authoritative runtime
   state: a descriptor `Ready` (the Codex title) completes on settle plus
   grace even while transcript writes stay fresh. First sightings never
   complete.
6. **Generic shells stay busy-by-default.** A non-prompt generic pane is
   `Working` unless it is a declared service; at the prompt a command that ran
   past the grace window completes, a quick command returns to `Unknown`, and
   an acknowledged session stays acknowledged.

Acknowledgement is sticky in `Status` alone: it survives holds, redraws, stale
markers, and completed turns, and only strong work or strong input breaks it.
Kimi's historical hold is a consequence of rules 2-4: a chrome-less scrolled
screen carries no live evidence, so it holds the prior status instead of
flipping to attention, and a stale questionnaire never short-circuits into
`NeedsYou` before the hold check.

Movement is dashboard policy, so the plugin keeps `screen_hash` plus
`stable_screen` normalization: Pi footer counters and the Kimi status bar are
excluded from the hash, so a session whose screen only shows cosmetic noise
counts as settled. Tool identity (`Tool::from_cli_session`) only selects this
normalization and the display accent; it carries no state machine.

### Adding a tool strategy

Implement and register `CliSessionStrategy` in `qol-terminal-sessions`,
including its `classify_screen` and evidence-bearing `describe`. All consumers
immediately gain a useful label and attention evidence; this dashboard needs no
per-tool changes. Only shared-classifier gaps should ever send you back into
the plugin, and then only to adjust the reducer's policy, never to re-parse a
tool screen.

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

## The reducer seam

`attention::reduce` is the only place terminal evidence becomes a `Status`. It
keeps the transition pure and total, and the two monotonic timers it carries
(`working_since`, `settled_since`) are the only memory beyond the status. The
phase it returns (`Busy`, `Service`, `Blocked`, `Done`, `Idle`, `Hold`) feeds
the anomaly recorder; `Hold` is the fail-safe phase that preserves the prior
status when evidence is absent, historical, or stale.

Transition timing is process-local monotonic time: the reconciler feeds the
reducer monotonic seconds since process start (`mono_now`), never wall time, so
NTP or manual wall jumps can neither expire a grace early nor stall a turn.
The two timers live on the in-memory `SessionState` only and are serde-skipped:
nothing writes them to disk, and a restart restores `Working` with no timers,
so a restored turn with the same screen hash still observes a full fresh grace
before completing. Wall timestamps stay separate and persisted: `last_activity`
is the display clock shown by the panel.

Status transitions emit one redacted, transition-only line through the
existing `CLI_SESSIONS_RECON` probe (trace target `cli-sessions-recon`), in the
same release-safe channel as the per-tick probes: session id, tool, prev/new
status, transition reason, elapsed grace, and the evidence flags that drove the
decision - never screen text, titles, or paths. No transition is ever written
to stderr.

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
