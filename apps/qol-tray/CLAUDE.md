# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Development Commands

```bash
make build          # Debug build
make release        # Release build
make test           # Run all tests
make run            # Build and run with info logging
make run-debug      # Build and run with debug logging
make install        # Install to /usr/bin and setup autostart
make clean          # Clean build artifacts
```

Build with optional plugin-store feature:
```bash
cargo build --features plugin-store
```

## Architecture

**Data-driven structure:** Core infrastructure consumes plugin manifest data. No feature registration system.

### Core Modules

**src/plugins/** - Plugin loading, execution, and configuration
- Scans `~/.config/qol-tray/plugins/` for plugin directories
- Each plugin has: `plugin.toml` (manifest), `run.sh` (executable), optional `config.json`
- Supports daemon processes and config toggles
- Key types: `Plugin`, `PluginManager`, `PluginManifest`
- Files: `mod.rs` (Plugin struct), `manager.rs` (PluginManager), `loader.rs` (scan/load), `manifest.rs` (data structures)

**src/menu/** - Menu abstraction and event routing
- `builder.rs`: Builds menu from PluginManager, renders all items generically
- `router.rs`: EventRouter with EventPattern (Exact/Prefix) for O(k) routing
- EventHandler supports Sync/Async execution
- Core items (Quit) hardcoded in builder, plugins loaded from manifests
- Event format: `plugin-id::menu-item-id`

**src/tray/** - System tray UI with platform abstraction
- Platform-specific implementations in `platform/` subdirectory
  - `linux.rs`: GTK-based, spawns separate thread for event loop
  - `windows.rs`, `macos.rs`: Standard tray icon implementation
- `PlatformTray` enum handles platform differences at compile time
- `icon.rs`: Icon loading from embedded RGBA data

**plugin-store** (optional feature) - GitHub plugin browser/installer
- Only compiled with `--features plugin-store` flag
- Fetches plugins from `github.com/qol-tools/*` organization
- Looks for repos prefixed with `plugin-`
- Uses git clone/pull for install/update operations
- Not core functionality - manual plugin installation works without it

### Platform Abstraction Pattern

Platform-specific code lives in `platform/` subdirectories with conditional compilation:

```rust
#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
mod windows;

pub enum PlatformTray {
    #[cfg(target_os = "linux")]
    Linux,
    #[cfg(not(target_os = "linux"))]
    Standard(TrayIcon),
}
```

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
- **Conventional commits** - Use format: `feat:`, `fix:`, `refactor:`, etc.
- **Short commit messages** - One-liners, no fluff, no co-authors
- **No dead code warnings** - Remove unused code or gate with feature flags
- Platform-specific code belongs in `platform/` directories, not root modules

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

Plugins are external to this codebase. They live in `~/.config/qol-tray/plugins/`. See example at `examples/plugins/screen-recorder/`.

The daemon only provides:
- Plugin loading and manifest parsing
- Tray menu generation
- Config file management (read/write JSON)
- Process execution (scripts and daemons)

Plugins handle their own logic via shell scripts.
