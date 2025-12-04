# Window Actions

Window snapping, centering, and multi-monitor management for qol-tray.

## Features

- **Snap Left/Right/Bottom** - Tile windows to half of screen
- **Center** - Center window with reasonable size (1152x892)
- **Maximize/Minimize/Restore** - Standard window state controls
- **Move to Left/Right Monitor** - Move windows between monitors with proportional scaling

## Multi-Monitor Support

Moving windows between monitors preserves proportions:
- Window size scales relative to monitor resolution
- Position scales relative to monitor dimensions
- Works seamlessly between different resolutions (e.g., 1080p ↔ 1440p)

**Bonus:** Auto-reveals taskbar when moving to primary monitor (except fullscreen windows).

## Requirements

- Linux Mint Cinnamon (or other Cinnamon desktop)
- Multi-monitor support requires X11
- Dependencies: `gdbus`, `xdotool` (for panel reveal)

## Installation

Install via qol-tray plugin browser or manually:

```bash
git clone https://github.com/qol-tools/plugin-window-actions.git ~/.config/qol-tray/plugins/plugin-window-actions
```

## Usage

Bind actions to hotkeys in qol-tray settings for instant window management.

## Implementation

All window operations use the Cinnamon D-Bus API for instant, reliable execution:
- 0ms latency
- No stale geometry issues
- Clean, simple scripts (~10-40 lines each)
- No complex X11 workarounds needed

## License

MIT
