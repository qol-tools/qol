# Configuration

## Config File Location

The plugin checks these locations (first valid JSON wins):

- Install-scoped paths under active QoL Tray install:
  - `plugins/plugin-alt-tab/config.json`
  - `plugins/alt-tab/config.json`

Where `<base>` = `dirs::data_local_dir()/qol-tray` (Linux: `~/.local/share/qol-tray`, macOS: `~/Library/Application Support/qol-tray`).

Install ID is resolved from `$QOL_TRAY_INSTALL_ID` env var or `<base>/active-install-id` file.

## Schema

```json
{
  "action_mode": "hold_to_switch",
  "reset_selection_on_open": true,
  "open_behavior": "cycle_once",
  "display": {
    "max_columns": 6,
    "transparent_background": false,
    "card_background_color": "1a1e2a",
    "card_background_opacity": 0.85
  },
  "label": {
    "show_app_name": true,
    "show_window_title": true
  }
}
```

## Fields

### Top-level

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `action_mode` | `"sticky"` \| `"hold_to_switch"` | `"hold_to_switch"` | Sticky keeps picker open until Enter/Esc. Hold-to-switch activates on Alt release. |
| `reset_selection_on_open` | bool | `true` | Reset selection to first item each time picker opens. |
| `open_behavior` | `"cycle_once"` \| `"show_only"` | `"cycle_once"` | Whether opening the picker also advances selection by one. |

### `display`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `max_columns` | int (2-12) | `6` | Maximum number of columns in the grid. |
| `transparent_background` | bool | `false` | Remove the window background so only preview cards are visible. |
| `card_background_color` | hex string | `"1a1e2a"` | Card fill color in transparent mode (6-char hex, no `#` prefix). |
| `card_background_opacity` | float (0.0-1.0) | `0.85` | Card opacity in transparent mode. |

### `label`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `show_app_name` | bool | `true` | Show app name in card label. |
| `show_window_title` | bool | `true` | Show window title in card label. |

## Legacy Keys

- `display.preview_mode` and `display.preview_fps` are accepted by serde but have no effect on runtime behavior. They are preserved in config for backwards compatibility.

Unknown fields are ignored by default serde behavior.
