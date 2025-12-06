# Session Handoff

## Current State

Fully working launcher plugin for qol-tray (Linux only for now).

- Borderless WebView window (wry + tao)
- Daemon mode with instant show via Unix socket
- Real-time file search via plocate
- Multi-word search with ~80ms performance
- Modifier key actions (Ctrl/Shift/Alt + Enter)
- Window positioning on monitor with focused window
- 75 Rust unit tests

## What Works

- `run.sh` sends "show" via socket for instant display
- Window centers on monitor where focused window is
- Search queries run in background thread
- Custom plocate databases for mounted drives under `/media/`
- All modifier key actions (open, terminal, folder, copy)
- Escape or focus loss hides window (daemon stays running)

## Known Issues / TODO

1. **Wayland support** - Uses `xdotool` for focused window detection and `xclip` for clipboard. Needs alternatives.

2. **Terminal detection** - Currently tries gnome-terminal, konsole, xfce4-terminal, xterm in order. Should detect user's default terminal.

## Development

Plugin directory is symlinked via qol-tray Developer tab or manually:
```
~/.config/qol-tray/plugins/plugin-launcher -> /path/to/plugin-launcher
```

Workflow:
1. Edit files
2. Run `cargo build --release`
3. Kill daemon: `pkill -f launcher; rm /tmp/qol-launcher.sock`
4. Trigger hotkey to test

## Releasing

`make release` does everything locally:
1. Runs tests
2. Bumps version
3. Builds release binary
4. Commits, pushes
5. Creates GitHub release with `launcher-linux-x86_64` binary

## Tests

```bash
make test                         # 75 Rust tests
./tests/test_linux_backend.sh    # Bash backend tests
```
