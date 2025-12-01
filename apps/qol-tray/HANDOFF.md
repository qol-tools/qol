# Session Handoff

## What Was Done

### 1. Removed plugin items from tray menu
- Tray menu now only has "Plugins" (opens browser) and "Quit"
- Removed plugin loop from `src/menu/builder.rs`
- Removed `open_plugin_ui`, `load_plugin_icon`, `load_icon_from_path` functions
- Removed unused imports: `Plugin`, `Path`, `IconMenuItem`, `Icon`

### 2. Added landing page with plugin grid
**Backend (`src/features/plugin_store/server.rs`):**
- Added `GET /api/installed` — returns installed plugins with metadata
- Added `GET /api/cover/:plugin_id` — serves plugin cover images
- New struct `InstalledPlugin { id, name, description, version, has_cover, has_ui }`

**Frontend:**
- `ui/index.html` — Netflix-style grid of plugin cards
- `ui/app.js` — keyboard navigation (←↑↓→), Enter to open, Tab to store
- `ui/style.css` — grid layout, selected state, dimmed no-ui plugins
- `ui/store.html` + `ui/store.js` — moved old plugin store here

**Features:**
- 4-column grid with 16:9 cards
- Cover images from `{plugin}/cover.png`, fallback SVG placeholder
- Click to select, double-click to open
- Plugins without `ui/index.html` are dimmed

### 3. Updated CLAUDE.md
- Changed architecture description to reflect minimal tray menu
- Added "No builds or tests unless asked" rule
- Added "Atomic commits" rule
- Removed redundant platform abstraction code example
- Added Frontend Architecture section

## Current State

The app compiles and runs. Landing page works with keyboard navigation.

**Known warnings:**
- `plugin_manager` unused in `build_menu` function signature (kept for API compatibility)

## What's Next (User's Vision)

1. **Hotkeys** — User wants to assign hotkeys to plugin actions from the browser UI
2. **Actions from browser** — Execute plugin actions (run.sh, toggle-config) via API
3. **Cover images** — Plugins need to provide `cover.png` files

## Key Files

| File | Purpose |
|------|---------|
| `src/menu/builder.rs` | Builds tray menu (now minimal) |
| `src/features/plugin_store/server.rs` | Web server, API endpoints |
| `ui/index.html` | Landing page |
| `ui/app.js` | Landing page logic + keyboard nav |
| `ui/store.html` | Plugin store (install/uninstall) |
| `CLAUDE.md` | Code style and architecture rules |

## User Preferences

- Keyboard-first UI design
- Functional/declarative code
- No comments in code
- No builds/tests unless explicitly asked (expensive)
- Atomic commits representing working states
- Direct communication, no fluff

