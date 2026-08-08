<div align="center">

# QoL Color

Hex color parsing for qol-tools.

</div>

## Quick start

```toml
[dependencies]
qol-color.workspace = true
```

```rust
let (r, g, b) = qol_color::parse_hex_color("1a1e2a").unwrap();
```

Returns `None` for invalid input.

## License

PolyForm Noncommercial 1.0.0
