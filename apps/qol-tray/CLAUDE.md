# CLAUDE.md

## Development Commands

```bash
make run      # Build and run
make dev      # Build and run with dev features (Developer tab)
make test     # Run tests
make install  # Build release and install to /usr/bin
make clean    # Clean build artifacts
```

## Architecture

**Minimal tray menu:** The tray menu only has "Plugins" (opens browser UI) and "Quit". All plugin interaction happens in the browser.

### Core Modules

**src/plugins/** - Plugin loading, execution, and configuration
- Scans `~/.config/qol-tray/plugins/` for plugin directories
- Each plugin has: `plugin.toml` (manifest), `run.sh` (executable), optional `config.json`
- Supports daemon processes and config toggles
- Key types: `Plugin`, `PluginManager`, `PluginManifest`
- Files: `mod.rs` (Plugin struct), `manager.rs` (PluginManager), `loader.rs` (scan/load), `manifest.rs` (data structures)

**src/menu/** - Menu abstraction and event routing
- `builder.rs`: Builds minimal menu (features + Quit), no per-plugin items
- `router.rs`: EventRouter with EventPattern (Exact/Prefix) for O(k) routing
- EventHandler supports Sync/Async execution
- Event format: `feature-id::menu-item-id`

**src/tray/** - System tray UI with platform abstraction
- Platform-specific implementations in `platform/` subdirectory
  - `linux.rs`: GTK-based, spawns separate thread for event loop
  - `windows.rs`, `macos.rs`: Standard tray icon implementation
- `PlatformTray` enum handles platform differences at compile time
- `icon.rs`: Icon loading from embedded RGBA data

**src/features/plugin_store/** - Browser-based plugin management
- Serves web UI at `http://127.0.0.1:42700`
- Landing page shows installed plugins and plugin store
- Plugin settings accessed via `/plugins/{plugin_id}/`
- API endpoints for install/uninstall operations
- Fetches available plugins from `github.com/qol-tools/*`

### Plugin Manifest Format

Plugins define their menu structure in `plugin.toml`:

```toml
[plugin]
name = "Plugin Name"
description = "Description"
version = "1.0.0"

[menu]
label = "Menu Label"
items = [
    { type = "action", id = "run", label = "Run", action = "run" },
    { type = "checkbox", id = "toggle", label = "Enable", checked = true,
      action = "toggle-config", config_key = "enabled" },
    { type = "separator" },
    { type = "submenu", id = "sub", label = "More", items = [...] }
]

[daemon]  # Optional
enabled = true
command = "daemon.sh"
restart_on_crash = false
```

Action types:
- `run` - Execute `run.sh`
- `toggle-config` - Toggle boolean in `config.json` at `config_key` path
- `settings` - Reserved for future use

### Code Style

- **No comments** - Code removed all comments; keep it that way
- **Conventional commits** - Use format: `feat:`, `fix:`, `refactor:`, `test:`, etc.
- **Short commit messages** - One-liners, no fluff, no co-authors
- **Atomic commits** - One logical change per commit. Split distinct changes (bug fix, refactor, tests) into separate commits. Each commit must compile and represent a working state.
- **No dead code warnings** - Remove unused code or gate with feature flags
- Platform-specific code belongs in `platform/` directories, not root modules
- **No builds or tests unless asked** - Do not run `cargo build`, `cargo run`, `make`, or browser tests unless explicitly requested. These operations are expensive.

### Frontend Architecture

- **Functional and declarative** - Pure render functions, no imperative DOM manipulation
- **Data-driven** - UI derived from state, not manually synchronized
- **Single responsibility** - Split logical chunks into focused modules
- **Type safety** - Define data structures explicitly, validate API responses
- **Scalability** - Design for N plugins, not hardcoded assumptions
- **Keyboard-first** - All interactions MUST be accessible via keyboard. This is critical. Design keyboard flow first, then add mouse/hover as secondary. Use single-letter shortcuts (e.g., `d` for delete) since Mac lacks Delete key. Always show keyboard hints in UI.

### Complexity Thresholds

- **Max 50 lines per function** - Split beyond this
- **Max 2 levels of nesting** - Extract inner logic into helpers
- **One concern per function** - Don't mix state management, navigation, and action dispatch
- **Sequential ifs checking selectors** → Use config array with `{ selector, handler }` objects
- **Conditional rendering with shared structure** → Extract state-specific render functions
- **Key event handlers** → Separate recording/navigation/actions, use declarative handler maps

### Test Style

- **AAA Pattern** - All tests follow Arrange-Act-Assert pattern with explicit comments
- **Arrange** - Set up test data and dependencies
- **Act** - Invoke the system under test
- **Assert** - Verify expected behavior
- **Never mix** - Keep each section separate, no Arrange+Act or Act+Assert mixing
- **Descriptive names** - Use full snake_case descriptions that explain what is being tested

## Icon Management

Icon is embedded as raw RGBA data at compile time from `assets/icon.rgba` (64x64 pixels, generated from `icon.png`).

To update icon:
1. Edit `assets/icon.png`
2. Convert to RGBA: `python3 -c "from PIL import Image; img = Image.open('assets/icon.png'); open('assets/icon.rgba', 'wb').write(img.tobytes())"`
3. Rebuild

## Plugin Development

Plugins are external to this codebase. They live in `~/.config/qol-tray/plugins/`.

The daemon provides:
- Plugin loading and manifest parsing
- Browser-based settings UI (each plugin can have `ui/index.html`)
- Config file management (read/write JSON)
- Process execution (scripts and daemons)

Plugins handle their own logic via shell scripts.
