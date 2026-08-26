# Settings panel navigation latency

Date: 2026-08-26
Host: this workstation, dev build (`target/qol-dev/runtime/...`), tray pid 76194 then 147586.

## Symptom

Opening the aggregated qol settings panel and moving down fast through the
plugin rail produces multi-second stalls on some entries. The panel looks
frozen, and nothing on screen says why.

## What was measured

All numbers are from the live host, not from reading code.

### The open path is not the problem

`prepare_source` runs two HTTP round trips per source (`/config` and
`/config-form`). Measured across the 16 sources:

```
core                     config=1.0ms  config-form=0.6ms (404)
plugin-alt-tab           config=1.5ms  config-form=3.0ms
plugin-bluetooth         config=1.5ms  config-form=3.6ms
plugin-cli-sessions      config=1.3ms  config-form=2.2ms
plugin-controllers       config=1.2ms  config-form=2.3ms
plugin-ide-checkout      config=1.3ms  config-form=2.2ms
plugin-keyremap          config=1.6ms  config-form=1.2ms
plugin-launcher          config=1.2ms  config-form=2.1ms
plugin-lights            config=1.5ms  config-form=3.3ms
plugin-monitor           config=1.4ms  config-form=2.6ms
plugin-os-themes         config=1.3ms  config-form=2.9ms
plugin-pointz            config=1.4ms  config-form=2.3ms
plugin-removeapp         config=1.5ms  config-form=1.3ms
plugin-window-actions    config=1.3ms  config-form=3.1ms
qol-shot                 config=1.6ms  config-form=2.9ms
qol-voice               config=1.3ms  config-form=3.2ms
```

That is roughly 64ms of HTTP for the whole unified panel, and the probe log
agrees: `phase=prepared outcome=ready elapsed_ms=159`, `phase=activate
outcome=opened` 484ms later. Re-focusing an already open window is 4ms.

### The navigation path is the problem

Per-source runtime query cost, measured through the tray HTTP API:

```
/api/core                 profiles                      1 ms      total     1 ms
/api/plugins/plugin-lights                              2 ms      total     2 ms
/api/plugins/plugin-controllers                         3 ms      total     3 ms
/api/plugins/plugin-os-themes                           3 ms      total     3 ms
/api/plugins/plugin-monitor                             4 ms      total     4 ms
/api/plugins/qol-shot                                  10 ms      total    10 ms
/api/plugins/qol-voice     (6 queries, worst 20 ms)               total    34 ms
/api/plugins/plugin-bluetooth (6 queries, worst 19 ms)            total    44 ms
/api/plugins/plugin-pointz connection_info         10408 ms
/api/plugins/plugin-pointz pairing_status          10236 ms       total 20644 ms
```

Repeat sampling of the two pointz queries: 10.42 / 10.24 / 10.24 / 10.24 /
10.24 / 7.08 / 5.00 / 5.00 / 5.00 / 5.00 seconds. Everything else on this host
is between 1ms and 20ms.

## Root cause, three links

### 1. Every rail keystroke restarts the whole poller synchronously

`libs/qol-gpui/src/settings_panel/view.rs:706` `select_source` ends with:

```rust
self.pause_runtime_poll();
self.resume_runtime_poll(cx);
```

`start_runtime_poll` seeds `due = vec![Instant::now(); n]`
(`sample_queries_at_contract_cadence`), so **all** of the newly selected
source's queries fire immediately, sequentially, on entry. There is no
debounce. Holding Down through the rail fans out every source's query set in
turn. Call sites are the rail arrow keys (`on_rail_key` ->
`step_selected_source`), rail clicks, and `retarget_focus`.

Body navigation (moving between rows inside one plugin) does not restart the
poller. The spike belongs specifically to the rail, which is what "moving down
through the different views" hits.

### 2. A query has a 10 second ceiling and cannot be cancelled

`apps/qol-tray/src/plugins/action_transport/platform/unix.rs:9`

```rust
pub(super) const DEFAULT_IO_TIMEOUT: Duration = Duration::from_secs(10);
```

`dispatch_query` uses this for both the initial dispatch and, after a 3s
`DAEMON_READY_TIMEOUT`, a retry. Worst case for a single query is therefore
10 + 3 + 10 seconds.

Cancellation is cooperative: `pause_runtime_poll` flips an `AtomicBool` that
`sample_queries_at_contract_cadence` only checks at the top of its loop, so a
query already in flight runs to completion. Scrolling fast leaves N
uninterruptible blocking tasks occupying gpui background executor threads.

The tray's HTTP server itself is not serialised: with a pointz query hanging
for 10.4s, three concurrent `/api/core/queries/profiles` calls returned in
1.3ms, 1.1ms and 0.9ms. The stall is per-source, not global.

### 3. The settings host silently pays the timeout twice

`libs/qol-gpui/src/settings_panel/persistence.rs` builds its client with
`with_io_timeout(Duration::from_secs(2))`, but `Session::request`
(`libs/qol-runtime/src/local_http.rs:123`) retries on **any** error:

```rust
match self.request_on_open_connection(method, path, body) {
    Ok(response) => Ok(response),
    Err(_) => {
        self.stream = None;
        self.request_on_open_connection(method, path, body)
    }
}
```

A read timeout is indistinguishable from a stale keep-alive socket here, so a
slow query costs two full budgets and issues a second complete dispatch on the
tray side. The retry exists for connection reuse, not for timeouts.

## Why pointz specifically (a separate plugin bug)

The tray is not adding the latency. Talking to the daemon socket directly:

```
$ printf '{"action":"pairing_status"}\n' | nc -U .../qol-pointz.sock
{"status":"handled","data":{"pairing_open":false,"pin":null,"seconds_remaining":0}}
0.33 s
... 5.00 s
... 5.00 s
```

`pairing_status` is an in-memory read (`plugins/pointz/src/app/daemon.rs:51`
-> `crate::security::pairing_status_json()`), and `connection_info` is
`gethostname` plus `get_if_addrs`. Neither can take seconds. The daemon's
request handling shares a thread with a 5 second periodic task
(`plugins/pointz/src/input/mod.rs:13` `SCREEN_BOUNDS_REFRESH =
Duration::from_secs(5)`), so a request lands at a random phase of that cycle:
0.33s if it arrives late in the period, 5.00s if it arrives just after a tick.
Average around 2.5s, worst case 5s, and two queries back to back reach 10s.

This is a pointz defect and is not fixed by anything in the panel. It is,
however, the thing that makes the panel's architectural weakness visible.

## Is the fix per-setting or higher abstraction?

Higher abstraction. Nothing about any individual field is slow. The cost lives
in three shared layers, each of which serves every plugin and every
query-backed row:

- the panel's poll lifecycle (`select_source` / `start_runtime_poll`),
- the tray's query transport budget (`DEFAULT_IO_TIMEOUT`),
- the local HTTP session retry rule.

Fixing pointz would hide the symptom on this host and leave the next slow
plugin to rediscover it.

## Proposed changes

Ordered by value per unit of risk.

### A. Debounce the source switch

`select_source` should not start sampling on the keystroke. Arm a short timer
(120 to 150ms) and only start the poller once the rail selection settles.
Holding Down through 15 sources then costs one poller start instead of 15.
This is local to `select_source` / `start_runtime_poll` and is the single
biggest win for the reported symptom.

### B. Give queries an interactive latency budget

Split the transport budget by intent. An action is user-initiated and may
legitimately take seconds; a query feeds a settings row and is a read for
display. Give `dispatch_query` its own ceiling in the hundreds of
milliseconds and return "unavailable" past it rather than stalling. The
codebase already accepts this distinction: `QUERY_DAEMON_READY_PROBE_TIMEOUT`
is 100ms while `DAEMON_READY_TIMEOUT` is 3s.

### C. Make the session retry precise

Retry only on connection-reuse failures (broken pipe, EOF on a kept-alive
socket). A timeout must surface as a timeout, once. Today it silently doubles
the wait and doubles the load on the tray.

### D. Deferral with three visible row states

Non-negotiable 6 says failures are visible and self-explanatory. Today a
wedged daemon and a healthy one both render as plain default values, which is
exactly the silent failure the mission forbids.

Render rows immediately from config values, and give every query-backed row
three states:

- **loading**: first sample in flight, show a subdued placeholder on the value,
- **stale**: last good value retained, refresh pending, marked lightly,
- **unavailable**: query failed or exceeded its budget, show the reason inline
  and offer retry.

`apply_frame_paced_samples` already applies samples incrementally, so the
missing pieces are the initial state and the switch debounce, not a new
pipeline.

With B and D in place the residual latency stops reading as "the app froze"
and starts reading as "plugin-pointz is not answering", which is the honest
report and points at the real bug.

### E. Separately, fix pointz

Move the pointz daemon's request handling off the thread that owns the 5
second screen-bounds refresh. Not part of the panel work.

## Instrumentation landed

Two debug-only probes, both compiled out in release:

- `libs/qol-gpui/src/settings_panel/persistence.rs`: the existing
  `SURFACE_ACTIVATION ... phase=runtime-query` line now carries `elapsed_ms`.
- `libs/qol-gpui/src/settings_panel/view.rs`: `select_source` emits
  `SETTINGS_NAV plugin=<id> phase=source-switch queries=<n>`.

Reader: `scratchpad/navtrace.py <epoch_ms>` correlates switches with the
queries they fire and flags anything at or above 250ms. Trace file is
`/tmp/qol-altmon.log`.
