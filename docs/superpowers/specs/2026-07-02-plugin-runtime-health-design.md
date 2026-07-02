# Plugin Runtime Health - Design

Date: 2026-07-02
Status: Draft (awaiting review)

## Problem

`qol dev`'s Plugins page (`tools/qol-cli/src/dev_console.rs`, `draw_plugins`)
only encodes link/build state - each row is linkable / stale / linked, sourced
from `GET /api/dev/links` (`dev::LinkedPlugin`). There is no daemon-liveness
field anywhere in that payload.

Separately, qol-tray already runs a background `daemon_supervisor`
(`apps/qol-tray/src/plugins/daemon_supervisor.rs`, 5s tick) that detects dead
daemons and auto-restarts them, with a `MAX_CONSECUTIVE_FAILURES`-based
backoff that gives up and only logs an error. That state is local to the
supervisor task and invisible outside the process.

Concrete trigger: after an ARMED+Ctrl+R restart, several plugins died to
panics. The user manually tested a few plugins but had no way to confirm the
rest were actually running - `qol dev` only shows that they are *linked*.
Doctor's `plugin_process_leaks` check catches the opposite case (untracked
processes running that shouldn't be), not this one.

A deeper audit (see Prerequisites) found the supervisor's backoff itself does
not catch the reported failure mode: a daemon that spawns successfully and
then panics shortly after loops forever without ever being suppressed,
because `Command::spawn()` succeeding resets the failure counter today. This
must be fixed for any health signal built on top of it to be truthful.

## Goals

- Per-plugin, live "is the daemon actually running" signal, glanceable on
  `qol dev`'s existing Plugins page.
- A coarse "something's wrong" signal that appears in `qol dev`'s existing
  Doctor panel for free, no new client polling loop for the common case.
- A truthful signal during and immediately after the ARMED+Ctrl+R generation
  handoff - never confidently wrong.
- Correctly distinguish "transient, will self-heal" from "stuck in a crash
  loop," which today's supervisor cannot do.

## Non-goals

- Socket/IPC readiness (`existing_daemon_socket_ready`) as a health field.
  "Stable" below means sustained process liveness only, not that the daemon
  is accepting connections yet. A slow-initializing (but not crash-looping)
  daemon can read "stable" before it's actually ready. Acceptable for v1
  since the motivating problem is crash-looping, not slow starts.
- `last_error` (the restart failure message) in the per-plugin payload. Would
  require new `SupervisorState` storage that doesn't exist today. Add only if
  a real need shows up.
- Health for plugins outside what the Plugins page already lists
  (`/api/dev/links` + `/api/dev/discovery-state`). Release-installed plugins
  the supervisor tracks internally but that page doesn't show are unaffected.
- Making doctor network-dependent. It stays fully offline-capable - see
  Architecture.

## Prerequisites (must land before the health signal is trustworthy)

### P1. Handoff error propagation (`tools/qol-cli/src/dev_console/reload.rs`)

Today `restart_child_from_prebuilt` (~line 147-184): if `promote_shadow_generation`
returns `Err` (line 174), it logs "successor promotion incomplete", then
*unconditionally* assigns `*child = next`, `*lines = next_lines`, pokes
doctor/links to refresh, logs "successor generation active", and returns
`Ok(())`. No caller can distinguish a failed promotion from a successful one.

Fix: on `Err`, terminate/reap `next` (mirroring the existing
`retire_child_for_handoff` failure path just above it) and return `Err` to
the caller before touching `*child`/`*lines`.

Blocks the feature because the handoff freeze/thaw rule below needs a real
`Result` to gate on.

### P2. Supervisor recovery semantics (`apps/qol-tray/src/plugins/daemon_supervisor.rs`)

Two existing reset paths both reset `consecutive_failures` on a single
momentary "alive" observation:

- `for id in &outcome.alive { state.observe_alive(id); }` (line 64)
- `restart_daemon(...).Ok(()) => state.observe_alive(&plugin_id)` (line 72)

`Command::spawn()` succeeding only proves the OS created the process, not
that it's stable. A daemon that panics ~200ms after every respawn attempt
loops forever at the 5s tick cadence: each restart's spawn succeeds, the
counter resets, `MAX_CONSECUTIVE_FAILURES` never trips. This is exactly
"plugins died due to a panic" - and today it is indistinguishable from
healthy operation except in the log stream.

Fix: pid-aware probation. Track per plugin, in `SupervisorState`:
`last_pid: Option<u32>`, `stable_since_tick: Option<u32>`,
`consecutive_failures: u32`, `last_seen: LastSeen`.

- Observed pid appears or differs from `last_pid` -> enter probation, reset
  the stability counter.
- Alive but probation hasn't reached `STABLE_TICKS` (3 ticks, ~15s) -> do not
  reset `consecutive_failures`.
- Dies during probation -> increment `consecutive_failures` (a new failure
  source, distinct from a spawn-level `Err`).
- Survives past `STABLE_TICKS` -> mark stable, reset `consecutive_failures`
  to 0.
- Dies after stable -> fresh failure cycle, retry as today.
- Suppress restart attempts only after `MAX_CONSECUTIVE_FAILURES` failed
  cycles under this corrected definition.

This also correctly handles restarts that happen outside the supervisor
(e.g. the log-control mute/unmute path's `try_restart_daemon` in
`dev_link_handlers.rs`, which changes the pid out of band) - probation
triggers on any pid change, not only supervisor-initiated ones.

## Architecture: who owns what

Three consumers, one source of truth. None re-derives liveness independently.

1. **qol-tray (source of truth).** The corrected supervisor (P2) is the only
   thing that knows true liveness/stability/failure history. It already
   ticks every 5s; no new server-side polling loop.
2. **Offline doctor check (coarse, free dashboard surfacing).** New dev-only,
   `dev-loop`-grouped check `plugin_daemon_pids`, mirroring
   `plugin_staleness.rs`'s `stale_running_daemons`: reads `tracked_pids()`
   from the runtime pids dir, warns on any tracked pid that isn't alive. Zero
   tracked pids is not a failure (tray may simply not be running). Cheap,
   offline, and lands automatically in `qol dev`'s existing adaptive Doctor
   panel (`dev_console/doctor.rs`, 10-60s poll) - no new endpoint or merge
   logic needed for the coarse signal.
3. **`GET /api/dev/plugin-health` (row-level detail).** New endpoint on
   qol-tray's existing dev HTTP surface, for what doctor's message-string
   format can't give: per-plugin-row detail on the Plugins page. Reads the
   corrected supervisor state. Follows the `list_linked_plugins` convention
   (`tokio::task::spawn_blocking` around manager-lock access, not a raw sync
   `lock()` in the async fn body) and the existing lock-poisoned defensive
   pattern (`get_discovery_state`): log and return a degraded response,
   never a 500 - this is polled every 5s and transient lock contention
   should not flash an error state.

Doctor deliberately never calls the endpoint. It is the tool for diagnosing
"why won't qol-tray start" - making any check depend on qol-tray's HTTP
server being up would break that.

## Data model

Response envelope - once per response, not per row, since generation
identity is a single per-process fact:

```
process_pid: u32              // the responding qol-tray's own pid
role: "stable" | "shadow"     // matches dev_generation::is_shadow()
bind_port: u16                // 42700 by default; shadows bind elsewhere until promoted
daemon_autostart_held: bool
generation_id: Option<String>
plugins: [PluginHealth]
```

`PluginHealth`, modeled as a status enum rather than a bag of independent
booleans/options so illegal combinations (e.g. `pid: None, alive: true`)
cannot be represented:

```
PluginHealth {
  plugin_id: String,
  status: PluginRuntimeStatus,
}

PluginRuntimeStatus =
  | NotExpected                                            // manifest.daemon.enabled == false
  | Down { consecutive_failures: u32, suppressed: bool }    // no living pid
  | Probation { pid: u32, consecutive_failures: u32 }       // alive, pre-STABLE_TICKS
  | Stable { pid: u32 }                                     // alive, past STABLE_TICKS
```

Derived directly from `SupervisorState`'s per-plugin fields: `suppressed` is
`!can_retry()`, `consecutive_failures` is the counter as-is, and the
`Probation`/`Stable` split is whether `stable_since_tick` has been reached
for the current `last_pid`. The endpoint does not add new supervisor state
beyond P2 - it only projects it.

## Client (`qol dev` / `dev_console`)

- Fetch `/api/dev/plugin-health` on the existing `LINKS_REFRESH_INTERVAL` (5s)
  cadence alongside `/api/dev/links` - no new timer.
- `plugin_row_line` gains a second, independent status dimension (running /
  probation / dead / suppressed) alongside the existing linked/stale/linkable
  dot.
- Handoff freeze/thaw (depends on P1): while `dash.is_reloading()`, freeze
  health consumption and render a "handoff in progress" state instead of
  polling. On a confirmed successful promotion (a real `Ok` from
  `restart_child_from_prebuilt`), resume polling the default port. On a
  confirmed failure (a real `Err`), show an explicit "handoff failed" state
  and keep health stale/unknown - never reinterpret a stale predecessor
  response as successor health. Reuse `PROMOTION_TIMEOUT` /
  `SHADOW_READY_TIMEOUT` from `reload.rs` rather than a parallel timeout.

## Testing

- **Supervisor probation** (table-driven): spawn-fails-repeatedly (unchanged,
  still suppresses), spawn-succeeds-but-dies-immediately-repeatedly (the bug
  case - must now suppress after N cycles instead of looping forever),
  spawn-succeeds-and-survives-past-`STABLE_TICKS` (resets to `Stable`),
  dies-after-stable (fresh cycle, does not inherit the old count),
  out-of-band pid change mid-probation (re-enters probation).
- **`restart_child_from_prebuilt`**: on `promote_shadow_generation` returning
  `Err`, assert the function returns `Err`, does not mutate `*child`/`*lines`,
  and terminates/reaps `next`.
- **`plugin_daemon_pids` check**: pure `diagnose()` tests following
  `plugin_process_leaks`/`plugin_staleness`'s existing pattern - empty
  tracked pids is ok, a tracked-but-dead pid warns.
- **Endpoint**: `tokio::test` following `dev_link_handlers.rs` /
  `dev_state_handlers.rs`'s `isolated_env` pattern; assert envelope shape and
  each `PluginRuntimeStatus` variant.

## Risks / open questions

- `STABLE_TICKS` (set to 3, ~15s) is a judgment call - long enough to clear
  the panic-after-spawn case, short enough that a normal restart doesn't read
  "unstable" for too long. Tune empirically if it proves twitchy or slow.
- `MAX_CONSECUTIVE_FAILURES` (existing constant, 5) was tuned under the old,
  broken failure-counting semantics. Revisit once P2 ships.
- Three consumers (doctor check, endpoint, dashboard) means two schemas
  (doctor's plain warn message vs. `PluginHealth`) to keep conceptually in
  sync with the supervisor's internal state if it changes shape later.
  Accepted: the doctor check is intentionally minimal (pid + alive only) and
  does not share a wire format with the endpoint.
