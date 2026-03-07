# plugin-os-themes

A [qol-tray](https://github.com/qol-tools/qol-tray) plugin for cursor effects and OS-wide theming.

## Features

**Shake-to-grow** — shake your cursor to temporarily scale it up, then it smoothly animates back to normal size.

- Triggered by shaky motion (direction reversals), not just fast movement — gliding across monitors won't activate it
- Cursor grows instantly and shrinks back gradually over configurable steps
- Intermediate movement sustains the grown state via a lower post-trigger threshold
- Smooth bilinear-interpolated scaling applied to all windows

## Build

- `make dev` — builds and installs to the plugin root
- `make release` — optimized build

## Configuration

Settings are editable via the qol-tray UI (Settings → OS Themes → Settings).

| Field | Default | Description |
|---|---|---|
| `velocity_threshold` | `7777` | px/s required to trigger grow |
| `shakiness_threshold` | `48` | path/displacement ratio required (filters out glides) |
| `post_trigger_threshold` | `1500` | px/s to sustain grown state |
| `scale_factor` | `5` | cursor size multiplier when grown |
| `calm_duration_ms` | `1000` | ms of calm before shrinking back |
| `restore_steps` | `15` | animation frames for the shrink-back |

Config is written to `~/.config/qol-tray/plugins/plugin-os-themes/config.json`.

## Runtime Contract

- Command: `plugin-os-themes`
- Actions: `run` (start daemon), `settings` (open config UI)
- Daemon socket: `/tmp/qol-os-themes.sock`
- Platforms: Linux (X11)

## License

MIT
