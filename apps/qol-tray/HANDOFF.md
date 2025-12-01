# Session Handoff

## What Was Done

### 1. Sidebar Navigation Architecture
Replaced separate `index.html` + `store.html` with unified single-page layout:

**Structure:**
- `ui/index.html` — Shell with sidebar + content container
- `ui/main.js` — Router with Tab/Shift+Tab navigation between views
- `ui/components/sidebar.js` — Pure render function for sidebar
- `ui/views/plugins.js` — Plugin grid view
- `ui/views/store.js` — Store view (install plugins)
- `ui/views/hotkeys.js` — Placeholder for future hotkey config

**Navigation:**
- Tab — cycle forward through views (Plugins → Store → Hotkeys)
- Shift+Tab — cycle backward
- Arrow keys — navigate within active view
- Enter — activate selection

### 2. Plugin Uninstall on Plugins Page
- `d` key — shows confirmation modal to uninstall selected plugin
- Cogwheel button on hover — opens context menu with "Delete" option
- Confirmation modal with Cancel (Esc) / Delete (Enter)

### 3. Store View Improvements
- Installed plugins greyed out with "Installed" badge (no uninstall button)
- Plugins sorted alphabetically
- Rate limit banner with GitHub token input when API fails

### 4. GitHub Token Storage
- Token stored in `~/.config/qol-tray/.github-token`
- API endpoints: `GET/POST/DELETE /api/github-token`
- Token automatically used in GitHub API requests when present
- UI shows banner to add token when rate limited

### 5. Improved GitHub API Error Handling
- Now shows actual HTTP status and response body instead of generic decode error
- Helps diagnose rate limits, auth failures, etc.

### 6. Updated CLAUDE.md
- Added keyboard-first principle with Mac considerations (use `d` instead of Delete key)
- Added rule to always show keyboard hints in UI

## Current State

App compiles. All frontend features work. GitHub token flow implemented but needs testing with valid `ghp_...` classic token.

**Known warnings:**
- `plugin_manager` unused in `build_menu` function signature
- `has_token` method unused in GitHubClient

## What's Next

1. **Test GitHub token flow** — User needs to create classic token at https://github.com/settings/tokens/new (no scopes needed)
2. **Cover images** — Plugins need `cover.png` files (320×180 recommended)
3. **Hotkeys** — Implement hotkey configuration view
4. **Actions from browser** — Execute plugin actions (run.sh, toggle-config) via API

## Key Files

| File | Purpose |
|------|---------|
| `ui/main.js` | Router, view switching, keyboard handling |
| `ui/views/plugins.js` | Plugin grid with uninstall flow |
| `ui/views/store.js` | Store with install + token banner |
| `ui/style.css` | All styles including modal, banner, context menu |
| `src/features/plugin_store/github.rs` | GitHub API client + token storage |
| `src/features/plugin_store/server.rs` | API endpoints including token management |
| `CLAUDE.md` | Code style and architecture rules |

## Token Storage

GitHub token is stored at: `~/.config/qol-tray/.github-token`

To manually set: `echo "ghp_xxxxx" > ~/.config/qol-tray/.github-token`
To delete: `rm ~/.config/qol-tray/.github-token`

## User Preferences

- Keyboard-first UI design (use single-letter shortcuts like `d` for Mac compatibility)
- Functional/declarative code
- No comments in code
- No builds/tests unless explicitly asked
- Atomic commits representing working states
- Direct communication, no fluff
