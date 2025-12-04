# PointZ

A [qol-tray](https://github.com/qol-tools/qol-tray) plugin for remote PC control from mobile devices.

## Features

- **Remote control** - Control your PC from any mobile device on your network
- **Auto-discovery** - Mobile app automatically finds your PC
- **QR code setup** - Scan to download and connect instantly
- **Background daemon** - Runs silently, always ready

## Dependencies

The plugin automatically downloads the `pointzerver` binary from the [pointZ releases](https://github.com/qol-tools/pointZ/releases).

No manual dependency installation required.

## Installation

Install via qol-tray Plugin Store, or manually:

```bash
git clone https://github.com/qol-tools/plugin-pointz ~/.config/qol-tray/plugins/plugin-pointz
```

## Usage

1. The daemon starts automatically with qol-tray
2. Click **PointZ → Settings** in the tray menu
3. Scan the QR code to download the mobile app
4. The app auto-discovers and connects to your PC

## Ports

| Port  | Protocol | Purpose         |
|-------|----------|-----------------|
| 45454 | UDP      | Discovery       |
| 45455 | TCP      | Command/Control |
| 45460 | HTTP     | Status API      |

## More Information

For details about the PointZerver daemon and standalone usage, see the [main pointZ repository](https://github.com/qol-tools/pointZ).

## License

MIT
