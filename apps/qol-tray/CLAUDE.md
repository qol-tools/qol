# qol-tray

## IMPORTANT: Linux Only (For Now)

Cross-platform support is planned for the future, but **not now**. Do NOT implement macOS or Windows code until explicitly asked. Do NOT add cross-platform CI workflows or abstractions. Focus only on Linux. When the user asks for cross-platform support, then implement it.

## Development Commands

```bash
make run      # Build and run
make dev      # Build and run with dev features (Developer tab)
make test     # Run tests
make install  # Build release and install to /usr/bin
make clean    # Clean build artifacts
make deb      # Build .deb package
make release  # Bump version, build, push, create GitHub release
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
  - `mod.rs`: Contains shared Windows/macOS implementation via `#[cfg(not(target_os = "linux"))]`
- `PlatformTray` enum handles platform differences at compile time
- `icon.rs`: Icon loading from embedded RGBA data, supports notification dot variant
- Uses `tray-icon` crate (cross-platform)

**src/features/plugin_store/** - Browser-based plugin management
- Serves web UI at `http://127.0.0.1:42700`
- Landing page shows installed plugins and plugin store
- Plugin settings accessed via `/plugins/{plugin_id}/`
- API endpoints for install/uninstall operations
- Fetches available plugins from `github.com/qol-tools/*`

**src/updates/** - Auto-update system
- Checks GitHub API on startup for new releases (2s timeout)
- Compares semantic versions
- Shows orange notification dot on tray icon when update available
- Menu item "⬆ Update to vX.Y.Z" downloads .deb and installs via `pkexec dpkg -i`
- Kills plugin daemons before restart to avoid socket conflicts

### Plugin Manifest Format

Plugins define their menu structure in `plugin.toml`:

```toml
[plugin]
name = "Plugin Name"
description = "Description"
version = "1.0.0"
platforms = ["linux"]  # Optional - omit for all platforms

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

Platform-specific code belongs in `platform/` directories, not root modules.

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
