# plugin-alt-tab

High-performance Alt-Tab switcher for QoL Tray.

## Current Behavior

- One visual mode: a brick/grid layout with live previews.
- Fast open path: the picker window opens immediately, then window data and previews stream in.
- Consistent preview sizing: every thumbnail is rendered to a fixed canvas size.
- Grid navigation:
  - `Left/Right/Up/Down` follow visual grid movement.
  - `Tab` and `Shift+Tab` cycle forward/backward.
  - `Enter` activates selected window.
  - `Esc` closes the picker.

## Platform Notes

- Primary target is Linux on X11.
- Non-Linux builds compile with stubbed window discovery/preview capture.

## Architecture (Short)

- `src/main.rs`
  - GPUI app/view, picker lifecycle, keyboard handling, async UI updates.
- `src/x11_utils.rs`
  - X11 window discovery and preview capture/processing.
- `src/daemon.rs`
  - Unix socket command listener (`--show`, `--kill`) for persistent daemon workflow.
- `src/monitor/`
  - Active monitor tracking for picker placement.

See `docs/ARCHITECTURE.md` for details.

## Development

```bash
cargo test
cargo run -- --show
```

To terminate daemon:

```bash
cargo run -- --kill
```

## Configuration

The plugin currently runs in a single layout mode. Historical `preview_mode` values are legacy and no longer drive runtime layout behavior.

See `docs/CONFIG.md`.
