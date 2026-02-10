# QoL Tray

A pluggable system tray daemon. One tray icon, infinite possibilities.

## Installation

```bash
git clone https://github.com/qol-tools/qol-tray
cd qol-tray
cargo run --release --bin qol-tray-install
```

On Linux and macOS, `make install` runs the same installer command.
On Windows, run `cargo run --release --bin qol-tray-install` from PowerShell or Command Prompt.

## Usage

Run `qol-tray`. Click the tray icon to open the browser UI at `http://127.0.0.1:42700`.

From there you can:
- Browse and install plugins
- Configure plugin settings
- Check for updates

## Platform Support

| Platform | Status |
|----------|--------|
| Linux (X11) | Supported |
| Linux (Wayland) | Partial |
| macOS | Supported |
| Windows | Planned |

## Roadmap Notes

- Cross-platform daemon IPC abstraction:
  - Unix: Unix domain sockets (current)
  - Windows: named pipes (planned)
  - Keep plugin action dispatch daemon-first where daemon is configured.
  - Keep manifest contract platform-agnostic; platform transport selected by host.

## License

MIT
