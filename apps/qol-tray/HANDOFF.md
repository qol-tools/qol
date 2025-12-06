# Session Handoff

## Current State

qol-tray v1.4.3 - Pluggable system tray daemon. Single tray icon opens browser UI at `http://127.0.0.1:42700` for plugin management.

### Recent Changes (Dec 2025)
- Extracted shared modules: `src/version.rs`, `src/paths.rs`
- Flattened nested code to max 1 level (icon.rs, linux.rs, hotkeys/mod.rs)
- Prevented mutex panics in server.rs (proper error handling)
- Removed unused crates (thiserror, notify, image)
- Detached spawned processes from terminal stdio

### What Works
- System tray icon (SNI protocol)
- Browser-based plugin store
- Plugin install/uninstall with one click
- Hotkey configuration per plugin action
- Dual-location config system (survives uninstall/reinstall)
- Daemon plugin support
- Developer mode (`make dev`) with plugin reload
- Auto-update with notification dot and one-click install
- Local releases via `make release` (runs tests, bumps version, builds .deb, uploads to GitHub)

### Architecture
- `src/tray/` - System tray with platform abstraction
- `src/plugins/` - Plugin loading, config management
- `src/features/plugin_store/` - Browser UI server
- `src/hotkeys/` - Global hotkey registration
- `src/updates/` - Auto-update system (GitHub API check, .deb download/install)
- `ui/` - Embedded web UI (rust-embed)

### Developer Mode

`make dev` runs with Developer tab enabled. Press `r` to reload all plugins (stops daemons and restarts them).

### Releasing

`make release` does everything locally:
1. Runs tests (aborts if failing)
2. Bumps patch version in Cargo.toml
3. Builds release binary and .deb
4. Commits, pushes
5. Creates GitHub release with .deb attached

No cloud CI needed for releases.

## Known Issues / TODO

1. **Wayland support** - qol-tray core works on Wayland (SNI tray, browser UI). Individual plugins may use X11-only tools. See each plugin's HANDOFF.md for details.

2. **macOS/Windows** - Planned but not implemented. Platform abstraction exists in `src/tray/platform/`. Auto-update on these platforms just opens releases page.

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
