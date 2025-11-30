# QoL Tray

A system tray daemon for managing utility scripts on Linux.

## Installation

```bash
git clone <repository-url>
cd qol-tray
make install
```

Start the daemon:
```bash
qol-tray
```

## Adding Plugins

Plugins are stored in `~/.config/qol-tray/plugins/`. Each plugin requires:

1. A directory with the plugin name
2. A `plugin.toml` manifest file
3. A `run.sh` executable script

Example structure:
```
~/.config/qol-tray/plugins/my-tool/
├── plugin.toml
├── run.sh
└── config.json (optional)
```

See `examples/plugins/screen-recorder/` for a complete example.

## Plugin Manifest

Minimal `plugin.toml`:
```toml
[plugin]
name = "My Tool"
description = "Brief description"
version = "1.0.0"

[menu]
label = "My Tool"
items = [
    { type = "action", id = "run", label = "Run", action = "run" }
]
```

Reload plugins from the tray menu after adding new ones.

## License

MIT
