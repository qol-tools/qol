# Alt Tab Plugin for QoL Tray

A high-performance, configurable Alt-Tab switcher built for the [QoL Tray](https://github.com/qol-tools/qol-tray) ecosystem. Features live window previews via CoreGraphics (macOS) and X11 (Linux), app icons, and a hardware-accelerated GPUI grid.

## Features

-   **Live Preview Grid**: Window thumbnails captured via CG (macOS) or X11 GetImage (Linux), displayed in a responsive grid layout.
-   **App Icons**: 16px icons rendered inline with window labels for instant visual identification.
-   **Transparent Background Mode**: Optional borderless transparent mode where only preview cards float over the desktop. Card background color and opacity are configurable.
-   **Configurable Layout**: Grid column count, label formatting, and card appearance are all adjustable.
-   **Two Action Modes**:
    -   `Sticky`: The UI stays open until explicitly dismissed with `Enter` or `Esc`.
    -   `Hold-to-Switch`: The UI automatically activates the selected window when the `Alt` key is released.
-   **Prewarm Cache**: Background preview capture keeps thumbnails warm between invocations for near-instant picker open.
-   **WYSIWYG Settings**: A built-in web-based configuration page with live grid visualizer.
-   **Daemon Architecture**: Runs as a persistent background process via Unix sockets for near-instantaneous activation.

## Keyboard Controls

-   **Arrow Keys**: Navigate the visual grid.
-   **Tab / Shift+Tab**: Cycle forward/backward through the window list.
-   **Enter**: Activate the selected window.
-   **Escape**: Dismiss the picker without switching.
-   **Alt Release** (Hold-to-Switch mode): Automatically activates the selected window.

## Configuration

The plugin is configured via `config.json` or through the QoL Tray settings UI.

### `display` Settings
-   `max_columns`: Integer (2-12). Controls the grid wrap point.
-   `transparent_background`: Boolean. Removes the window background so only cards are visible.
-   `card_background_color`: Hex string (e.g. `"1a1e2a"`). Card fill color in transparent mode.
-   `card_background_opacity`: Float (0.0-1.0). Card opacity in transparent mode.

### `action_mode` Settings
-   `sticky` | `hold_to_switch`

### `label` Settings
-   `show_app_name`: Boolean. Show app name in card label.
-   `show_window_title`: Boolean. Show window title in card label.

## Architecture

-   **GPUI Rendering**: Uses the GPUI framework for hardware-accelerated UI.
-   **Cross-Platform**: macOS (CoreGraphics + NSRunningApplication), Linux (X11/x11rb), Windows (stub).
-   **Unix Sockets**: Fast IPC for daemon control (`--show`, `--show-reverse`, `--kill`).

## Development

```bash
# Run contract validation tests
cargo test

# Run in development mode (as a tray plugin)
# qol-tray will automatically resolve the binary from target/debug
```

License: MIT
