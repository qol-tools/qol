# Launcher Active-Monitor Reuse — Handoff

## Current status

The active-monitor open behavior now works with fast window reuse on both Linux and macOS.

- Active monitor tracks cursor settling and focus changes with timestamps; most recent signal wins
- Launcher reuses a cached window per monitor target and activates the correct one
- macOS: app hidden from Cmd+Tab via `NSApplicationActivationPolicyAccessory`
- macOS: launcher window is non-movable and non-draggable

## Architecture

### Monitor detection

Platform-specific code lives in `src/monitor/platform/linux.rs` and `src/monitor/platform/macos.rs`. `src/monitor/platform/mod.rs` dispatches via `#[cfg(target_os)]`. The `PlatformQueries` trait abstracts platform capabilities so core logic stays platform-agnostic (e.g. `poll_focused_window()` lets each platform declare what is safe to call from a background thread).

- **Background poller** (`src/monitor/tracker.rs`): runs on both platforms. Tracks cursor position continuously. Tracks focused window when safe (see below). Both cursor and focus use "skip same-monitor" semantics — timestamps only update when the signal actually changes monitors, so `pick_active_monitor` correctly reflects which input changed most recently.
- **Linux**: `poll_focused_window()` returns `true` — focus is always polled from the background thread.
- **macOS**: `poll_focused_window()` returns `false` — focus is only polled when `any_visible` is false (no windows rendering), avoiding `CGWindowListCopyWindowInfo` deadlocks with AppKit. Focus is also freshened on-demand in `snapshot()`.

### Active monitor selection

`snapshot()` clones the background-polled state, freshens only cursor position from a live query (CGEventCreate — always thread-safe), then calls `pick_active_monitor` which picks whichever signal (cursor or focus) changed monitors most recently. Focus is **not** queried on-demand in `snapshot()` — it comes exclusively from the background poller, which tracks it with proper timestamps when safe (`poll_focused_window()` on Linux, or when `!any_visible` on macOS). Both `update_cursor` and `update_focus` skip same-monitor updates, preserving the timestamp from when the signal actually transitioned.

### Window reuse strategy

- Derives `LauncherTarget` from monitor snapshot (`Default` or monitor-bounds key)
- Maintains `ActiveLaunchers` map: `LauncherTarget -> WindowHandle<LauncherView>`
- On each show:
  - snapshot monitor to determine target
  - if a launcher is already visible on the **same** target monitor, re-activate it
  - if the target is a **different** monitor, close the old launcher and open on the new target
  - otherwise: hide non-target launchers, activate existing or create new

### Visibility tracking

`any_visible: Arc<AtomicBool>` is shared between the command poll loop, all `LauncherView` instances, and the background poller. Updated via `LauncherView::set_showing()` on every show/hide transition. The poller uses it to determine when focus polling is safe on macOS.

### macOS window behavior

- `WindowKind::PopUp` with `is_movable: false` prevents dragging
- `NSApplicationActivationPolicyAccessory` set at startup hides from Cmd+Tab and Dock
- Both set in `src/launcher_app/mod.rs`

## Known tradeoffs / limitations

- A window is cached per monitor target; multi-monitor usage retains multiple hidden launcher windows
- Target identity is based on monitor bounds; display topology changes create new targets
- Debug `eprintln!` logs remain gated behind `#[cfg(debug_assertions)]`
- macOS accessory policy means the app can't own the menu bar; focus loss edge cases possible
- On macOS, focus changes while the launcher is visible are not tracked (to avoid CG deadlocks); they are picked up once the launcher hides

## Key files

- `plugin-launcher: src/monitor/platform/mod.rs` — `PlatformQueries` trait, platform dispatch
- `plugin-launcher: src/monitor/platform/linux.rs` — X11 cursor, focus, and monitor queries
- `plugin-launcher: src/monitor/platform/macos.rs` — CoreGraphics cursor, focus, and monitor queries
- `plugin-launcher: src/monitor/tracker.rs` — background poller, adaptive polling, snapshot
- `plugin-launcher: src/monitor/state.rs` — `InputState`, `pick_active_monitor`, timestamp logic
- `plugin-launcher: src/launcher_app/mod.rs` — window lifecycle, command poll, activation policy
- `plugin-launcher: src/daemon.rs` — Unix socket IPC
