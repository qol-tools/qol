# qol-color

Hex color parsing for qol-tools. No dependencies beyond std.

## Usage

```rust
use qol_color::parse_hex_color;

let (r, g, b) = parse_hex_color("1a1e2a").unwrap();
let (r, g, b) = parse_hex_color("#1a1e2a").unwrap(); // leading # is optional
```

Returns `None` for invalid input.

## License

PolyForm Noncommercial 1.0.0
