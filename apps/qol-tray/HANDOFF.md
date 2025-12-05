# Session Handoff

## Latest Session: Dual-Location Plugin Config System

### What Was Implemented
Implemented a dual-location config system to preserve plugin configs across uninstall/reinstall cycles while keeping the filesystem clean and organized.

**Architecture:**
- **Primary location**: `~/.config/qol-tray/plugins/{plugin-id}/config.json`
  - Clean, organized, easy to browse/edit manually
  - Lives alongside plugin files
- **Backup location**: `~/.config/qol-tray/plugin-configs.json`
  - Single file containing all plugin configs
  - Survives plugin uninstall
  - Easy to backup entire system

**Behavior:**
- **Auto-sync on write**: Every config save writes to both locations
- **Auto-restore on read**: If plugin dir config is missing, restores from backup
- **Transparent**: Zero user intervention needed
- **Hassle-free**: Works automatically for all plugins

### Key Files
- `src/plugins/config.rs` - PluginConfigManager with dual-location logic
- `src/plugins/mod.rs` - Export PluginConfigManager
- `src/features/plugin_store/server.rs` - Updated API endpoints to use manager
- `Cargo.toml` - Added tempfile dev dependency for tests

### Testing
- 10 unit tests with AAA pattern (Arrange-Act-Assert)
- Tests cover: path construction, load/save, restore from backup
- All tests passing, no warnings
- Manual test: Screen recorder config syncs correctly to both locations

### User Request Context
User wanted to experiment with uninstalling/reinstalling plugins but was concerned about losing:
1. Hotkeys (already safe - stored in `hotkeys.json`)
2. Plugin configs (now safe with dual-location system)

Solution provides best of both worlds:
- Clean filesystem for normal browsing
- Safety net for uninstall/reinstall scenarios
- Easy full-system backup (one file)

---

## Previous Session Summary

### 1. README Update
Updated README.md to reflect current architecture.

### 2. Created plugin-window-actions
New plugin for window management with 9 actions.
Repo: https://github.com/qol-tools/plugin-window-actions

### 3. Plugin Manifest Fetching Fix
`github.rs` now tries both `main` and `master` branches.

### 4. Hotkey Execution Fix
`src/hotkeys/mod.rs` now passes action ID as first argument to `run.sh`.

### 5. Hotkey Modal UX Improvements
Major refactor of `ui/views/hotkeys.js`.

## Notes
- Window actions use `xdotool`, `wmctrl`, `xrandr`, `xprop` — X11 only
- Cinnamon uses Muffin as its window manager (fork of Mutter)
- The issue is specifically with Cinnamon's tiling/snapping feature
