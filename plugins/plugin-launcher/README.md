# plugin-launcher

Universal file search launcher for qol-tray.

## Installation

```bash
git clone https://github.com/qol-tools/plugin-launcher ~/.config/qol-tray/plugins/plugin-launcher
```

## Usage

Trigger via hotkey, then:

| Key | Action |
|-----|--------|
| `Enter` | Open file/directory |
| `Ctrl+Enter` | Open in terminal |
| `Shift+Enter` | Open containing folder |
| `Alt+Enter` | Copy path to clipboard |
| `Esc` | Close |

## Search Backends

| Platform | Backend |
|----------|---------|
| Linux | plocate |
| macOS | mdfind (Spotlight) |
| Windows | Everything CLI / Windows Search |

## Building

```bash
cargo build --release
```

## License

MIT
