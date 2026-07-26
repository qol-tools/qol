# Shared Library Candidates

Reference list for Rust code that is worth lifting into `libs/qol-*` when reuse pressure justifies it.

## App inventory and desktop integration

Target: `libs/qol-apps`

Status: shared.

Shared app metadata, Linux desktop entry parsing/formatting, macOS bundle and Spotlight inventory, and launcher root scanning serve launcher discovery, tray launcher export, installer desktop files, media app selection, and app removal inventory. Consumer-specific ranking, response shaping, protection, and removal policy stay with each app or plugin.

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

## Process lifecycle

Target: `libs/qol-process`

Status: shared.

PID liveness, signaling, attached-child waiting, graceful termination escalation, process-group shutdown, and child reaping are shared across the tray, dev CLI, and plugins. Process ownership policy and caller-specific logging stay at each integration point.

## Live terminal sessions

Target: `libs/qol-terminal-sessions`

Status: shared.

CLI Sessions and Voice independently consume backend-neutral terminal identity, live discovery, screen reading, focus, validated text input, and an extensible CLI-session interpreter. The interpreter provides a generic fallback plus registered tool-specific enrichment for Codex, Claude, and future tools; consumers do not reproduce process detection or semantic session naming. Backend adapters, typed transport errors, and CLI interpretation belong in the library. Attention state, screen-state policy, notifications, and persistence remain in CLI Sessions; recognition, target selection, delivery policy, and conversational routing remain in Voice. Neither plugin brokers the other.

## Doctor report shape

Target: converge on `libs/qol-headless`

Status: do not create a new crate yet.

Tray doctor and plugin doctor output overlap with the existing `qol-headless` report types. Prefer converging on that crate instead of adding a separate doctor framework.
