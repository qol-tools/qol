# plugin-launcher

Universal file search launcher for qol-tray.

## Usage

Trigger via hotkey, then:

| Key | Action |
|-----|--------|
| `Enter` | Open file/directory |
| `Ctrl+Enter` | Open in terminal |
| `Shift+Enter` | Open containing folder |
| `Alt+Enter` | Copy path to clipboard |
| `Esc` | Close |

## Features

- Instant startup
- Window appears on monitor with focused window
- Multi-word search support
- Searches mounted drives under `/media/`

## Dependencies

| Platform | Requirements | Status |
|----------|--------------|--------|
| Linux | `plocate`, `xdotool` | ✓ Tested |
| macOS | Spotlight (built-in) | Untested |
| Windows | [Everything CLI](https://www.voidtools.com/support/everything/command_line_interface/) | Untested |

### Linux: Index Mounted Drives

```bash
~/.config/qol-tray/plugins/plugin-launcher/backends/update-dbs.sh
```

## License

MIT
