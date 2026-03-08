# QoL Tray

A lightweight system tray daemon that lets you control how your desktop behaves through plugins. Alt-tabbing, window management, app launching, keyboard remapping, screen recording. Install what you need, configure it how you like.

Plugins are standalone Rust binaries discovered automatically from the [qol-tools](https://github.com/qol-tools) GitHub org. Browse and install them from the built-in plugin store.

## Getting started

```bash
git clone https://github.com/qol-tools/qol-tray
cd qol-tray
make install
```

Run `qol-tray`. Click the tray icon to open the UI at `http://127.0.0.1:42700`, then install plugins from the store.

## Platform support

| Platform | Status |
|----------|--------|
| macOS | Supported |
| Linux (X11) | Supported |
| Linux (Wayland) | Partial (tray works, some plugins need X11 for window capture) |

## Roadmap

The long term goal is a portable, personal environment that follows you across machines and operating systems. Install qol-tray on any PC and get the same experience you have at home.

What exists today:
- Plugin system with a self-service store
- Per-plugin configuration (local)
- Global hotkey binding
- Task runner

What is planned:
- Config sync across machines
- Windows support
- Lower friction first-install (USB, wireless, etc.)

## Plugin store

The built-in store currently only discovers plugins from the [qol-tools](https://github.com/qol-tools) GitHub org. Support for additional plugin sources does not exist yet but could be added.

## Creating a plugin

> See [plugin-template](https://github.com/qol-tools/plugin-template) for a ready-to-go starting point with CI, release workflows, and a valid plugin contract.

A plugin is a standalone Rust binary with a `plugin.toml` manifest. It can live in any repo or org.

## License

MIT
