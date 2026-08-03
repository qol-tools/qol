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

Status: shared.

Alt-tab and window-actions both model window identity, activation, geometry, minimize, restore, and platform backends. The shared crate owns the normalized window identity (`WindowId`), geometry (`WindowRect`), and the `WindowOps` platform trait (enumerate, geometry, move/resize, focus, minimize, restore). Both plugins implement the trait on their platform backends; X11 backend code stays plugin-local behind the trait. Pure restore state logic and preview capture remain in the plugins.

## Zigbee and ZNP protocol stack

Target: `libs/qol-zigbee`

Status: shared.

The ZNP protocol stack (frame/coordinator/device/zcl layers, coordinator serial transport, request engine, controller events, and port probing) lives in the shared crate. Lifted from the lights plugin without a second consumer yet, per explicit user request; lights consumes the lib while its daemon action dispatch, domain types (color/brightness/colortemp), presets, pair flow, and config stay plugin-local. Serial-port enumeration and coordinator-vendor detection stay in the plugin's platform layer.

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

## Profile sync engine

Target: `libs/qol-profile-sync`

Status: shared.

The tray's `SyncService` and `qol sync` drive one profile store and one GitHub-backed repo; the shared crate owns the conflict model and field-level merge, the state-file and toggles formats, conflict backup naming, the sync allowlist, the git repo shape, and the cross-process sync lock (flock-style lockfile in the per-device sync state dir). Consumers stay thin adapters that resolve the config directory and pass the profile root in; tray-specific runtime-config guards and GitHub HTTP connect logic remain in the tray, CLI token loading stays in the CLI.

## Doctor report shape

Target: converge on `libs/qol-headless`

Status: do not create a new crate yet.

Tray doctor and plugin doctor output overlap with the existing `qol-headless` report types. Prefer converging on that crate instead of adding a separate doctor framework.
