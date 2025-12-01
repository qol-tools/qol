# Session Handoff

## What Was Done This Session

### 1. Early Returns Refactoring
Applied early return pattern across the codebase:

- `server.rs`: `install_plugin`, `uninstall_plugin`, `list_installed`, `serve_cover`, `set_github_token`, `delete_github_token`
- `plugin_ui.rs`: `serve_file`
- `linux.rs`: `setup_event_loop` match handling

### 2. Dead Code Removal
Removed unused code to eliminate warnings:

- `is_cache_valid()` — replaced by `get_valid_cache()`
- `has_token()` — unused method on GitHubClient
- `reload_plugins()` — part of removed refresh system
- `request_plugin_refresh()` — entire refresh wiring removed (tray menu doesn't show plugins)
- Prefixed `_plugin_manager` in `build_menu` (intentionally unused parameter)

### 3. Daemon Lifetime Bug Fix
**Critical fix:** Plugin daemons were stopping immediately after starting.

**Root cause:** `plugin_manager` Arc was passed to `build_menu()` where it was unused (`_plugin_manager`). Since `main.rs` didn't keep a reference, the PluginManager was dropped → all Plugins dropped → `stop_daemon()` called.

**Fix:** Clone the Arc before passing, keep reference alive until shutdown:
```rust
let _tray = TrayManager::new(plugin_manager.clone(), ...);
// ...
drop(plugin_manager); // explicit drop at shutdown
```

### 4. plugin-pointz QR Code Fix
Fixed QR code not displaying in PointZ plugin UI:
- CDN script wasn't loading (jsdelivr issue)
- Changed to unpkg CDN: `https://unpkg.com/qrcode/build/qrcode.min.js`
- Pushed to `qol-tools/plugin-pointz` repo

## Current State

App compiles with no warnings. Plugin daemons start and stay running. PointZ plugin UI shows QR code.

## What's Next

1. **Hotkeys view** — Implement hotkey configuration (currently placeholder)
2. **Plugin actions** — Execute plugin actions (run.sh, toggle-config) via browser API
3. **Cover images** — Done (user added)
4. **GitHub token flow** — Done (tested, works)

## Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point, keeps plugin_manager alive |
| `src/plugins/mod.rs` | Plugin struct with daemon lifecycle |
| `src/plugins/manager.rs` | PluginManager, loads and stores plugins |
| `src/tray/platform/linux.rs` | GTK tray, event loop |
| `src/features/plugin_store/server.rs` | API endpoints |
| `ui/` | Browser UI (plugins, store, hotkeys views) |

## Storage Paths

| File | Purpose |
|------|---------|
| `~/.config/qol-tray/.github-token` | GitHub personal access token |
| `~/.config/qol-tray/.plugin-cache.json` | Cached plugin list from GitHub |
| `~/.config/qol-tray/plugins/` | Installed plugins directory |

## API Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/plugins` | GET | List available plugins (uses cache) |
| `/api/plugins?refresh=true` | GET | Force refresh from GitHub |
| `/api/installed` | GET | List installed plugins |
| `/api/install/:id` | POST | Install plugin |
| `/api/uninstall/:id` | POST | Uninstall plugin |
| `/api/github-token` | GET/POST/DELETE | Token management |
| `/api/cover/:id` | GET | Plugin cover image |
| `/plugins/:id/` | GET | Serve plugin UI |

## Plugin Repos

| Repo | Purpose |
|------|---------|
| `qol-tools/plugin-pointz` | PointZ remote control plugin |
| `qol-tools/pointZ` | PointZ app (Flutter + Rust server) |

## User Preferences

- Keyboard-first UI (single-letter shortcuts like `d`, `r`)
- Functional/declarative code patterns
- No comments in code
- No builds/tests unless explicitly asked
- Atomic commits with conventional prefixes
- Flatten nested conditionals, use early returns
- Direct communication, no fluff
