# Launcher Active-Monitor Reuse — Handoff

## Current status

The active-monitor open behavior now works with fast window reuse.

- Focus monitor is detected by X11 event-driven cache in `plugin-launcher: src/monitor.rs`
- Daemon snapshots monitor on `show` and sends `Command::Show(Option<ActiveMonitor>)` in `plugin-launcher: src/daemon.rs`
- Launcher no longer does X11 reposition attempts for reused windows
- Launcher reuses a cached window per monitor target and activates the correct one in `plugin-launcher: src/launcher_app/mod.rs`

## What changed

### 1) Reuse strategy changed

Instead of reusing one window and trying to move it, launcher now:

- Derives `LauncherTarget` from monitor snapshot (`Default` or monitor-bounds key)
- Maintains `ActiveLaunchers` map: `LauncherTarget -> WindowHandle<LauncherView>`
- On each show:
  - minimizes non-target launcher windows
  - activates existing target window if present
  - creates target window once if missing

This avoids WM-dependent repositioning behavior and keeps show-path fast.

### 2) State reset on reuse

On reuse, `LauncherView::reset_for_show()` resets UI state (`LauncherState::new()`), then focus/activation is applied.

### 3) Compile fix

Monitor key generation uses public `Pixels` conversion methods (`to_f64()`) instead of private tuple fields.

## Architecture quality pass applied

`plugin-launcher: src/launcher_app/mod.rs` was refactored to keep boundaries explicit:

- `LauncherTarget::from_snapshot(...)` encapsulates monitor-to-target mapping
- `ActiveLaunchers` encapsulates registry operations:
  - `existing`
  - `insert`
  - `remove`
  - `hide_non_target`

No shallow wrappers were added; orchestration remains in `activate_or_open_launcher`.

## Known tradeoffs / limitations

- A window is cached per monitor target, so multi-monitor usage can retain multiple minimized launcher windows over time
- Target identity is based on monitor bounds; if monitor layout/resolution changes while running, new targets may be created
- Debug `eprintln!` logs remain in monitor/daemon/launcher paths

## Suggested next follow-ups (optional)

1. Add lifecycle policy for stale target windows after display topology changes
2. Reduce debug logging once behavior is stable
3. If needed, cap cached launcher windows (e.g., remove least-recent target)

## Key files

- `plugin-launcher: src/monitor.rs`
- `plugin-launcher: src/daemon.rs`
- `plugin-launcher: src/launcher_app/mod.rs`
