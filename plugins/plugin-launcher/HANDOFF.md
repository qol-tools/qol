# Session Handoff

## Current State

Working launcher plugin for qol-tray with:
- Borderless WebView window (wry + tao)
- Real-time file search via platform backends
- Modifier key actions (Ctrl/Shift/Alt + Enter)
- Embedded UI (no file I/O on startup)
- ~811KB binary size

## What Works

- Window opens and displays correctly
- Search queries are sent to backend scripts
- Results render with keyboard navigation (↑/↓)
- Modifier key hints show in top-right when held
- Actions execute (open, terminal, folder, copy)
- Escape closes the window

## Known Issues / TODO

1. **Startup speed** - Still takes ~200-300ms to show window. WebKitGTK initialization is the bottleneck. Could explore:
   - Keeping a hidden window ready (daemon mode)
   - Using a lighter rendering approach

2. **Terminal detection** - Currently tries gnome-terminal, konsole, xfce4-terminal, xterm in order. Should detect user's default terminal.

3. **Search scoring** - Results come back unsorted from plocate. Could integrate frequency tracking from the original systemsearch extension.

4. **Window positioning** - Currently uses default position. Should center on active monitor.

5. **Focus** - Input field should auto-focus on window open (currently does via `autofocus` attribute).

## Development Setup

Plugin directory is symlinked:
```
~/.config/qol-tray/plugins/plugin-launcher -> /media/kmrh47/WD_SN850X/Git/qol-tools/plugin-launcher
```

Workflow:
1. Edit files in `/media/kmrh47/WD_SN850X/Git/qol-tools/plugin-launcher/`
2. Run `cargo build --release`
3. Trigger hotkey in qol-tray to test

Note: UI changes require rebuild since HTML/CSS/JS are embedded via `include_str!()`.

## Context

This plugin replaces the Ulauncher-based systemsearch extension. The goal is a universal, cross-platform launcher that:
- Works on Linux, macOS, Windows
- Uses native search backends (plocate, mdfind, Everything)
- Integrates with qol-tray's hotkey system
- Provides consistent UX across all platforms

The original plan included a custom query language (`l query -t` for terminal), but this was replaced with modifier keys since we control the full UI now.
