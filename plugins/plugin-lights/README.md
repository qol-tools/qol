# Plugin Lights

Lights domain plugin for `qol-tray`.

The first backend target is `Zigbee2MQTT`, with the plugin shaped around a long-running daemon, backend adapters, and capability-driven light control.

## Current status

- scaffolded from `plugin-template`
- runtime and daemon entrypoints exist
- `plugin.toml` exposes stable v1 action IDs
- domain, backend, service, config, and daemon seams exist
- `config.json` is the first settings surface and is intended for `qol-tray` auto-config
- `ui/index.html` is a thin auto-config shell so settings work on older `qol-tray` builds too
- `Zigbee2MQTT` backend is still a stub

## Near-term goal

Get one real end-to-end path working:

1. connect to `Zigbee2MQTT`
2. discover the main RGB+CCT target
3. toggle power
4. adjust brightness
5. set color
6. set color temperature
7. trigger one preset through a stable action slot

## Repo shape

- `src/main.rs` wires the runtime entrypoint
- `src/lib.rs` exposes the plugin modules
- `src/runtime/` handles runtime action dispatch
- `src/daemon/` owns the socket daemon
- `src/backend/` isolates backend implementations
- `src/service/` keeps orchestration out of transport code
- `src/domain/` holds transport-agnostic light types
- `src/config/` holds plugin configuration shape and validation
- `src/platform/` keeps settings launch behavior platform-specific

## Contract notes

- commands stay binary basenames only
- stable action IDs are fixed for v1 so hotkeys and launcher integration remain practical
- daemon and runtime share the same binary
- platform-specific behavior stays behind `src/platform/`

## License

PolyForm Noncommercial 1.0.0
