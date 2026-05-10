# qol-search

[![tests](https://github.com/qol-tools/qol-search/actions/workflows/tests.yml/badge.svg)](https://github.com/qol-tools/qol-search/actions/workflows/tests.yml)
[![lint](https://github.com/qol-tools/qol-search/actions/workflows/lint.yml/badge.svg)](https://github.com/qol-tools/qol-search/actions/workflows/lint.yml)

Fuzzy search algorithm used across qol-tools. Compiles to native Rust and WebAssembly.

## Quick start

```toml
[dependencies]
qol-search = { git = "https://github.com/qol-tools/qol-search" }
```

```rust
use qol_search::fuzzy_match;

let m = fuzzy_match("code", "Visual Studio Code").unwrap();
```

For batch matching against the same query, prepare it once with `prepare_fuzzy_query` + `fuzzy_match_prepared`.

## About

Runs four scoring strategies (greedy, boundary-aware, contiguous substring, whole-word match) and picks the best result. Wrapped by [qol-wasm](https://github.com/qol-tools/qol-wasm) for browser contexts.

## License

PolyForm Noncommercial 1.0.0
