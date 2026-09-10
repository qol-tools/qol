# Monitor arrangement canvas editor

Status: frozen v2. Supersedes the window-based v1 (main `54cf85731`), which is
being removed. Companion to `2026-09-10-monitor-management.md`.

## 1. Goal

Inside the monitor settings window, a pane where connected displays render as
rectangles at their real relative positions. The user drags or nudges them,
picks resolution and refresh per display, chooses the primary, and presses
Apply. Nothing reaches the hardware before Apply. This is the Cinnamon display
dialog experience, as a pane in the settings window rather than a second window.

## 2. Surface

A new contract field kind `display_layout`, rendered by the shared GPUI
settings panel (`libs/gpui/src/settings_panel/`) as a pushed card pane, the way
the Bluetooth device list opens a card from its `list` field. The plugin keeps
`[capabilities] gpui = true` and its existing `[action.settings]`; there is no
plugin-owned window, no daemon-spawned UI child, and no web implementation.

The field reuses existing `FieldSpec` slots, so no new field keys exist:

```toml
[field.arrangement]
type = "display_layout"
label = "Arrangement"
section = "arrangement"
query = "layout"
active_query = "modes"
action = "arrange"
active_action = "set_mode"
```

Cross-validation requires the kind to declare all four and requires each name to
be declared in `qol-runtime.toml` (`layout` and `modes` as queries, `arrange`
and `set_mode` as actions). The kind is runtime-only: no stored value, no
default. The web config page registers the kind for registry parity and shows
its unsupported marker; the native panel is the surface.

Removed with this correction: `plugins/monitor/src/ui/`, the monitor daemon's
`OpenCanvas` command and child management, the `open` and `ui` CLI verbs,
`[action.open]` plus its launcher shortcut, the runtime `open` entry, and
`MonitorPalette` in `libs/theme`. The settings page field
`[field.open_arrangement]` is replaced by `[field.arrangement]` above.

## 3. Panel integration

- `RowControl::DisplayLayout` carries a boxed `DisplayLayoutState`.
- `row_query_names` returns `layout` and `modes`; `apply_runtime_query` merges
  fresh geometry and mode lists into the state, preserving staged edits for
  displays that remain present.
- The root page shows one compact row. Enter pushes the card
  (`open_display_layout_card` modeled on `open_object_array_card`), whose body
  renders the stage, the selected display's mode dropdown, the primary control,
  and Apply and Cancel rows. The card body scrolls inside the fixed panel
  height; the stage gets its own height constant beside
  `PANEL_GAMEPAD_HEIGHT`.
- The card is a real navigable level: five rows walked with the shared
  `level.selected`, painted with `SettingsRow` selection, scrolled by
  `sync_scroll`, and activated by Enter on the selected row. Every row is also
  clickable through the same `SettingsRow` on_click path (pointing hand and
  hover wash). The Apply row carries the panel's commit affordance, the same
  shape the object array draft `Save` line uses: an accent label plus a
  caption-sized muted `enter` hint, never a keycap. The Cancel row keeps the
  shared `esc` keycap.
- Apply dispatches through `SettingsRuntime::run_action`, exactly like a list
  row action: one `set_mode` per display whose mode changed, then one `arrange`
  carrying every display plus the top-level primary. Pending shows the shared
  action spinner; refusals render through the shared feedback. On success the
  committed baseline replaces the staged state. Cancel and card-popping discard
  staged state without writing.
- After every query refresh, a `sync_display_layout_card` keeps the card in step
  beside `sync_list_card` and `sync_live_card`.

## 4. Canvas behavior

### 4.1 Rendering

- Geometry from the polled `layout` rows: one rectangle per display labelled
  with its connector and current resolution; `refresh_hz == 0` means the
  current mode is unreadable and the label omits the rate.
- Union bounding box, padded, scaled to fit the stage with one factor for both
  axes; origins round to whole pixels.
- Primary marker, selection ring, and overlap/refusal danger all come from the
  shared kit and `SettingsPanelPalette`. The stage is a relative div, each
  display an absolutely positioned child with mouse handlers; a `canvas`
  bounds capture on the stage maps pointer positions.
- The selected display reads unmistakably: a two-pixel accent border
  (`row_border_selected`), the `row_bg_selected` fill, and an explicit
  `selected` pill in the tile header from `Kit::status_pill`. A conflicted tile
  keeps the danger tone at the same width so a selected-and-conflicted tile
  reads as both. The style decision lives in one pure helper,
  `display_layout_tile_style`, so the tile has a single source of truth.
- Mode options come from the `modes` rows joined by display id, rendered with
  the shared `Dropdown`. A display with no rows renders its mode control
  unavailable, never an empty dropdown. When the payload reports every display
  unwritable, the mode control is disabled.
- Empty state: a short message.

### 4.2 Staged interaction

- Drag: press selects and starts a drag; move converts the client delta through
  the inverse scale and updates only the staged position; release leaves it
  staged. Nothing is written.
- Snap: edges and centers snap to other rectangles' edges and centers within
  ten screen pixels, and to a one-pixel grid otherwise. The threshold is
  expressed in screen pixels and converted through the fit scale, so it feels
  the same at any canvas zoom. Edge snapping is
  exact: touching edges share a coordinate, never overlap by one pixel and
  never leave a one-pixel gap.
- Keyboard: arrow keys move the card's row selection and, while the card owns
  an edit mode, they move the selected display by ten pixels per press and by
  one pixel with Shift for fine adjustment. Enter on the Apply row commits;
  Escape discards the staged edits first and pops the card on the second press.
  Outside the edit mode, Up/Down keep panel navigation.
- Mode and primary changes stage only. The primary control stages a new primary;
  the commit never issues `set_primary`, because the primary rides in the single
  `arrange` payload.
- Apply payloads: `arrange` with `placements: [{id, x, y}]` for every display
  plus `primary`; `set_mode` with `{id, token, width, height, refresh}` where a
  refresh of 0 serializes as null.
- Failure keeps the staged edits so the user can retry; the next successful
  `layout` poll clears a stale commit error.

### 4.3 Client validation

The daemon stays the authority; the staged state mirrors these rules:

- Every display exactly once, exactly one primary.
- Whole pixels; X11 range `-32768..=32767`.
- Overlap is a true intersection on both axes; edge touching is valid,
  matching the backend's strict inequalities.
- Overlap is evaluated against staged mode sizes when a mode change is staged. A staged mode change that would overlap a neighbour resolves the overlap by shifting the changed display to exact edge contact along the axis of least overlap, rather than blocking the apply; the shift is clamped to the coordinate range and falls back to shifting the neighbour when the changed display cannot move.
- The committed payload is normalized so the smallest x and the smallest y across all displays are zero, preserving every relative offset and the primary. The canvas may stage a display to the left of or above the origin, but the write is compacted to a non-negative origin, because X servers commonly refuse CRTC positions below zero.

## 5. Reuse

Chrome and controls come from the shared kit: `Kit::header`, `Kit::section`,
`Kit::row_selected`, `Kit::chip`, `Kit::status_pill`, `Kit::segment` and
`Kit::segmented_group`, `Kit::button_ghost`, `Kit::keycap`, `Kit::hint_bar` and
`Kit::hint`, `Kit::focus_ring`; `SettingsRow::setting` and `SettingsRow::rule`,
`SettingsRow::paint_settings_selection`, `SettingsGroupHeader`,
`SettingsFeedback`, `settings_action_spinner`, `settings_dropdown_style`;
`Dropdown`; `StatusIndicator`. No hand-rolled colors, chips, buttons, or
selection styling.

## 6. Files

- Contract: `libs/config/src/contract/v1.rs` (kind, name, has_stored_value),
  `libs/config/src/validation.rs` (`default_matches_kind`),
  `libs/config/src/contract/cross_validate.rs` (query and action reference
  checks), `apps/qol-tray/src/plugins/config/mod.rs`
  (`field_default_matches_kind`), `apps/qol-tray/ui/views/plugin-config/`
  (registry parity and runtime-only kind list), and the kind lists in
  `libs/config/docs/v1.md` and `docs/plugin-contract.md`.
- Panel: `libs/gpui/src/settings_panel/display_layout.rs` (pure model plus
  state), `rows.rs` (control, queries, payload application), `view.rs` (card
  open, render, keyboard branch, dispatch, sync), `mod.rs` (stage height
  constant).
- Plugin: `plugins/monitor/qol-config.toml` (the field), `plugin.toml` (remove
  `[action.open]` and the shortcut), `qol-runtime.toml` (remove `[action.open]`),
  `src/daemon.rs` (remove `OpenCanvas`, `CanvasHost`, socket helper), `src/cli.rs`
  (remove `open` and `ui`), `src/lib.rs` (remove `pub mod ui`),
  `src/main.rs` (manifest test expectations), `Cargo.toml` (drop gpui and
  qol-gpui; restore the lock), and delete `plugins/monitor/src/ui/`.
- Theme: remove `MonitorPalette`, `monitor_runtime`, the component palettes
  field, its test, and the literal-test file entry.

## 7. Tests

- Model: the migrated geometry, snap, overlap, staged-edit, and payload tests
  under `cargo test -p qol-gpui`, including payload round trips through the
  daemon parsers and staged-edit retention across polls.
- Panel: control mapping and both queries; card open carries state; Apply
  dispatches the exact `set_mode` and `arrange` JSON; pending and error
  surfacing; Escape discards staged edits before popping; Cancel leaves the
  root untouched; fixed row height and panel width stay green.
- Contract: parses the field, rejects each undeclared reference, rejects the
  kind on other fields.
- Plugin: the shipped contracts validate together and the manifest test matches
  the new field.
- Guest: open monitor settings, confirm the arrangement row opens the card
  pane in the same window, drag or nudge to stage, Apply, confirm
  `plugin-monitor layout` reflects it, Cancel leaves the layout unchanged, and
  a refusal surfaces in the pane.

## 8. Out of scope

Rotation, display scale, mirroring, per-display gamma, mirrored displays that
share an origin, and macOS mode writes.
