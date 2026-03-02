# Architecture

## Runtime Flow

1. QoL Tray triggers `alt-tab --show` (or action `open`).
2. If daemon is already alive, the command is forwarded over local socket.
3. Daemon receives `Show` and calls `open_picker()`.
4. Picker checks prewarm preview cache — cached previews are used instantly.
5. Only missing previews are captured synchronously via CG/X11.
6. App icons are fetched asynchronously and pushed to the UI.
7. UI opens with full previews and icons in <50ms (warm path).
8. On macOS with SC: background streams are already running at 5fps; picker promotes selected+hovered to 30fps for live preview.

## Core Components

### `src/main.rs`

- App entrypoint: GPUI init, daemon socket bind, command dispatch.

### `src/app/mod.rs`

- `AltTabApp` struct: owns delegate, focus handle, action mode, alt poll task.
- `new()`: creates delegate, starts alt-poll if hold-to-switch, spawns live preview task.
- `apply_cached_windows()`: hot-updates window list and previews on reuse path.

### `src/app/render.rs`

- `Render` impl for `AltTabApp`: grid layout, card styling, icon + label rendering.
- Transparent background mode: conditional header, card bg with configurable color/opacity.

### `src/app/input.rs`

- Keyboard event handling: arrow navigation, tab cycling, enter/escape actions.

### `src/app/alt_poll.rs`

- Alt key release polling for hold-to-switch mode.

### `src/app/live_preview/mod.rs`

- Spawn dispatcher: picks SC or CG loop based on platform capability.
- Shared constants: `SC_POLL_INTERVAL_MS`, `LIVE_PREVIEW_INTERVAL_MS`.

### `src/app/live_preview/sc.rs`

- SC Spotlight loop: `activate`/`deactivate`/`sync_promoted`, surface updates, notify throttling.
- `StreamState`: tracks active flag and `promoted: [Option<u32>; 2]` (selected + hovered at 30fps).
- `CgState`: CG bridge capture scheduling for gap-filling when SC frames are missing.
- Stall detection: falls back to CG-only if SC stops delivering frames.

### `src/app/live_preview/cg.rs`

- CG fallback loop: `capture_cg` + `build_targets` round-robin for macOS <14 or SC permission denied.

### `src/app/live_preview/perf.rs`

- `PerfCounters`: ticks, frames, notify, skip, CPU usage (getrusage). Logged every 2s when visible.

### `src/delegate/mod.rs`

- `WindowDelegate`: owns window list, selection state, hover state, label config, preview/icon/surface caches.

### `src/delegate/selection.rs`

- Grid-aware selection navigation.

### `src/delegate/activation.rs`

- Window activation: calls `platform::activate_window`, pushes SET_FOCUS to runtime.

### `src/picker/mod.rs`

- `open_picker()`: the main entry point for showing the picker.
- Handles reuse path (same window, update data) and fresh-open path.
- Builds icon cache, resolves card bg config, manages transparent window options.

### `src/picker/reuse.rs`

- `try_reuse`: reuses existing picker window, repositions on monitor change via NSWindow API.

### `src/picker/create.rs`

- `create_new`: creates a fresh picker window when reuse is not possible.

### `src/picker/keepalive.rs`

- Hidden 1x1 PopUp window that prevents GPUI from quitting when picker is dismissed.

### `src/picker/run.rs`

- Daemon run loop: socket listener, command dispatch, prewarm scheduling.
- Prewarm loop: starts persistent 5fps SC streams in background, refreshes window/icon caches every 1.2s.
- Stream restart only when target window set actually changes (`prev_stream_wids` tracking).

### `src/config.rs`

- Config discovery/loading from install-scoped paths.
- `DisplayConfig`, `LabelConfig`, `ActionMode`, `OpenBehavior` types.

### `src/layout.rs`

- Sizing/grid math constants + functions (`picker_dimensions`, grid card sizes).

### `src/icon.rs`

- `build_icon_cache()`: converts raw BGRA icon data to `Arc<RenderImage>` keyed by app name.

### `src/window_source.rs`

- `preview_tile()`: renders a preview image or placeholder fallback for a grid card.

### `src/preview.rs`

- `bgra_to_render_image()`: converts raw BGRA bytes to `Arc<RenderImage>` via image crate.

### `src/daemon.rs`

- Socket endpoint and command dispatch (Show/ShowReverse/Kill/Ping).

### `src/platform/mod.rs`

- Platform facade: cross-platform contract for all OS-specific operations.
- `get_open_windows`, `capture_previews_cg`, `activate_window`, `get_app_icons`, SC stream management, etc.

### `src/platform/macos/`

- 9 modules: `sc/` (ScreenCaptureKit streams), `capture.rs` (CG bitmap capture), `ax.rs` (Accessibility API), `window_list.rs` (CG window enumeration + parsing), `window_enum.rs` (dedup + filtering logic), `window_actions.rs` (activate/close/quit/minimize), `picker.rs` (NSWindow helpers), `process.rs` (process info), `mod.rs` (type definitions + FFI bindings).

### `src/platform/linux.rs`

- Linux/X11: window enumeration, `_NET_WM_ICON` icon extraction, modifier detection.

### `src/platform/cg_helpers.rs`

- Shared macOS CG dictionary helpers (CFString, dict accessors, rect parsing).

### `src/monitor/`

- `MonitorTracker`: queries qol-tray runtime via `PlatformStateClient` for active monitor.

### `ui/`

- Settings page (HTML/JS/CSS) served by qol-tray.

## Navigation Model

- Arrow keys move in visual grid space using runtime column count.
- Vertical moves preserve the current column when possible.
- `Tab`/`Shift+Tab` provide fast cyclic stepping.

## Performance Characteristics

### macOS (SC live preview — always-on streams)
- Prewarm loop starts persistent 5fps SC streams in background; picker show/hide only promotes/demotes.
- First live frame on open: instant (streams already warm).
- Idle/background: ~0% CPU (heartbeat only).
- Visible, low activity: 4-6% CPU.
- Visible, cycling through windows: 7-9% CPU.
- Visible, heavy hover (2 promoted at 30fps): 11-13% CPU.
- Notify throttled to 100ms (~10fps GPUI re-render rate).
- CG skipped on open for windows covered by prewarm cache.

### General
- Picker open uses cached previews instantly; only missing windows are captured synchronously.
- App icons are fetched asynchronously after picker opens (~50ms).
- Window reuse path avoids GPU window recreation cost.
- Picker repositions without close/reopen on monitor change.
