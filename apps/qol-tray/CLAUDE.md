# qol-tray (Always-loaded summary)

Loaded for every session in this repo. Brief invariants only - depth lives in the skills cited.

## Mission non-negotiables

qol-tray is a portable QoL layer that makes any computer feel like the user's own, then leaves it exactly as found. Decisions that fight these are wrong, not the mission:

1. **The user never configures the host OS.** No editing GNOME/Cinnamon shortcuts, plist files, registry keys, systemd units. If qol-tray needs the OS to behave differently, qol-tray makes it happen silently and reversibly.
2. **qol-tray owns its surface area.** Hotkeys, tray icon, autostart, menu entries qol-tray claims are qol-tray's source of truth. If a DE has grabbed a hotkey, qol-tray takes it back.
3. **Host left exactly as found.** On exit (clean, crash, USB pull) every change is reversed. Files outside the profile dir were either never written or get cleaned up.
4. **Plug-in to working in single-digit seconds.** Slower than that is a bug.

Depth + the vision context: `qol-project:qol-mission`.

## Cross-platform: warnings are errors

Code that compiles green on Linux frequently breaks macOS/Windows under `-D warnings` because `dead_code`, `unused_imports`, `unused_mut` differ per backend. Before adding a `pub fn`, `pub const`, `use`, or `#[cfg(target_os)]` to a shared module, check what *every* backend will actually consume. CI runs `RUSTFLAGS=-D warnings` everywhere; this gate is not optional.

Workflow + symbol-hygiene patterns: `qol-project:qol-arch-cross-platform`. Strategy-pattern code layout: `qol-project:qol-arch-code`.

## "Did my change actually load?"

When in doubt, hit the in-app **Recompile** button. It kills every plugin daemon AND replaces the qol-tray process in-place. `cargo build` alone leaves a stale running process; restarting the binary alone leaves stale plugin daemons. Recompile is the only single action that does both.

Depth: `qol-tray:qol-tray-dev-recompile`.

## Where else to look

| Working on... | Skill |
| --- | --- |
| Rust backend (`src/`) | `qol-tray:qol-tray-rust`, `qol-langs:rust-conventions` |
| Preact UI (`ui/`) | `qol-tray:qol-tray-ui-systems`, `qol-langs:preact-conventions` |
| Profile / sync feature | `qol-tray:qol-tray-feature-profile` |
| World canvas / dive UI | `qol-tray:qol-world-canvas` |
| Tests anywhere | `qol-tray:qol-apps-testing` |
| Diagnostic logging in UI | `qol-tray:qol-tray-dev-logging` |
| Release | `qol-tray:qol-tray-release-flow` |
