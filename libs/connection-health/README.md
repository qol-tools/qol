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
`RuntimeEventKind::ConnectionHealthChanged`. The tray replays the current health
on subscribe and publishes again whenever the level or its cause changes.

```rust
use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};

if let RuntimeEvent::ConnectionHealthChanged { health } = event {
    let degraded = health.is_degraded();
}
```

A native surface colours its busy indicator with `Kit::connection_hue(level)`,
or `settings_connection_spinner` inside settings pages.

## About

qol-tray owns the only `Tracker`. Every tray HTTP request goes through
`SendObserved::send_observed`, which times the request until the server
answers. While a page or a subscriber is watching, the tray also requests
`https://api.github.com/rate_limit`, which does not count against the GitHub
rate limit and goes through the same proxy settings as every other request. It
probes when nothing else has measured recently, or when a request has waited
past the bad threshold. An answer to a request started later supersedes an
older request still in flight, so one stalled request cannot hold the signal
at `bad`.

| Level | Meaning |
|---|---|
| `unknown` | Nothing measured within the memory window |
| `good` | Answers arrive faster than the slow threshold |
| `slow` | Answers are slow, or a request has been waiting past the slow threshold |
| `bad` | The server could not be reached, or a request has been waiting past the bad threshold |

`cause` says which: `slow_answers`, `waiting` or `unreachable`. The thresholds
are the caller's policy and live with the tray service, not in this crate.

## License

PolyForm Noncommercial 1.0.0. See [LICENSE](LICENSE).
