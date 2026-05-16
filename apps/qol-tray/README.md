# QoL Tray

[![tests](https://github.com/qol-tools/qol-tray/actions/workflows/tests.yml/badge.svg)](https://github.com/qol-tools/qol-tray/actions/workflows/tests.yml)
[![lint](https://github.com/qol-tools/qol-tray/actions/workflows/lint.yml/badge.svg)](https://github.com/qol-tools/qol-tray/actions/workflows/lint.yml)

A system tray daemon for running desktop behavior plugins.

## Quick start

```bash
git clone https://github.com/qol-tools/qol-tray
cd qol-tray
make install
```

Run `qol-tray`, click the tray icon to open the UI at `http://127.0.0.1:42700`, and install plugins from the store.

## About

Plugins are standalone Rust binaries discovered from the [qol-tools](https://github.com/qol-tools) GitHub org. Each runs as its own process; qol-tray discovers, configures, and launches them.

## Architecture

Two views of the same system:

- [`ARCHITECTURE.md`](ARCHITECTURE.md), code-grounded textual reference. Every claim anchors to a `file:line`. Covers the 21-step boot sequence, the three IPC channels, the plugin runtime, the hotkey backends, and the persistence quadrant.
- [Runtime Architecture Map](diagram/Runtime%20Architecture%20Map.html), interactive SPA of the same material. Open with `make diagram`; `make diagram-build` rebuilds compiled JS after editing the `.jsx` sources. `data.js` is plain JS, edit and refresh.

## License

PolyForm Noncommercial 1.0.0
