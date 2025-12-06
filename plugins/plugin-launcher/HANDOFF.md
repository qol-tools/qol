# Session Handoff

## Current State

Fully working launcher plugin for qol-tray with:
- Borderless WebView window (wry + tao)
- Daemon mode with instant show via Unix socket
- Real-time file search via platform backends
- Multi-word search with ~80ms performance
- Modifier key actions (Ctrl/Shift/Alt + Enter)
- Window positioning on monitor with focused window
- Always on top with focus grab
- 29 Rust unit tests, 10 bash backend tests

## What Works

- `init.sh --preload` starts daemon with hidden window
- `run.sh` sends "show" via socket for instant display
- Window centers on monitor where focused window is (not cursor)
- Search queries run in background thread
- Custom plocate databases for mounted drives under `/media/`
- All modifier key actions (open, terminal, folder, copy)
- Escape or focus loss hides window (daemon stays running)

## Known Issues / TODO

1. **Terminal detection** - Currently tries gnome-terminal, konsole, xfce4-terminal, xterm in order. Should detect user's default terminal.

2. **Search scoring** - Results come back unsorted from plocate. Could integrate frequency tracking from the original systemsearch extension.

3. **macOS/Windows backends** - Exist but untested.

## Development Setup

Plugin directory is symlinked:
```
~/.config/qol-tray/plugins/plugin-launcher -> /path/to/plugin-launcher
```

Workflow:
1. Edit files
2. Run `cargo build --release`
3. Kill daemon: `pkill -f launcher; rm /tmp/qol-launcher.sock`
4. Trigger hotkey to test

Note: UI changes require rebuild since HTML/CSS/JS are embedded via `include_str!()`.

## Tests

```bash
cargo test                        # 29 Rust tests
./tests/test_linux_backend.sh    # 10 bash tests
```

## Context

This plugin replaces the Ulauncher-based systemsearch extension. The goal is a universal, cross-platform launcher that:
- Works on Linux, macOS, Windows
- Uses native search backends (plocate, mdfind, Everything)
- Integrates with qol-tray's hotkey system
- Provides consistent UX across all platforms
