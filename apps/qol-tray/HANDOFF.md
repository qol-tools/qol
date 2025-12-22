# Session Handoff

## Current State

qol-tray v1.4.3 - Pluggable system tray daemon. Single tray icon opens browser UI at `http://127.0.0.1:42700` for plugin management.

**Cross-platform:** Builds and tests pass on Linux, Windows, macOS. Plugins declare platform support via `platforms` field.

### Recent Changes (Dec 2025)
- Architecture: Introduced `Daemon` and `EventBus` for structured background tasks and event broadcasting.
- Dev Mode: Implemented background plugin discovery with `walkdir` and depth-limited scanning.
- Dev Mode: Unified "Plugin Links" and "Discovered Plugins" into a single, alphabetically sorted list with status badges.
- Dev Mode: Added manual refresh trigger for discovery (via UI button or `r` key).
- Dev Mode: Robust TOML parsing with fallback for minimal manifests during development.
- Dev Mode: Consistent `.refresh-btn` spinner across all loading states (discovery, reload).
- Dev Mode: Fixed arrow key navigation breaking after link/unlink operations.
- Dev Mode: Fixed SSE event subscription not restoring after view blur/focus.
- Dev Mode: Smooth spinner animation during link/unlink (guards against intermediate re-renders).
- Dev Mode: Keyboard shortcuts: `r` rescan, `Ctrl+r` reload (was both on `r`).
- UI: Stable layouts with fixed row heights to prevent jumping on state changes.
- UI: Unified button system (`.btn` variants) and badge styles across views.
- Fix: Discovery now correctly handles broken symlinks and deduplicates by canonical path.
- macOS: Fixed tray icon not appearing (requires NSApplication.run() on main thread)
- macOS: Added Cmd+R support alongside Ctrl+R for refresh in browser UI
- Refactor: Platform-specific tray code split into linux.rs, macos.rs, windows.rs
- Refactor: main.rs no longer contains any platform-specific code
- Security: Symlink rejection for plugin UI files (TOCTOU mitigation)
- Security: Symlink rejection for GitHub token file
- Security: Plugin config JSON size limit (1MB)
- Security: Plugin ID validation at all server endpoints using `is_safe_path_component()`
- Security: Action ID validation in hotkey execution (rejects shell metacharacters, leading dashes)
- Security: Path traversal prevention via shared `is_safe_path_component()` in paths.rs
- Security: Null byte injection prevention in plugin IDs and file paths
- Security: Git branch name validation to prevent injection attacks
- Security: Remove internal error details from HTTP responses
- Fix: Kill orphan daemon processes on startup (prevents port conflicts after crash)
- Fix: Git operations have 120s timeout (prevents hanging on network issues)
- Fix: Graceful daemon shutdown with SIGTERM before SIGKILL (2s timeout)
- Fix: Daemon startup error visibility (captures stderr, reports immediate exits)
- Fix: Return proper HTTP error codes from install_plugin endpoint
- Fix: Handle uppercase 'V' prefix in version parsing
- Fix: Server errors now logged instead of silently swallowed
- Fix: Use `std::env::temp_dir()` instead of hardcoded `/tmp` for updates
- Fix: Cover image size limit (5MB) to prevent memory exhaustion
- Refactor: Proper TOML parsing for plugin name extraction (was naive string split)
- Refactor: Compile-time assertion for icon data size
- Refactor: Removed `.expect()` panics in github.rs path functions (now return Option)
- Refactor: Consolidated duplicate `is_safe_id` functions into shared `paths::is_safe_path_component`
- Refactor: Simplified restart_with_cleanup (daemon cleanup happens via Drop)
- Refactor: Use `&Path` instead of `&PathBuf` in function signatures
- Tests: Consolidated table-driven tests (hotkeys 10→4, version 4→3)
- Tests: Added manifest parsing tests (MenuItem, ActionType, full/minimal manifest)
- Tests: Added HTML body tag parsing tests with quote handling
- Tests: Added branch name validation tests
- Tests: ~60 new edge cases across hotkeys, version, github, paths, installer modules
- Cross-platform CI via GitHub Actions (Linux, Windows, macOS)
- Plugin platform filtering - `platforms = ["linux"]` in plugin.toml
- Developer tab with plugin linking/unlinking for dev workflow
- Auto-update with notification dot and one-click install

### What Works
- System tray icon (SNI protocol)
- Browser-based plugin store
- Plugin install/uninstall with one click
- Hotkey configuration per plugin action
- Dual-location config system (survives uninstall/reinstall)
- Daemon plugin support
- Developer mode (`make dev`) with unified plugin management and background discovery
- Auto-update with notification dot and one-click install
- Local releases via `make release`

### Architecture
- `src/tray/` - System tray abstraction
  - `platform/linux.rs` - GTK event loop in separate thread
  - `platform/macos.rs` - NSApplication.run() on main thread (objc2)
  - `platform/windows.rs` - Condvar-based blocking
- `src/daemon/` - Background task orchestration and event bus
- `src/dev/` - Developer-specific discovery and linking logic
- `src/plugins/` - Plugin loading, config management
- `src/features/plugin_store/` - Browser UI server
- `src/hotkeys/` - Global hotkey registration
- `src/updates/` - Auto-update system
- `src/paths.rs` - Shared paths and `open_url()` via `open` crate
- `src/version.rs` - Semantic version parsing and comparison
- `ui/` - Embedded web UI (rust-embed)

### Developer Mode

`make dev` runs with Developer tab enabled:
- Unified plugin list showing: Linked (green), Installed (blue), Local Clone (yellow)
- Link local plugins via symlinks (backs up existing store installs)
- Unlink restores original installed version from backup
- Circular refresh button in section header for rescanning local plugins
- Reload card with spinning button to restart all daemons
- Keyboard: ↑/↓ navigate, Space/Enter link/unlink, `r` rescan, `Ctrl+r` reload

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
