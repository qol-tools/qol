<div align="center">

# 🧰 QoL Tray

**A pluggable system tray daemon that doesn't suck.**

*One tray icon. Infinite possibilities.*

[Install](#installation) · [Browse Plugins](#plugin-store) · [Build Your Own](#creating-plugins)

</div>

---

## What is this?

QoL Tray is a single system tray daemon that hosts plugins. Instead of 15 different apps cluttering your tray, you get one clean icon with a menu that does everything.

```
┌─────────────────────────┐
│ 📱 PointZ               │ → Control PC from phone
│ 📋 Clipboard History    │ → Never lose a paste again
│ ⏱️  Pomodoro            │ → Focus timer
│ ───────────────────     │
│ 🔌 Plugin Store         │ → Browse & install more
│ ───────────────────     │
│ Quit                    │
└─────────────────────────┘
```

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

Click **Plugin Store → Browse Plugins** in the tray menu. Install plugins with one click.

Official plugins live at [github.com/qol-tools](https://github.com/qol-tools) (repos prefixed with `plugin-`).

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
| `action` | Executes `run.sh` when clicked |
| `checkbox` | Toggles a boolean in `config.json` |
| `separator` | Visual divider |
| `submenu` | Nested menu |

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
http://localhost:PORT/plugins/my-plugin/
```

Accessible via **Settings** in the plugin's menu.

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
