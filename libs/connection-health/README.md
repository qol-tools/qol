<div align="center">

# QoL Connection Health

One connection health signal for qol-tray and every plugin.

</div>

## Quick start

```toml
[dependencies]
qol-connection-health.workspace = true
```

A plugin reads the tray's signal by subscribing to
`RuntimeEventKind::ConnectionHealthChanged`. On subscribe the tray replays the
health it last published, then publishes again whenever the level or its cause
changes. Subscribe to it on its own connection: a tray older than this event
refuses the whole subscribe request, so mixing it with other kinds would lose
those too.

```rust
use qol_connection_health::Level;
use qol_runtime::protocol::RuntimeEvent;

if let RuntimeEvent::ConnectionHealthChanged { health } = event {
    let degraded = matches!(health.level, Level::Slow | Level::Bad);
}
```

A native surface colours its busy indicator with `Kit::connection_hue(level)`,
or `settings_connection_spinner` inside settings pages. `unknown` takes a
neutral colour, `good` green, `slow` yellow and `bad` red in every theme.

## About

qol-tray owns the only `Tracker`. Every tray HTTP request goes through
`SendObserved::send_observed`, which times the request until the server
answers; clippy refuses a plain `RequestBuilder::send` in the tray. A download
reads its body through `observe_body`, so a body that stops arriving counts as
waiting and a body cut short counts as unreachable.

While a settings page is open or a plugin subscribes, the tray also requests
`https://api.github.com/rate_limit`. That endpoint does not count against the
GitHub rate limit, and the request goes through the same proxy environment
variables as every other one. The tray probes the moment a
request waits past the bad threshold, and otherwise when nothing has measured
recently: every minute while `good`, and from every 10 seconds backing off to
every minute while probes keep failing. Only one probe runs at a time.

An answer to a request started later supersedes an older request still in
flight, whichever host either went to, so one stalled request cannot hold the
signal at `bad` while the connection itself answers.

Traffic outside the tray's reqwest client is not observed: git operations in
profile sync and the installer, and the settings migrations gist store in
`qol-migrations`.

| Level | Meaning |
|---|---|
| `unknown` | Nothing measured within the memory window |
| `good` | Answers arrive faster than the slow threshold |
| `slow` | Answers are slow, or a request has been waiting past the slow threshold |
| `bad` | The server could not be reached, or a request has been waiting past the bad threshold |

`cause` says which: `slow_answers`, `waiting` or `unreachable`. A level or cause
this version does not know reads as `unknown` or no cause, so a newer tray never
breaks an older reader. The thresholds are the caller's policy and live with the
tray service, not in this crate.

## License

PolyForm Noncommercial 1.0.0. See [LICENSE](LICENSE).
