# plugin-launcher

GPUI-native search launcher for qol-tray.

## Controls

| Key | Action |
|-----|--------|
| Type | Fuzzy search apps/files |
| Tab | Switch mode (Apps / Files) |
| Ctrl+Up/Down | Adjust fuzziness (Strict / Balanced / Loose) |
| Up/Down | Navigate results |
| Enter | Launch selected |
| Esc | Quit |

## Build

```
cargo build --release
```

Binary: `target/release/launcher`

## Examples

POC prototypes from development are in `examples/`.

```
make list              # list available examples
make example-01        # run by number
make example-minimal   # run by name fragment
```

## License

MIT
