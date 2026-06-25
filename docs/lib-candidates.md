# Shared Library Candidates

Reference list for Rust code that is worth lifting into `libs/qol-*` when reuse pressure justifies it.

## App inventory and desktop integration

Target: `libs/qol-apps`

Status: started.

Shared app metadata, Linux desktop entry parsing/formatting, launcher inventory, and app export helpers are used by launcher discovery, tray launcher export, installer desktop files, media app selection, and app removal inventory.

## Hotkey grammar and input mapping

Target: `libs/qol-hotkeys`

Status: started.

Tray hotkeys and keyremap both parse user-facing key strings into backend-specific key codes. The stable seam is the grammar, canonical key model, and pure adapters for backend key-code spaces. Tray-specific registration, plugin action routing, and capture lifecycle should stay in `qol-tray`.

## Window discovery, activation, and restore

Target: `libs/qol-windowing`

Status: candidate.

Alt-tab and window-actions both model window identity, activation, geometry, minimize, restore, and platform backends. The first safe extraction is shared window identity/geometry plus pure restore logic, before moving X11 or macOS backend code.

## Zigbee and ZNP protocol stack

Target: `libs/qol-znp` or `libs/qol-zigbee`

Status: conditional.

The lights plugin has a mostly self-contained protocol stack for ZNP frames, transport, controller events, and ZCL payloads. Lift it if another device integration needs Zigbee primitives; otherwise keep it plugin-local.

## Dev workspace and plugin discovery

Target: `libs/qol-workspace` or `libs/qol-dev-core`

Status: candidate.

Tray dev mode and the `qol` CLI both reason about workspace roots, plugin source discovery, Cargo buildability, and fingerprint inputs. The shared seam is workspace/plugin scanning and build fingerprinting, not UI or process orchestration.

## CLI session domain

Target: `libs/qol-cli-sessions`

Status: conditional.

The CLI sessions plugin has clean domain boundaries around panes, terminal hosts, strategies, and registry state. Lift it only when another surface consumes the same live session model.

## Doctor report shape

Target: converge on `libs/qol-headless`

Status: do not create a new crate yet.

Tray doctor and plugin doctor output overlap with the existing `qol-headless` report types. Prefer converging on that crate instead of adding a separate doctor framework.
