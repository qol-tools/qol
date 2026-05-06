# qol-frecency

[![CI](https://github.com/qol-tools/qol-frecency/actions/workflows/ci.yml/badge.svg)](https://github.com/qol-tools/qol-frecency/actions/workflows/ci.yml)

Frequency + recency ranking for qol-tools plugins.

## Quick start

```toml
[dependencies]
qol-frecency = { git = "https://github.com/qol-tools/qol-frecency" }
```

```rust
use qol_frecency::{record, frequency_bonus, load, save, default_store_path};
use std::time::{SystemTime, UNIX_EPOCH};

let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
let path = default_store_path("my-plugin");
let mut data = load(&path);

record(&mut data, "firefox".into(), now);
let bonus = frequency_bonus("firefox", &data, now, 7.0, -20);
save(&path, &data);
```

## About

Tracks how often and how recently items were accessed, then applies exponential decay so stale entries fade out naturally.

## License

PolyForm Noncommercial 1.0.0
