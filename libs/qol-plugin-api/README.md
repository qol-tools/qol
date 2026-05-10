# QoL Plugin API

[![tests](https://github.com/qol-tools/qol-plugin-api/actions/workflows/tests.yml/badge.svg)](https://github.com/qol-tools/qol-plugin-api/actions/workflows/tests.yml)
[![lint](https://github.com/qol-tools/qol-plugin-api/actions/workflows/lint.yml/badge.svg)](https://github.com/qol-tools/qol-plugin-api/actions/workflows/lint.yml)

Shared library for [QoL Tray](https://github.com/qol-tools/qol-tray) plugins.

## Quick start

```toml
[dependencies]
qol-plugin-api = { git = "https://github.com/qol-tools/qol-plugin-api" }
```

## About

Common utilities so plugins don't duplicate platform-specific code: config loading, Unix socket daemon, platform state queries, search/frecency scoring, app icon retrieval, and shared window/monitor types.

## License

PolyForm Noncommercial 1.0.0
