# qol-config

[![tests](https://github.com/qol-tools/qol-config/actions/workflows/tests.yml/badge.svg)](https://github.com/qol-tools/qol-config/actions/workflows/tests.yml)
[![lint](https://github.com/qol-tools/qol-config/actions/workflows/lint.yml/badge.svg)](https://github.com/qol-tools/qol-config/actions/workflows/lint.yml)

Versioned plugin configuration contract for the QoL ecosystem.

## Quick start

```toml
[dependencies]
qol-config = { git = "https://github.com/qol-tools/qol-config" }
```

```rust
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct MyConfig { enabled: bool }

const PLUGIN_NAMES: &[&str] = &["plugin-foo", "foo"];

let config: MyConfig = qol_config::load_plugin_config(PLUGIN_NAMES);
```

## About

Defines what a plugin can express in `qol-config.toml`, validates it, and normalizes it into a renderer-friendly form for `qol-tray`. The plugin contract is declarative; rendering logic stays in `qol-tray`.

For the v1 schema reference, see [docs/v1.md](./docs/v1.md).

## License

PolyForm Noncommercial 1.0.0
