# qol-search

Fuzzy search algorithm used across qol-tools. Compiles to both native Rust and WebAssembly.

Runs four scoring strategies (greedy, boundary-aware, contiguous substring, whole-word match) and picks the best result. Scoring rewards boundary alignment, case matches, and contiguity.

## Usage

```rust
use qol_search::{fuzzy_match, FuzzyMatch};

let m = fuzzy_match("code", "Visual Studio Code").unwrap();
// m.score: lower is better
// m.positions: matched character indices
```

For batch matching against the same query, prepare it once:

```rust
use qol_search::{prepare_fuzzy_query, fuzzy_match_prepared};

let query = prepare_fuzzy_query("code");
for candidate in candidates {
    if let Some(m) = fuzzy_match_prepared(&query, candidate) {
        // ...
    }
}
```

## WebAssembly

This crate is wrapped by [qol-wasm](https://github.com/qol-tools/qol-wasm) for use in browser contexts. The qol-tray CommandPalette uses the wasm build for fuzzy filtering.

## License

PolyForm Noncommercial 1.0.0
