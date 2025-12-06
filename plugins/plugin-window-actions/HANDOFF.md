# Session Handoff

## Current State

Window management plugin for qol-tray (Linux only for now). Scripts for minimize, restore, and move between monitors.

## Known Issues / TODO

1. **Wayland support** - Uses X11-only tools:
   - `xdotool` for window minimize and mouse position
   - `xprop` for window properties
   - `wmctrl` for window activation

   Wayland alternatives:
   - `wlr-foreign-toplevel-management` protocol (wlroots compositors)
   - `xdg-desktop-portal` for some operations
   - Compositor-specific D-Bus APIs (GNOME, KDE)

## Scripts

- `minimize.sh` - Minimize active window
- `restore.sh` - Restore last minimized window
- `move-monitor-left.sh` - Move window to left monitor
- `move-monitor-right.sh` - Move window to right monitor
