# My Plugin

A binary-first `qol-tray` plugin template.

## Build

- `make dev` builds `plugin-template` and copies it to the plugin root
- `make release` builds an optimized `plugin-template` and copies it to the plugin root

## Runtime Contract

- Runtime command: `plugin-template`
- Default action: `run`
- Action mapping: `run -> ["run"]`

## Customize

1. Rename `plugin-template` in `Cargo.toml`, `plugin.toml`, and `Makefile`
2. Implement your action handling in `src/main.rs`
3. Update `plugin.toml` menu/actions to match your runtime contract
4. Update `[[dependencies.binaries]]` to your release repository and pattern

## License

MIT
