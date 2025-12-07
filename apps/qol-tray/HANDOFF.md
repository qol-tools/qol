# Session Handoff

## Current State

qol-tray v1.4.3 - Pluggable system tray daemon for Linux. Single tray icon opens browser UI at `http://127.0.0.1:42700` for plugin management.

**Linux only for now.** Cross-platform support planned for the future.

### Recent Changes (Dec 2025)
- Cross-platform URL opening via `open` crate (replaced 28-line platform-specific code)
- Windows/macOS tray event handling implemented (was dead code)
- Merged windows.rs and macos.rs into single `#[cfg(not(target_os = "linux"))]` block in mod.rs
- Comprehensive edge case tests for version parsing, path safety, hotkey parsing, plugin loading
- Developer tab with plugin linking/unlinking for dev workflow
- Auto-reload after link/unlink (keeps PluginManager in sync)
- Backup/restore of installed plugins when linking/unlinking
- Keyboard navigation in Developer tab (arrow keys, Space/Enter)
- Extracted shared modules: `src/version.rs`, `src/paths.rs`
- Plugin version from `plugin.toml` (not git tags)
- Plugin update uses `git fetch && reset --hard` (handles divergent branches)
- `make dev` kills existing qol-tray process before starting

### What Works
- System tray icon (SNI protocol)
- Browser-based plugin store
- Plugin install/uninstall with one click
- Hotkey configuration per plugin action
- Dual-location config system (survives uninstall/reinstall)
- Daemon plugin support
- Developer mode (`make dev`) with plugin linking and reload
- Auto-update with notification dot and one-click install
- Local releases via `make release`

### Architecture
- `src/tray/` - System tray (Linux SNI, Windows/macOS via tray-icon)
- `src/plugins/` - Plugin loading, config management
- `src/features/plugin_store/` - Browser UI server
- `src/hotkeys/` - Global hotkey registration
- `src/updates/` - Auto-update system
- `src/paths.rs` - Shared paths and `open_url()` via `open` crate
- `src/version.rs` - Semantic version parsing and comparison
- `ui/` - Embedded web UI (rust-embed)

### Developer Mode

`make dev` runs with Developer tab enabled:
- Link plugins from dev directories (symlinks)
- Unlink restores original installed version from backup
- Press `r` to reload all plugins (stops daemons and restarts)
- Arrow keys navigate, Space/Enter to toggle

### Releasing

`make release` does everything locally:
1. Runs tests (aborts if failing)
2. Bumps patch version in Cargo.toml
3. Builds release binary and .deb
4. Commits, pushes
5. Creates GitHub release with .deb attached

No cloud CI needed.

## Known Issues / TODO

1. **Wayland support** - Core works on Wayland. Individual plugins may use X11-only tools.

## Plugins

| Plugin | Status |
|--------|--------|
| plugin-launcher | Working |
| plugin-pointz | Working |
| plugin-screen-recorder | Working |
| plugin-window-actions | Working |

## Config Locations

- Plugin configs: `~/.config/qol-tray/plugins/{plugin-id}/config.json`
- Config backup: `~/.config/qol-tray/plugin-configs.json`
- Hotkeys: `~/.config/qol-tray/hotkeys.json`
