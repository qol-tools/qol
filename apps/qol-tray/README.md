<div align="center">

# 🧰 QoL Tray

**A pluggable system tray daemon that doesn't suck.**

*One tray icon. Infinite possibilities.*

[Install](#installation) · [Browse Plugins](#plugin-store) · [Build Your Own](#creating-plugins)

</div>

---

## What is this?

QoL Tray is a single system tray daemon that hosts plugins. Instead of 15 different apps cluttering your tray, you get one clean icon that opens a browser-based dashboard.

```
┌─────────────────┐
│ 🔌 Plugins      │ → Opens browser UI
│ ───────────     │
│ Quit            │
└─────────────────┘
```

All plugin management happens in the browser at `http://127.0.0.1:42700`:
- View installed plugins
- Browse and install from the plugin store
- Configure plugin settings

Each plugin is just a folder with a manifest and a script. No compilation needed. Works with any language.

## Installation

```bash
git clone https://github.com/qol-tools/qol-tray
cd qol-tray
make install
```

Then just run:
```bash
qol-tray
```

## Plugin Store

Click **Plugins** in the tray menu to open the browser UI. The store tab shows available plugins from [github.com/qol-tools](https://github.com/qol-tools) (repos with `qol-tray-plugin` topic).

Install with one click. Updates are detected automatically.

Or install manually:
```bash
git clone https://github.com/qol-tools/plugin-pointz ~/.config/qol-tray/plugins/plugin-pointz
```

## Creating Plugins

A plugin is just a folder in `~/.config/qol-tray/plugins/`:

```
my-plugin/
├── plugin.toml      # Manifest (required)
├── run.sh           # What happens when you click "Run" (required)
├── config.json      # Runtime config (optional)
└── ui/              # Web UI for settings (optional)
    └── index.html
```

### Minimal plugin.toml

```toml
[plugin]
name = "My Plugin"
description = "Does cool stuff"
version = "1.0.0"

[menu]
label = "🔧 My Plugin"
items = [
    { type = "action", id = "run", label = "Run", action = "run" }
]
```

### Menu item types

| Type | Description |
|------|-------------|
| `action` | Triggers an action (see action types below) |
| `checkbox` | Toggles a boolean in `config.json` via `toggle-config` action |
| `separator` | Visual divider |
| `submenu` | Nested menu with child items |
| `submenu` | Nested menu with child items |

### Action types

| Action | Description |
|--------|-------------|
| `run` | Executes `run.sh` |
| `toggle-config` | Toggles boolean at `config_key` path in `config.json` |
| `settings` | Reserved for future use |

### Daemon plugins

For long-running background services:

```toml
[daemon]
enabled = true
command = "daemon.sh"
restart_on_crash = true
```

### Plugin UI

Drop an `index.html` in `ui/` and it becomes accessible at:
```
http://127.0.0.1:42700/plugins/my-plugin/
```

Click the plugin card in the browser UI to access settings.

## Platform Support

| Platform | Status |
|----------|--------|
| Linux (X11) | ✅ Full support |
| Linux (Wayland) | ⚠️ Tray works, some plugins may not |
| macOS | 🚧 Planned |
| Windows | 🚧 Planned |

## License

MIT

---

<div align="center">
<sub>Built with Rust. No Electron. No bloat.</sub>
</div>

