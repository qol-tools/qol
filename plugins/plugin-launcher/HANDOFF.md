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

## macOS multi-monitor fix

Two bugs were present on macOS multi-monitor setups. Both worked correctly on Linux.

### Bug 1: Launcher always opened on primary monitor

**Root cause**: `FocusCache::start()` only called `poll_focus_once()` once at startup on macOS. There was no continuous monitoring thread (unlike Linux which has `x11_focus_listener`). When `snapshot()` was called on a "show" command, it returned stale focus data from daemon startup — always pointing to whatever monitor was active at launch time.

**Fix**: Replaced the one-shot `poll_focus_once()` in the `#[cfg(target_os = "macos")]` block of `FocusCache::start()` with a background thread running `macos_focus_poller()`. This mirrors the Linux pattern: a dedicated thread continuously polls `poll_focus_once()` every 300ms (`MACOS_FOCUS_POLL_MS`) and updates the shared `InputState.focus` via the existing `Arc<Mutex<InputState>>`. The `snapshot()` call path remains zero-latency — it just reads the cached value.

On-demand polling in `snapshot()` was rejected because `osascript` takes 100–300ms per call, which adds unacceptable latency to the launcher show path.

### Bug 2: Window position shifted left and up

**Root cause**: `physical_monitors()` checked `cx.displays().len() > 1` first. On macOS, GPUI reports multiple displays correctly (unlike Linux where it sometimes doesn't), so it returned GPUI display bounds early. However, GPUI bounds on macOS use a different coordinate space than CoreGraphics — likely Retina-scaled logical coordinates. The `centered_bounds()` calculation used GPUI's space, but `WindowBounds::Windowed()` and `poll_focus_once()` (osascript) both operate in CG/AppKit point coordinates. This mismatch caused the off-center positioning.

**Fix**: Reordered `physical_monitors()` so the `#[cfg(target_os = "macos")]` `macos_display_bounds()` (CGDisplayBounds) check runs **before** the GPUI `displays().len() > 1` check. This ensures macOS multi-monitor always uses CG bounds, which are in the same coordinate space as the focus tracking and window positioning APIs. The GPUI check remains as a fallback for single-monitor macOS and for Linux (where GPUI may under-report display count).

### Changes in `src/monitor.rs`

- Added `MACOS_FOCUS_POLL_MS` constant (300ms)
- Added `macos_focus_poller()` function — background loop that polls focus and updates shared state
- Changed `FocusCache::start()` macOS block to spawn background thread instead of one-shot poll
- Reordered `physical_monitors()`: macOS CG → GPUI → Linux xrandr → GPUI fallback

### Verification

- `cargo build --bin launcher` compiles cleanly on macOS
- All 96 property tests pass (`cargo test`)

## Suggested next follow-ups (optional)

1. Add lifecycle policy for stale target windows after display topology changes
2. Reduce debug logging once behavior is stable
3. If needed, cap cached launcher windows (e.g., remove least-recent target)

## Key files

- `plugin-launcher: src/monitor.rs`
- `plugin-launcher: src/daemon.rs`
- `plugin-launcher: src/launcher_app/mod.rs`
