# My Plugin

A binary-first `qol-tray` plugin template.

## Build

- `make dev` builds `plugin-template` and copies it to the plugin root
- `make release` builds an optimized `plugin-template` and copies it to the plugin root

## Runtime Contract

- Runtime command: `plugin-template`
- Default action: `run`
- Action mapping: `run -> ["run"]`
- No daemon by default (add `[daemon]` only when needed)

## Customize

1. Rename `plugin-template` in `Cargo.toml`, `plugin.toml`, and `Makefile`
2. Implement your action handling in `src/main.rs`
3. Update `plugin.toml` menu/actions to match your runtime contract
4. Update `[[dependencies.binaries]]` to your release repository and pattern

## Contract Notes

- Commands must be binary basenames (`[A-Za-z0-9_-]+`)
- If `runtime.actions` is present, map every executable menu action
- If using a daemon socket, use absolute socket paths and optional `daemon.action_aliases` for action renames

## Usage

Trigger plugin actions from QoL Tray UI and bind them to global hotkeys as needed.

## License

MIT
