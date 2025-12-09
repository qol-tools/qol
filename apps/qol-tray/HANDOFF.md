# Session Handoff

## Current State

qol-tray v1.4.3 - Pluggable system tray daemon. Single tray icon opens browser UI at `http://127.0.0.1:42700` for plugin management.

**Cross-platform:** Builds and tests pass on Linux, Windows, macOS. Plugins declare platform support via `platforms` field.

### Recent Changes (Dec 2025)
- Security: Path traversal prevention via canonicalization in plugin UI file serving
- Security: Null byte injection prevention in plugin IDs and file paths
- Fix: Return actual version after plugin install (was hardcoded "1.0.0")
- Fix: Handle whitespace in version parsing
- Fix: Proper TOML parsing for plugin name extraction
- Refactor: Removed unused `plugin_manager` param from menu/tray creation
- Refactor: Removed unused WebSocket endpoint stub
- Refactor: Removed unused `restart_on_crash` field from DaemonConfig
- Refactor: Removed dead code (ActionType::Custom, EventHandler::Async, UiServerHandle)
- Refactor: Simplified UI server startup (no more mem::forget pattern)
- Cross-platform CI via GitHub Actions (Linux, Windows, macOS)
- Plugin platform filtering - `platforms = ["linux"]` in plugin.toml, filters both installed plugins and store listings
- Fixed Windows hotkey manager thread safety issue
- Shared CLAUDE.md in parent `/Git/` directory for all qol-tools repos
- Cross-platform URL opening via `open` crate
- Windows/macOS tray event handling implemented
- Merged windows.rs and macos.rs into single `#[cfg(not(target_os = "linux"))]` block
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

| Plugin | Platforms | Status |
|--------|-----------|--------|
| plugin-launcher | All | Working |
| plugin-pointz | All | Working |
| plugin-screen-recorder | Linux | Working |
| plugin-window-actions | Linux | Working |

## Config Locations

- Plugin configs: `~/.config/qol-tray/plugins/{plugin-id}/config.json`
- Config backup: `~/.config/qol-tray/plugin-configs.json`
- Hotkeys: `~/.config/qol-tray/hotkeys.json`
