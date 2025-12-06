# Session Handoff

## Current State

qol-tray is a pluggable system tray daemon. Single tray icon opens browser UI at `http://127.0.0.1:42700` for plugin management.

### What Works
- System tray icon (SNI protocol)
- Browser-based plugin store
- Plugin install/uninstall with one click
- Hotkey configuration per plugin action
- Dual-location config system (survives uninstall/reinstall)
- Daemon plugin support
- Developer mode (`make dev`) with plugin reload

### Architecture
- `src/tray/` - System tray with platform abstraction
- `src/plugins/` - Plugin loading, config management
- `src/features/plugin_store/` - Browser UI server
- `src/hotkeys/` - Global hotkey registration
- `ui/` - Embedded web UI (rust-embed)

### Developer Mode

`make dev` runs with Developer tab enabled. Press `r` to reload all plugins (stops daemons and restarts them).

## Known Issues / TODO

1. **Wayland support** - qol-tray core works on Wayland (SNI tray, browser UI). Individual plugins may use X11-only tools. See each plugin's HANDOFF.md for details.

2. **macOS/Windows** - Planned but not implemented. Platform abstraction exists in `src/tray/platform/`.

## Plugins

| Plugin | Status | Wayland |
|--------|--------|---------|
| plugin-launcher | Working | Needs work (xdotool, xclip) |
| plugin-pointz | Working | Should work |
| plugin-screen-recorder | Working | Needs work (xrandr, slop) |
| plugin-window-actions | Working | Needs work (xdotool, xprop, wmctrl) |

## Config Locations

- Plugin configs: `~/.config/qol-tray/plugins/{plugin-id}/config.json`
- Config backup: `~/.config/qol-tray/plugin-configs.json`
- Hotkeys: `~/.config/qol-tray/hotkeys.json`
