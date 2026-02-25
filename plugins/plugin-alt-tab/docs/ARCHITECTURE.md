# Architecture

## Runtime Flow

1. QoL Tray triggers `alt-tab --show` (or action `open`).
2. If daemon is already alive, the command is forwarded over local socket.
3. Daemon receives `Show` and calls `open_picker()`.
4. Picker checks prewarm preview cache — cached previews are used instantly.
5. Only missing previews are captured synchronously via CG/X11.
6. App icons are fetched asynchronously and pushed to the UI.
7. UI opens with full previews and icons in <50ms (warm path).

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

### `src/app/live_preview.rs`

- Background task that periodically refreshes window previews while picker is visible.

### `src/delegate/mod.rs`

- `WindowDelegate`: owns window list, selection state, label config, preview/icon caches.
- Selection logic: `select_next`, `select_prev`, grid-aware arrow navigation.

### `src/delegate/activation.rs`

- Window activation: calls `platform::activate_window`, pushes SET_FOCUS to runtime.

### `src/picker/mod.rs`

- `open_picker()`: the main entry point for showing the picker.
- Handles reuse path (same window, update data) and fresh-open path.
- Builds icon cache, resolves card bg config, manages transparent window options.

### `src/picker/keepalive.rs`

- Hidden 1x1 PopUp window that prevents GPUI from quitting when picker is dismissed.

### `src/picker/run.rs`

- Daemon run loop: socket listener, command dispatch, prewarm scheduling.

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
- `get_open_windows`, `capture_previews_cg`, `activate_window`, `get_app_icons`, `disable_window_shadow`, etc.

### `src/platform/macos.rs`

- macOS: CG window list, CG bitmap preview capture, NSRunningApplication activation.
- App icon extraction via CGBitmapContext (draws NSImage into known BGRA format).
- `disable_window_shadow`: iterates NSApplication windows, clears shadow + background.

### `src/platform/linux.rs`

- Linux/X11: window enumeration, `_NET_WM_ICON` icon extraction, modifier detection.

### `src/platform/cg_helpers.rs`

- Shared macOS CG dictionary helpers (used by both platform/macos.rs and monitor/).

### `src/monitor/`

- `MonitorTracker`: queries qol-tray runtime via `PlatformStateClient` for active monitor.

### `ui/`

- Settings page (HTML/JS/CSS) served by qol-tray.

## Navigation Model

- Arrow keys move in visual grid space using runtime column count.
- Vertical moves preserve the current column when possible.
- `Tab`/`Shift+Tab` provide fast cyclic stepping.

## Performance Characteristics

- Prewarm cache captures previews in background between invocations.
- Picker open uses cached previews instantly; only missing windows are captured synchronously.
- App icons are fetched asynchronously after picker opens (~50ms).
- Live preview task refreshes thumbnails while picker is visible.
- Window reuse path avoids GPU window recreation cost.
