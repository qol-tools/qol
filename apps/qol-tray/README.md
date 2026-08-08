<div align="center">

# QoL Tray

A system tray daemon for running desktop behavior plugins.

</div>

## Quick start

```bash
cargo setup
qol install
```

Run `qol-tray`, click the tray icon to open the UI at `http://127.0.0.1:42700`, and install plugins from the store.

## About

Plugins are standalone Rust binaries, one process each; qol-tray discovers, configures, and launches them. The dev CLI is documented in [docs/qol-commands.md](docs/qol-commands.md), and [diagram/](diagram/) holds an interactive map of the runtime, opened by loading `diagram/Runtime Architecture Map.html` in a browser.

## License

PolyForm Noncommercial 1.0.0
