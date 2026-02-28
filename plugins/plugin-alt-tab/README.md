# Alt Tab Plugin for QoL Tray

A window switcher with live previews for [QoL Tray](https://github.com/qol-tools/qol-tray). Shows a grid of open windows with thumbnails and app icons, activated via global hotkey.

## Features

- **Live preview grid** with window thumbnails and app icons
- **Two action modes**: Sticky (stays open until Enter/Esc) or Hold-to-Switch (activates on Alt release)
- **Transparent background mode** with configurable card color and opacity
- **Configurable layout**: grid columns, label formatting, card appearance
- **Background preview cache** keeps thumbnails warm for near-instant activation
- **Web-based settings** with live grid visualizer

## Controls

| Key | Action |
|-----|--------|
| Arrow keys | Navigate the grid |
| Tab / Shift+Tab | Cycle through windows |
| Enter | Activate selected window |
| Escape | Dismiss without switching |
| Alt release | Activate (hold-to-switch mode) |

## Configuration

Configured via `config.json` or the QoL Tray settings UI.

- `display.max_columns` — Grid column count (2-12)
- `display.transparent_background` — Remove window background, show only cards
- `display.card_background_color` — Card fill color in transparent mode (hex, e.g. `"1a1e2a"`)
- `display.card_background_opacity` — Card opacity in transparent mode (0.0-1.0)
- `action_mode` — `sticky` or `hold_to_switch`
- `label.show_app_name` / `label.show_window_title` — Toggle label content

## Platform Support

| Platform | Status |
|----------|--------|
| macOS | Supported |
| Linux (X11) | Supported |

License: MIT
