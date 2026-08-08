# plugin-controllers settings UI design

Date: 2026-07-08
Status: approved (chat)

## Purpose

Give plugin-controllers a settings page in qol-tray showing detected known controllers, their fix states, and an apply button.
Rendered entirely by the tray's auto-config system; no custom frontend code.

## Approach

Auto-config page driven by `qol-config.toml`, live data via daemon queries, mirroring plugin-lights' status/list/action field usage.
The daemon stays the single source of truth; the CLI remains a full-fidelity interface (headless-first preserved).

## qol-config.toml

Title "Controllers", one section `fixes` labeled "Controller Fixes":

- `field.fixes_status` (`type = "status"`): `query = "controllers_status"`, `value_from = "state"`.
  `label_map`: `ok` = "All fixes applied", `pending` = "Fix available", `driver_missing` = "Driver missing", `none` = "No known controllers connected".
  `tone_map`: `ok` = "success", `pending` = "danger", `driver_missing` = "danger".
  Tones mirror plugin-lights (only success/danger are exercised in production).
- `field.detected_pads` (`type = "list"`): `query = "list_controllers"`, `row_label = "{name}"`, `row_subtitle = "{mac} - {state}"`, `empty_message = "No known controllers connected."`.
- `field.apply_fixes` (`type = "action"`): `action = "apply_fixes"`, `variant = "primary"`, description noting the single pkexec authorization prompt.

## qol-runtime.toml

New file declaring the actions and queries, mirroring plugin-lights:

- `[action.apply_fixes]` with description.
- `[query.controllers_status]` and `[query.list_controllers]`, `poll_interval_ms = 3000`.
  The ListField/StatusField poll these, so the page live-updates as pads connect and disconnect.

## Daemon queries

Two new actions answered with `HandledWithData`:

- `controllers_status` returns `{ "state": <ok|pending|driver_missing|none>, "message": <string> }`.
  Aggregation, worst first: any DriverMissing beats any Pending/LiveOnly beats all Applied; empty snapshot = `none`.
  LiveOnly counts as `pending` (not yet persisted).
- `list_controllers` returns a JSON array of `{ "name": <display name>, "mac": <mac>, "state": <human label> }`.
  Human labels: "Applied", "Fix available", "Live only", "Driver missing".

`TargetStatus` gains `name` (the fix entry's device name).
`is_supported_action` covers both queries so CLI dispatch keeps parity.

## Testing

- Table-driven tests for the aggregate state function across snapshot combinations.
- Payload shape tests for both queries.
- Existing dispatch test extended with the two query actions.

## Non-goals

Custom HTML/JS page, per-pad actions, config-file editing UI, firmware update helper.
