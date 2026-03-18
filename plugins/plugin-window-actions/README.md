# Window Actions

Window snapping, centering, minimize/restore, and multi-monitor management for qol-tray.

## Features

- **Snap Left/Right/Bottom** - Tile windows to half of screen
- **Center** - Center window with reasonable size
- **Maximize** - Fill the screen
- **Minimize/Restore** - Instant minimize (no animation) and restore with state tracking
- **Move to Left/Right Monitor** - Move windows between monitors with proportional scaling

## Platform Support

### macOS

All actions use the Accessibility API (AXUIElement) directly — no AppleScript or JXA.

**Instant minimize**: AXHidden mask trick hides the app, minimizes the target window while hidden, then unhides. The minimize animation plays invisibly. Java/JetBrains apps fall back to plain minimize (their rendering pipeline ignores AXHidden).

**Restore**: Unminimize + 1px position nudge to force WindowServer input re-registration + NSRunningApplication activation for focus.

**Saved geometry**: Window position/size is saved before minimize and restored on unminimize via a file in `$TMPDIR`.

Requires: Accessibility permission (System Settings → Privacy & Security → Accessibility).

### Linux

Uses X11 tools for window operations and Cinnamon D-Bus API for geometry actions.

Requires: `xdotool`, `wmctrl`, `xprop`, `gdbus` (Cinnamon desktop).

## Multi-Monitor Support

Moving windows between monitors preserves proportions:
- Window size scales relative to monitor resolution
- Position scales relative to monitor dimensions

## Installation

Install via qol-tray plugin system or manually:

```bash
git clone https://github.com/qol-tools/plugin-window-actions.git ~/.config/qol-tray/plugins/plugin-window-actions
```

## Usage

Bind actions to hotkeys in qol-tray settings.

## License

PolyForm Noncommercial 1.0.0
