# QoL Tray

A pluggable system tray daemon. One tray icon, infinite possibilities.

## Installation

```bash
git clone https://github.com/qol-tools/qol-tray
cd qol-tray
cargo run --release --bin qol-tray-install
```

On Linux and macOS, `make install` runs the same command.

## Usage

Run `qol-tray`. Click the tray icon to open the browser UI at `http://127.0.0.1:42700`.

From there you can:
- Browse and install plugins from the built-in store
- Configure plugin settings
- Set up global hotkeys
- Check for updates

## Platform Support

| Platform | Status |
|----------|--------|
| macOS | Supported |
| Linux (X11) | Supported |
| Linux (Wayland) | Partial — tray icon works, some plugins need X11 for window capture |

## License

MIT
