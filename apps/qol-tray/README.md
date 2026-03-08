# QoL Tray

A pluggable system tray daemon for macOS and Linux. One tray icon, a browser-based UI, and a growing set of plugins that extend your desktop.

## What it does

QoL Tray runs as a tray icon and serves a local web UI at `http://127.0.0.1:42700`. From there you can install plugins, configure hotkeys, run tasks, and manage everything from one place.

Plugins are standalone Rust binaries that qol-tray discovers, installs, and manages. The built-in plugin store pulls directly from the [qol-tools](https://github.com/qol-tools) GitHub org.

### Available plugins

| Plugin | Description |
|--------|-------------|
| plugin-launcher | Application and file launcher with fuzzy search |
| plugin-alt-tab | Window switcher with live previews |
| plugin-keyremap | Keyboard and mouse remapping |
| plugin-window-actions | Window minimize, restore, and monitor movement |
| plugin-os-themes | OS theme detection and cursor effects |
| plugin-screen-recorder | Screen recording |
| plugin-ide-checkout | IDE branch checkout integration |
| plugin-pointz | Pointer utilities |

### UI features

| Feature | Description |
|---------|-------------|
| Plugin management | Install, update, configure, and remove plugins |
| Hotkeys | Bind global keyboard shortcuts to plugin actions |
| Task runner | Define and run custom shell tasks |
| Command palette | Fuzzy search across all available actions (wasm-powered) |
| Dev tools | Build overlay, recompilation tracking, dev links for local plugin repos |

## Installation

```bash
git clone https://github.com/qol-tools/qol-tray
cd qol-tray
make install
```

This builds in release mode and runs the installer, which sets up the tray daemon, autostart entry, and desktop integration.

### Binaries

| Binary | Purpose |
|--------|---------|
| `qol-tray` | Main daemon |
| `qol-tray-install` | Installer and updater |
| `qol-tray-doctor` | Diagnostics and troubleshooting |

## Platform support

| Platform | Status |
|----------|--------|
| macOS | Supported |
| Linux (X11) | Supported |
| Linux (Wayland) | Partial (tray icon works, some plugins need X11 for window capture) |

## Architecture

qol-tray is built on a set of shared libraries:

| Crate | Purpose |
|-------|---------|
| [qol-plugin-api](https://github.com/qol-tools/qol-plugin-api) | Plugin host API (daemon, keepalive, window management) |
| [qol-search](https://github.com/qol-tools/qol-search) | Fuzzy search algorithm (native + wasm) |
| [qol-frecency](https://github.com/qol-tools/qol-frecency) | Frequency/recency ranking |
| [qol-config](https://github.com/qol-tools/qol-config) | Config discovery and loading |
| [qol-color](https://github.com/qol-tools/qol-color) | Hex color parsing |
| [qol-platform](https://github.com/qol-tools/qol-platform) | Platform detection and capabilities |
| [qol-runtime](https://github.com/qol-tools/qol-runtime) | IPC protocol and shared types |
| [qol-wasm](https://github.com/qol-tools/qol-wasm) | WebAssembly bridge for qol-search |

## License

MIT
