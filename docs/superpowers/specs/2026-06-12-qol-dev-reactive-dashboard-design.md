# qol dev reactive dashboard

## Problem

The `qol dev` TUI dashboard polls tray/web health (2s) and emu (5s), and streams logs/trace, but the doctor row runs once at session start and then only on keypress. Divergences (stale plugin daemons, formatting drift, broken dev links) stay invisible until the user manually runs doctor. The plugins row shows a static boot-time count that is never re-verified, and the endpoints view probes once on entry and never refreshes.

## Goals

- Divergences surface on the dashboard without manual action.
- Cheap at idle: probe threads park between ticks; work that found nothing new decays its cadence.
- Dirty-aware: meaningful events (rebuild, reload, emu run) trigger immediate re-checks; the periodic timer is only a fallback.
- No visual churn: auto-refreshes update silently; the row never flashes a transient "checking" state.

## Non-goals

- Filesystem watching. Event pokes plus a decaying timer approximate dirtiness well enough.
- Changing doctor checks themselves or the `qol-tray-doctor` binary.
- Push/SSE from the dev server. Polling stays the transport.

## Architecture

### `Poller<T>` (new module `tools/qol-cli/src/poller.rs`)

One generic primitive replaces the bespoke probe threads:

```rust
pub(crate) struct Poller<T> {
    rx: Receiver<T>,
    poke_tx: Sender<()>,
}

impl<T: Send + 'static> Poller<T> {
    pub fn spawn(interval: Duration, work: impl FnMut() -> T + Send + 'static) -> Self;
    pub fn spawn_adaptive(base: Duration, cap: Duration, work: impl FnMut() -> T + Send + 'static) -> Self
    where T: PartialEq; // doctor only
    pub fn latest(&self) -> Option<T>; // drains the channel, returns the newest result
    pub fn poke(&self);                // marks dirty: immediate re-run, resets adaptive cadence
}
```

Thread loop: run `work()`, send the result, then block on `poke_rx.recv_timeout(interval)`. A poke wakes it immediately (a poke that arrived mid-run wakes it without waiting, and queued pokes collapse into one re-run); a timeout runs the periodic pass. Dropping the `Poller` closes both channels; the thread exits after its current iteration. Zero allocation and zero CPU while parked.

Adaptive mode (doctor): the work closure reports whether the result changed (`T: PartialEq` against the previous result, handled inside the poller). Unchanged result and no poke: the wait doubles (base 10s up to cap 60s). Changed result or poke: reset to base.

### Probe inventory

| Probe | Interval | Pokes (dirty signals) |
|---|---|---|
| health (api + web) | 2s fixed | - |
| emu | 5s fixed | emu view entry, emu run finished |
| links (`GET /api/dev/links`) | 5s fixed | plugin reload sent, first health-up |
| doctor auto | 10s base, 60s cap, adaptive | rebuild ack, plugin reload done, emu run done, manual doctor run done, first health-up |
| endpoints | 5s fixed, exists only while view open | view entry (initial spawn) |

`Dash` holds `Poller` fields instead of `Receiver` + spawn-state fields; the tick loop calls `latest()` uniformly. The endpoints poller is `Option<Poller<_>>`: created on view entry, dropped on Back. The tick loop owns the "first health-up" signal: on observing the health snapshot transition to Up, it pokes the links and doctor pollers once.

## Doctor semantics

- **Auto runs skip the cargo build.** They execute the existing `target/debug/qol-tray-doctor check` binary directly (measured: 1.2s wall, 0.6s CPU). If the binary is missing, the auto run reports "doctor binary not built" without triggering a build.
- **Manual runs are unchanged**: `d` / enter rebuilds then checks; armed enter runs fix. Manual state renders "checking"/"fixing" as today.
- **Silent refresh**: while an auto run is in flight, the row keeps showing the last completed report. New state model separates `last: Option<DoctorRun>` + `manual_in_flight: Option<Receiver<...>>` instead of the current replace-on-run enum.
- **Freshness**: the row appends a dim `· 12s ago` age (reuses `relative_age`). The doctor view title shows the same age.
- **Manual/auto interplay**: when a manual run completes, the auto poller is dropped and respawned, discarding any stale in-flight auto result, then poked.
- **Failure**: a failed auto run keeps the last good report and appends a dim `· probe failed` marker; only manual runs surface full failure text.

## Trace auto-start

The tracer (`tools/compact_trace.py`) currently starts on first dive into the trace view and stops on Back. It becomes session-long:

- Spawned once at TUI session start, so probe events are captured from boot instead of from first view entry.
- Back from the trace view no longer stops the tracer; it keeps buffering into the existing 2000-line ring. The dashboard trace row shows the live line count from boot.
- The tracer still stops on quit, reload, and child exit (the existing `stop_trace` teardown paths).
- If python3 or the script is unavailable, the trace row shows a dim `tracer unavailable` and the trace view keeps today's explanatory message. No retry loop.

## Plugins row

`GET /api/dev/links` returns `Vec<LinkedPlugin>` with `id`, `name`, `needs_rebuild`, `rebuild_reason`. The row becomes live:

- `7 linked` (green) when all linked and fresh.
- `7 linked · 2 stale` (yellow) when any `needs_rebuild`.
- Boot count in dim with `· api down` when the server is unreachable (no red panic; tray row already signals downtime).

The plugins view lists each plugin with its live link state and `rebuild_reason` for stale ones, replacing the hardcoded "dev-linked" suffix.

## HTTP helper

`dev_server.rs` gains `http_get_json<T: DeserializeOwned>(url) -> Result<T>` reusing the existing raw-socket client and 1s timeout. serde_json is already a dependency; `LinkedPlugin` gets a local mirror struct with only the consumed fields.

## Error handling

- Poller `work` closures return `Result`-shaped types where probes can fail (links, emu, doctor); the tick loop maps failures to the row fallbacks above. No panics across the channel.
- A wedged probe (e.g. HTTP timeout) delays only its own next tick; the 1s HTTP timeout bounds each iteration.
- Threads never outlive the session: drop order in `Dash` tears down all pollers on quit/reload/crash.

## Testing

- `poller.rs`: table-driven unit tests - periodic tick delivers results, poke triggers immediate re-run, adaptive backoff doubles on unchanged results and resets on poke, drop terminates the thread (bounded join).
- Links JSON parse: fixture deserialization including unknown-field tolerance.
- Row builders: pure-function tests for plugins/doctor span text across states (fresh, stale, api down, aged report), generic names per test style.
- Existing `dev_console` tests (action mapping, cursor clamps) remain green.

## Out of scope

- Doctor check content, fix mode behavior.
- Log/trace rendering (already streaming).
- The non-TUI plain session path (verbose mode stays as is).
