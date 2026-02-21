# Configuration

## Effective Runtime Contract

The current runtime uses a single brick/grid layout path. Layout mode switching is not part of active runtime behavior.

## Config File Location

The plugin checks these locations (first valid JSON wins):

- Install-scoped paths under active QoL Tray install:
  - `plugins/plugin-alt-tab/config.json`
  - `plugins/alt-tab/config.json`
- Legacy paths under config dir:
  - `qol-tray/plugins/plugin-alt-tab/config.json`
  - `qol-tray/plugins/alt-tab/config.json`

## Schema

Current deserialization accepts:

```json
{
  "display": {}
}
```

or empty object:

```json
{}
```

Unknown fields are ignored by default serde behavior unless strict parsing is later introduced.

## Legacy Keys

- `display.preview_mode` is considered legacy from prior multi-layout implementations.
- Keeping this key in config will not change runtime layout behavior in the current implementation.
