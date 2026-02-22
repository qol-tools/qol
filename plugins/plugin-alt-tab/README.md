# Alt Tab Plugin for QoL Tray

A high-performance, configurable Alt-Tab switcher built for the [QoL Tray](https://github.com/qol-tools/qol-tray) ecosystem. Features live X11 window previews, fluid GPUI-powered transitions, and deep layout customization.

## Features

-   **Dual Visual Modes**:
    -   `BelowList`: Classic vertical list of windows with previews rendered beneath each entry.
    -   `PreviewOnly`: A modern, preview-first grid layout that treats thumbnails as the primary content.
-   **Configurable Layout**: Define exactly how your grid grows using the `max_columns` setting.
-   **Two Action Modes**:
    -   `Sticky`: The UI stays open until explicitly dismissed with `Enter` or `Esc`.
    -   `Hold-to-Switch`: Direct X11 keyboard polling allows the UI to automatically activate the selected window as soon as you release the `Alt` key.
-   **WYSIWYG Settings**: A built-in web-based configuration page with a live grid visualizer to preview your layout changes before applying them.
-   **Daemon Architecture**: Runs as a persistent background process via Unix sockets for near-instantaneous activation.

## Keyboard Controls

-   **Arrow Keys**: Navigate the visual grid.
-   **Tab / Shift+Tab**: Cycle forward/backward through the window list.
-   **Enter**: Activate the selected window.
-   **Escape**: Dismiss the picker without switching.
-   **Alt Release** (Hold-to-Switch mode): Automatically activates the selected window.

## Configuration

The plugin is configured via `plugin.toml` or through the QoL Tray dashboard.

### `display` Settings
-   `preview_mode`: `below_list` | `preview_only`
-   `max_columns`: Integer (2-12). Controls the grid wrap bias.

### `action_mode` Settings
-   `sticky` | `hold_to_switch`

## Architecture

-   **GPUI Rendering**: Uses the GPUI framework for sleek, hardware-accelerated UI.
-   **X11 Integration**: Direct `x11rb` interaction for window discovery and live thumbnail capture.
-   **Unix Sockets**: Fast IPC for daemon control (`--show`, `--kill`).

## Development

```bash
# Run contract validation tests
cargo test

# Run in development mode (as a tray plugin)
# qol-tray will automatically resolve the binary from target/debug
```

License: MIT

