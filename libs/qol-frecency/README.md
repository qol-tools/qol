# qol-frecency

Frequency + recency ranking for qol-tools plugins. Tracks how often and how recently items were accessed, then applies exponential decay so stale entries fade out naturally.

Used by plugin-launcher to rank search results by usage patterns.

## Usage

```rust
use qol_frecency::{FrequencyData, record, frequency_bonus, load, save, default_store_path};
use std::time::{SystemTime, UNIX_EPOCH};

let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
let path = default_store_path("my-plugin");
let mut data = load(&path);

// Record a launch
record(&mut data, "firefox".into(), now);

// Score bonus (lower = better, applied to fuzzy match score)
let bonus = frequency_bonus("firefox", &data, now, 7.0, -20);

save(&path, &data);
```

`prune()` removes entries that have decayed below a threshold and caps the store at 1000 entries.

## License

PolyForm Noncommercial 1.0.0
