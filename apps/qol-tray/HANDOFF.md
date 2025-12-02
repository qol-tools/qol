# Session Handoff

## What Was Done This Session

### 1. Plugin Update Feature
Added ability to detect and update plugins with newer versions:

- `server.rs`: Added `available_version`, `update_available` fields to `InstalledPlugin`
- `server.rs`: Added `is_newer_version()` function with semver comparison
- `server.rs`: Added `/api/update/:id` endpoint using `git pull`
- `plugins.js`: Update button on plugin cards, `u` keyboard shortcut
- `style.css`: Green update button styling, `.has-update` border highlight
- 10 unit tests for `is_newer_version()`

### 2. Version Comparison Fix
Fixed false positives for update detection:

- **Problem:** Cache stored dependency binary version (e.g., `0.4.4` from pointZ releases)
- **Actual:** Installed plugin had its own manifest version (e.g., `0.4.0`)
- **Fix:** Removed `resolve_version()` from `github.rs`, now uses plugin manifest version only

### 3. Plugin-PointZ QR Code Fix (Again)
The unpkg CDN path `build/qrcode.min.js` doesn't exist in qrcode@1.5.4:

- Changed to ESM import: `import QRCode from 'https://esm.sh/qrcode@1.5.4'`
- Made `app.js` a module, centered QR code and download link
- Pushed to `qol-tools/plugin-pointz`

### 4. PointZ Release Workflow Fix
Flutter APK build was missing from releases:

- Restored `build-apk` job to `.github/workflows/release.yml`
- Added `needs: [build, build-apk]` to release job
- Created tag `v0.4.4` to trigger new release with APK

### 5. Plugin Config API
Added endpoints for reading/writing plugin config files:

- `GET /api/plugins/:id/config` — Read `config.json`
- `PUT /api/plugins/:id/config` — Write `config.json`

### 6. Screen Recorder Plugin UI
Created settings UI for `plugin-screen-recorder`:

- `ui/index.html`, `ui/style.css`, `ui/app.js`
- Audio settings (enable, mic/system inputs, devices)
- Video settings (framerate, CRF, preset, format)
- Pushed to `qol-tools/plugin-screen-recorder`

### 7. Global Hotkey System
Added global hotkey support for triggering plugin actions:

- New `src/hotkeys/mod.rs` module
- Uses `global-hotkey` crate (v0.7)
- `HotkeyManager` loads config, registers hotkeys, routes events
- `parse_hotkey()` parses strings like `Ctrl+Shift+R`
- Executes plugin `run.sh` on hotkey trigger

## Current State

App compiles with no warnings. Plugin updates work correctly. Global hotkeys backbone is in place.

## What's Next

1. **Hotkeys UI** — Implement `ui/views/hotkeys.js` for configuring hotkeys in browser
2. **Default bindings** — Create default `hotkeys.json` with screen recorder binding
3. **Hotkey recording** — Capture keypress to set hotkey (instead of typing string)

## Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point, starts hotkey listener |
| `src/hotkeys/mod.rs` | HotkeyManager, config loading, event handling |
| `src/features/plugin_store/server.rs` | API endpoints including config |
| `ui/views/plugins.js` | Plugin grid with update buttons |
| `ui/views/hotkeys.js` | Placeholder for hotkey configuration |

## Storage Paths

| File | Purpose |
|------|---------|
| `~/.config/qol-tray/.github-token` | GitHub personal access token |
| `~/.config/qol-tray/.plugin-cache.json` | Cached plugin list from GitHub |
| `~/.config/qol-tray/hotkeys.json` | Global hotkey bindings |
| `~/.config/qol-tray/plugins/` | Installed plugins directory |
| `~/.config/qol-tray/plugins/:id/config.json` | Per-plugin configuration |

## API Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/plugins` | GET | List available plugins (uses cache) |
| `/api/plugins?refresh=true` | GET | Force refresh from GitHub |
| `/api/installed` | GET | List installed plugins (with update info) |
| `/api/install/:id` | POST | Install plugin |
| `/api/update/:id` | POST | Update plugin (git pull) |
| `/api/uninstall/:id` | POST | Uninstall plugin |
| `/api/plugins/:id/config` | GET/PUT | Read/write plugin config |
| `/api/github-token` | GET/POST/DELETE | Token management |
| `/api/cover/:id` | GET | Plugin cover image |
| `/plugins/:id/` | GET | Serve plugin UI |

## Hotkey Config Format

```json
{
  "hotkeys": [
    {
      "id": "record",
      "key": "Ctrl+Shift+R",
      "plugin_id": "plugin-screen-recorder",
      "action": "run",
      "enabled": true
    }
  ]
}
```

Supported modifiers: `Ctrl`, `Alt`, `Shift`, `Super`/`Win`/`Meta`/`Cmd`
Supported keys: A-Z, 0-9, F1-F12, Space, Enter, Escape, arrows, etc.

## Plugin Repos

| Repo | Purpose |
|------|---------|
| `qol-tools/plugin-pointz` | PointZ remote control plugin |
| `qol-tools/plugin-screen-recorder` | Screen recording plugin |
| `qol-tools/pointZ` | PointZ app (Flutter + Rust server) |

## User Preferences

- Keyboard-first UI (single-letter shortcuts like `d`, `r`, `u`)
- Functional/declarative code patterns
- No comments in code
- No builds/tests unless explicitly asked
- Atomic commits with conventional prefixes
- Flatten nested conditionals, use early returns
- AAA pattern for unit tests (Arrange-Act-Assert)
- Direct communication, no fluff
