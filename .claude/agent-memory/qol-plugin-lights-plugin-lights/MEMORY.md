
## 2026-06-24
- When migrating plugin config to qol_runtime::plugin_config, the load/save API does not return Result — load() returns the config directly and save() returns bool.
- plugin-lights Cargo.toml still declares `qol-config.workspace = true` even after src no longer references it; check Cargo.toml deps separately when removing a crate's usage.
