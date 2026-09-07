<div align="center">

# QoL Search

Fuzzy search algorithm used across qol-tools.

</div>

## Quick start

```toml
[dependencies]
qol-search.workspace = true
```

```rust
use qol_search::fuzzy_match;

let m = fuzzy_match("code", "Visual Studio Code").unwrap();
```

For batch matching against the same query, prepare it once with `prepare_fuzzy_query` and `fuzzy_match_prepared`.

## About

Runs four scoring strategies (greedy, boundary-aware, contiguous substring, whole-word match) and picks the best result. Compiles to native Rust and WebAssembly, so browser contexts get the same ranking as the daemon through [qol-wasm](../qol-wasm).

## License

PolyForm Noncommercial 1.0.0
