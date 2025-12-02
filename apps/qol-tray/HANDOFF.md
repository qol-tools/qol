# Session Handoff

## What Was Done This Session

### 1. Hotkey Module Code Review & Fixes
Fixed issues in `src/hotkeys/mod.rs`:

- **Bug fix**: `unregister_all()` was passing empty slice — now tracks registered hotkeys in `Vec<HotKey>` field
- **Removed dead code**: Removed `#[allow(dead_code)]` from `save_config` by wiring it to API
- **Refactored**: Replaced 70-line match statement with `static KEY_CODE_MAP: Lazy<HashMap>`
- **Derived Default**: Replaced manual `Default` impl with `#[derive(Default)]`

### 2. Hotkey Unit Tests
Added 14 tests in `src/hotkeys/mod.rs`:

- `parse_key_code_*` — letter, digit, function, special, navigation keys
- `parse_hotkey_*` — single key, modifiers, whitespace, case insensitivity

### 3. Hotkeys API Endpoints
Added to `server.rs`:

- `GET /api/hotkeys` — Read hotkey config
- `PUT /api/hotkeys` — Save hotkey config

### 4. Git Tag Versioning for Plugins
Changed plugin version detection to use git tags instead of `plugin.toml`:

- Added `fetch_latest_tag()` in `github.rs`
- `build_plugin_metadata()` now prefers tag version, falls back to manifest
- Update detection now works when you push new tags (no need to bump `plugin.toml`)

### 5. Hotkeys UI Implementation
Full implementation in `ui/views/hotkeys.js`:

- List view with shortcut, plugin, action, status columns
- Keyboard navigation: ↑↓ navigate, Enter edit, `a` add, `d` delete, Space toggle
- Edit modal with key recording (captures Ctrl/Alt/Shift/Super + key)
- Persistence via `/api/hotkeys`

### 6. Modal Keyboard Isolation Fix
Fixed bugs in hotkeys UI:

- Added `isBlocking()` export checked by `main.js` before Tab handling
- Tab no longer switches views when modal is open
- Key recording ignores Tab/Escape, properly handles cancel

### 7. CLAUDE.md Update
Clarified atomic commit guidelines:
> One logical change per commit. Split distinct changes (bug fix, refactor, tests) into separate commits.

## Current State

App compiles with no warnings. All features working:
- Plugin updates via git tags ✓
- Hotkey backend (registration, execution) ✓
- Hotkey UI (add, edit, delete, toggle, key recording) ✓

## Commits This Session

```
74a51c2 fix(ui): block Tab navigation when modal open, fix key recording
67cc538 style(ui): add hotkeys view styles
22b8707 feat(ui): implement hotkeys configuration view
0e1bcce feat(github): use git tags for plugin version detection
5db36c7 feat(api): add hotkeys config endpoints
2c32875 docs: clarify atomic commit guidelines
7cf9691 test(hotkeys): add unit tests for parsing functions
10c0806 refactor(hotkeys): use static map for key code parsing
9ae5952 refactor(hotkeys): remove dead_code annotation from save_config
e9ca57c fix(hotkeys): track registered hotkeys for proper unregister
```

## What's Next

1. **Default hotkey bindings** — Create default `hotkeys.json` with screen recorder binding
2. **Hotkey reload** — Backend should reload hotkeys when config changes (currently requires restart)
3. **Screen recorder plugin version** — Bump version and push tag to test update flow

## Key Files

| File | Purpose |
|------|---------|
| `src/main.rs` | Entry point, starts hotkey listener |
| `src/hotkeys/mod.rs` | HotkeyManager, config loading, event handling, 14 tests |
| `src/features/plugin_store/server.rs` | API endpoints including hotkeys |
| `src/features/plugin_store/github.rs` | GitHub API, tag-based versioning |
| `ui/main.js` | View routing, keyboard handling with `isBlocking()` check |
| `ui/views/hotkeys.js` | Hotkey configuration UI |
| `ui/views/plugins.js` | Plugin grid with update buttons |

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
| `/api/hotkeys` | GET/PUT | Read/write hotkey config |
| `/api/github-token` | GET/POST/DELETE | Token management |
| `/api/cover/:id` | GET | Plugin cover image |
| `/plugins/:id/` | GET | Serve plugin UI |

## Storage Paths

| File | Purpose |
|------|---------|
| `~/.config/qol-tray/.github-token` | GitHub personal access token |
| `~/.config/qol-tray/.plugin-cache.json` | Cached plugin list from GitHub |
| `~/.config/qol-tray/hotkeys.json` | Global hotkey bindings |
| `~/.config/qol-tray/plugins/` | Installed plugins directory |
| `~/.config/qol-tray/plugins/:id/config.json` | Per-plugin configuration |

## Hotkey Config Format

```json
{
  "hotkeys": [
    {
      "id": "hk-1234567890",
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
- Atomic commits with conventional prefixes (one logical change per commit)
- Flatten nested conditionals, use early returns
- AAA pattern for unit tests (Arrange-Act-Assert)
- Direct communication, no fluff
