# qol-config

`qol-config` is the versioned plugin configuration contract for the QoL ecosystem.

It defines what a plugin can express in `qol-config.toml`, validates that contract, and normalizes it into a renderer-friendly form for `qol-tray`.

## Purpose

The intended flow is:

1. a plugin authors one `qol-config.toml`
2. `qol-cicd` validates it with `qol-config`
3. `qol-tray` consumes the normalized form and renders the settings UI
4. `qol-tray` stores user overrides outside the plugin repo

This keeps the plugin contract declarative and keeps rendering logic out of plugins.

## Contract vs Renderer

`qol-config` owns:

- the `qol-config.toml` schema
- supported field types
- validation rules
- normalization
- defaults and override merging

`qol-tray` owns:

- the actual HTML/UI rendering
- save flow
- tray-specific layout choices
- visual affordances such as segmented controls, chips, or unit adornments

That distinction matters:

- if a field can exist, validate, and normalize, it belongs in `qol-config`
- if a field is shown as a pill, slider, row, card, or grouped control, that belongs in `qol-tray`

## Authoring Model

A plugin may ship:

- `plugin.toml`
- `qol-config.toml`

The plugin authors only one config contract file.

User-edited state is not written back into `qol-config.toml`. `qol-tray` owns overrides separately and merges them with contract defaults to produce the effective config.

## Current Runtime Helper

This crate still includes the existing plugin config discovery helper used to locate `config.json`.

```rust
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct MyConfig {
    enabled: bool,
}

const PLUGIN_NAMES: &[&str] = &["plugin-foo", "foo"];

let config: MyConfig = qol_config::load_plugin_config(PLUGIN_NAMES);
```

That helper is separate from the `qol-config.toml` authoring contract.

## Supported v1 Shape

Top-level keys:

- `schema_version`
- `title`
- `description`

Top-level tables:

- `[section.<id>]`
- `[field.<id>]`

Author order is preserved for sections and fields.

## Supported Field Types

`qol-config` v1 supports:

- `boolean`
- `string`
- `number`
- `select`
- `string_array`
- `object_array`
- `object_map`

Every field requires:

- `type`

Common field keys:

- `config_key`
- `label`
- `description`
- `placeholder`
- `section`
- `default`
- `show_when`

## Sections

Section keys:

- `label`
- `description`
- `actions`

`actions` is metadata that tells the host which plugin actions a section affects.

Example:

```toml
[section.snap]
label = "Snap"
description = "Default fraction used by snap actions."
actions = ["snap-left", "snap-right", "snap-bottom"]
```

## Number Fields

Number fields may define:

- `min`
- `max`
- `step`

Example:

```toml
[field.center_width_px]
type = "number"
label = "Centered Width"
section = "center"
default = 1152
min = 320
step = 1
```

## Select Fields

Select fields require:

- `options`

They may also define:

- `option_labels`

Example:

```toml
[field.center_mode]
type = "select"
label = "Center Size Mode"
section = "center"
default = "pixels"
options = ["pixels", "percent"]

[field.center_mode.option_labels]
pixels = "Fixed Size"
percent = "Relative Size"
```

`options` are machine values.
`option_labels` are human-facing labels.

## show_when

Fields can be conditionally visible through:

- `[field.<id>.show_when]`

Supported keys:

- `field`
- `equals`

Example:

```toml
[field.center_width_px.show_when]
field = "center_mode"
equals = "pixels"
```

`show_when` is part of the contract.
How a host chooses to present those conditional branches is renderer behavior.

## config_key

Fields default to reading and writing the top-level config key matching their field id.

Use `config_key` when the runtime config is nested:

```toml
[field.audio_enabled]
type = "boolean"
config_key = "audio.enabled"
default = true
```

## object_array

`object_array` is for ordered lists of structured objects.

It is intended for rule-based plugin configs where each item contains a small set of scalar or `string_array` properties.

Example:

```toml
[field.key_rules]
type = "object_array"
label = "Key Rules"
section = "rules"
default = [
  { from_mods = ["ctrl"], to_mods = ["cmd"], keys = ["c", "v", "x"], global = false },
  { from_mods = ["ctrl"], from_key = "y", to_mods = ["cmd", "shift"], to_key = "z", global = false },
]
```

## object_map

`object_map` is for keyed dictionaries of structured objects.

It is intended for configs like named app launchers or named scripts.

Supported extra keys:

- `key_label`
- `entry_fields`

Example:

```toml
[field.apps]
type = "object_map"
config_key = "apps"
key_label = "App ID"
default = {}

[field.apps.entry_fields]
name = "string"
paths = "string_array"
```

## Authoring Example

```toml
schema_version = 1
title = "Window Actions"
description = "Default window sizing and monitor-move behavior."

[section.center]
label = "Center"
description = "Default centered size used by the center action."
actions = ["center"]

[field.center_mode]
type = "select"
label = "Center Size Mode"
section = "center"
default = "pixels"
options = ["pixels", "percent"]

[field.center_mode.option_labels]
pixels = "Fixed Size"
percent = "Relative Size"

[field.center_width_px]
type = "number"
label = "Centered Width"
section = "center"
default = 1152
min = 320
step = 1

[field.center_width_px.show_when]
field = "center_mode"
equals = "pixels"

[field.center_width_percent]
type = "number"
label = "Centered Width"
section = "center"
default = 0.64
min = 0.1
max = 1.0
step = 0.01

[field.center_width_percent.show_when]
field = "center_mode"
equals = "percent"

[section.monitor_move]
label = "Monitor Move"
actions = ["move-monitor-left", "move-monitor-right"]

[field.reveal_taskbar_after_move]
type = "boolean"
label = "Reveal Taskbar After Move"
section = "monitor_move"
default = true

[field.excluded_apps]
type = "string_array"
label = "Excluded Apps"
placeholder = "Bundle ID or app id"
default = []
```

## Validation Guarantees

v1 rejects:

- unsupported `schema_version`
- unknown field types
- invalid ids
- field references to missing sections
- invalid section action ids
- duplicate section action ids
- `select` fields without `options`
- empty `options`
- invalid `option_labels`
- invalid default type for a field type
- invalid override type for a field type
- select values not present in `options`
- numeric values outside `min` and `max`
- `min > max`
- `step <= 0`
- invalid `show_when` references

## Normalized Output

The normalized form exposed to `qol-tray` includes:

- title
- description
- ordered sections
- section action metadata
- ordered fields
- field type
- label
- description
- placeholder
- current value
- default value
- options
- constraints
- conditional visibility metadata

`qol-tray` should render the normalized form instead of interpreting raw TOML directly.

## Reference

For the lower-level v1 reference, see [docs/v1.md](./docs/v1.md).

## License

MIT
