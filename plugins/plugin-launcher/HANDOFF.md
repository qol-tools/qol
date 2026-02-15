# Launcher Active-Monitor Reuse — Handoff

## Current status

The active-monitor open behavior now works with fast window reuse on both Linux and macOS.

- Linux: focus monitor detected by X11 event-driven cache in `plugin-launcher: src/monitor/linux.rs`
- macOS: focus monitor detected by on-demand CoreGraphics query in `plugin-launcher: src/monitor/macos.rs`
- Launcher reuses a cached window per monitor target and activates the correct one in `plugin-launcher: src/launcher_app/mod.rs`
- macOS: app hidden from Cmd+Tab via `NSApplicationActivationPolicyAccessory`
- macOS: launcher window is non-movable and non-draggable

## Architecture

### Monitor detection

Platform-specific code lives in `src/monitor/linux.rs` and `src/monitor/macos.rs`. `src/monitor/mod.rs` dispatches via `#[cfg(target_os)]`.

- **Linux**: background event loop on X11 (`_NET_ACTIVE_WINDOW` property changes, XInput2 key/button events, pointer polling with settle delay). Event-driven, not polling.
- **macOS**: no background thread. Single synchronous `CGWindowListCopyWindowInfo` call at snapshot time via `background_spawn`. This avoids CG/AppKit main-thread deadlocks (see lessons learned below).

### Window reuse strategy

- Derives `LauncherTarget` from monitor snapshot (`Default` or monitor-bounds key)
- Maintains `ActiveLaunchers` map: `LauncherTarget -> WindowHandle<LauncherView>`
- On each show:
  - if a launcher is already visible (`any_visible` AtomicBool), skip snapshot and just activate
  - otherwise: snapshot monitor, hide non-target launchers, activate existing or create new

### Visibility tracking

`any_visible: Arc<AtomicBool>` is shared between the command poll loop and all `LauncherView` instances. Updated via `LauncherView::set_showing()` on every show/hide transition. This allows the async command loop to skip expensive CG calls and `cx.update` round-trips when the launcher is already visible.

### macOS window behavior

- `WindowKind::PopUp` with `is_movable: false` prevents dragging
- `NSApplicationActivationPolicyAccessory` set at startup hides from Cmd+Tab and Dock
- Both set in `src/launcher_app/mod.rs`

## Known tradeoffs / limitations

- A window is cached per monitor target; multi-monitor usage retains multiple hidden launcher windows
- Target identity is based on monitor bounds; display topology changes create new targets
- Debug `eprintln!` logs remain gated behind `#[cfg(debug_assertions)]`
- macOS accessory policy means the app can't own the menu bar; focus loss edge cases possible

## Key files

- `plugin-launcher: src/monitor/mod.rs` — shared logic, FocusCache, dispatch
- `plugin-launcher: src/monitor/linux.rs` — X11 event-driven focus tracking
- `plugin-launcher: src/monitor/macos.rs` — CoreGraphics on-demand focus detection
- `plugin-launcher: src/launcher_app/mod.rs` — window lifecycle, command poll, activation policy
- `plugin-launcher: src/daemon.rs` — Unix socket IPC
