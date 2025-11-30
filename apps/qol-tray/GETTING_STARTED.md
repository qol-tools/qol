# Getting Started

## Installation

```bash
cd /path/to/qol-tray
make install
```

This installs the daemon and sets up autostart.

## Running

```bash
qol-tray
```

A tray icon will appear. Right-click to access plugins.

## Screen Recorder

The included screen recorder plugin:

1. Right-click tray icon → "Screen Recorder" → "Start/Stop Recording"
2. Select screen region with mouse
3. Recording starts automatically
4. Use menu again to stop

Files are saved to `~/Videos/`.

**Configuration:** Edit `~/.config/qol-tray/plugins/screen-recorder/config.json`

Audio options:
- `enabled`: Enable/disable audio recording
- `inputs`: `["mic"]`, `["system"]`, or `["mic", "system"]`
- `mic_device`: Audio input device (default: "default")
- `system_device`: System audio device (default: "default")

Video options:
- `crf`: Quality, 0-51 (lower = better, default: 18)
- `preset`: Speed preset (default: "veryfast")
- `framerate`: FPS (default: 60)
- `format`: "mkv" or "mp4" (default: "mkv")

## Adding Your Own Tools

### Basic Plugin

1. Create directory:
```bash
mkdir -p ~/.config/qol-tray/plugins/my-tool
```

2. Create `plugin.toml`:
```toml
[plugin]
name = "My Tool"
description = "Does something"
version = "1.0.0"

[menu]
label = "My Tool"
items = [
    { type = "action", id = "run", label = "Run", action = "run" }
]
```

3. Create executable `run.sh`:
```bash
#!/usr/bin/env bash
notify-send "My Tool" "Running"
```

4. Make executable:
```bash
chmod +x ~/.config/qol-tray/plugins/my-tool/run.sh
```

5. Reload: Right-click tray → "Reload Plugins"

### Reading Configuration

If your plugin needs settings, create `config.json`:

```json
{
  "enabled": true,
  "option": "value"
}
```

Read in your script:
```bash
#!/usr/bin/env bash
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="$SCRIPT_DIR/config.json"

ENABLED=$(jq -r '.enabled' "$CONFIG")
```

Add menu toggle:
```toml
[[menu.items]]
type = "checkbox"
id = "toggle"
label = "Enable Feature"
checked = true
action = "toggle-config"
config_key = "enabled"
```

## Troubleshooting

**Tray icon not appearing:**
- Ensure your desktop environment supports system tray
- Check logs: `RUST_LOG=debug qol-tray`

**Screen recorder not working:**
- Install dependencies: `sudo apt install slop ffmpeg x11-xserver-utils jq`
- Check log file: `/tmp/record-region.log`

**Plugin not showing:**
- Verify `plugin.toml` syntax is valid
- Ensure `run.sh` is executable
- Reload plugins from tray menu
