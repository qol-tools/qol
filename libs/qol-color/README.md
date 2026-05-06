# qol-color

[![CI](https://github.com/qol-tools/qol-color/actions/workflows/ci.yml/badge.svg)](https://github.com/qol-tools/qol-color/actions/workflows/ci.yml)

Hex color parsing for qol-tools.

## Quick start

```toml
[dependencies]
qol-color = { git = "https://github.com/qol-tools/qol-color" }
```

```rust
let (r, g, b) = qol_color::parse_hex_color("1a1e2a").unwrap();
```

Returns `None` for invalid input.

## License

PolyForm Noncommercial 1.0.0
