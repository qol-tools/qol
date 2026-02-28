# QoL Plugin API

Shared library for [QoL Tray](https://github.com/qol-tools/qol-tray) plugins. Provides common utilities so plugins don't duplicate platform-specific code.

## What's Included

- **Config** — read and watch plugin configuration files
- **Daemon** — Unix socket server for persistent background plugins
- **Platform state** — query active monitor, cursor position, and focused window via the tray runtime
- **Search** — fuzzy matching and frecency scoring
- **App icons** — retrieve application icons by bundle ID or PID (macOS, Linux)
- **Window/monitor types** — shared data types for cross-plugin interop

## Usage

```toml
[dependencies]
qol-plugin-api = { git = "https://github.com/qol-tools/qol-plugin-api" }
```

Optional features:
- `app-icons` — enable app icon retrieval
- `gpui` (default) — GPUI integration helpers

License: MIT
