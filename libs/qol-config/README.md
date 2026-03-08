# qol-config

Config discovery and loading for qol-tools plugins. Searches multiple directories (install-specific, base data dir, XDG config dir) and loads the first valid `config.json` it finds.

## Usage

```rust
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct MyConfig {
    enabled: bool,
}

const PLUGIN_NAMES: &[&str] = &["plugin-foo", "foo"];

let config: MyConfig = qol_config::load_plugin_config(PLUGIN_NAMES);
```

`load_plugin_config` tries each name under each config root, looking for `plugins/{name}/config.json`. If nothing is found or parsing fails, it returns `T::default()`.

## Config resolution order

1. `$QOL_TRAY_INSTALL_ID` install directory (if set)
2. Active install from `active-install-id` file
3. Base data directory (`~/.local/share/qol-tray/` on Linux)
4. XDG config directory (`~/.config/qol-tray/`)

## License

MIT
