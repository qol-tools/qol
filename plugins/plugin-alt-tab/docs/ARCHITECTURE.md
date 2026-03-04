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
- Shared type aliases: `PreviewMap`, `IconMap`, `SharedPreviewCache`, `SharedIconCache`, `PickerWindowState`.

### `src/app/mod.rs`

- `AltTabApp` struct: owns delegate, focus handle, action mode, alt poll task.
- `new()`: creates delegate, starts alt-poll if hold-to-switch, spawns live preview task.
- `apply_cached_windows()`: hot-updates window list and previews on reuse path.
- Alt key release polling for hold-to-switch mode (inlined, no separate module).

### `src/app/render.rs`

- `Render` impl for `AltTabApp`: grid layout via `render_grid`, card styling via `render_card` + `card_bg`.
- Transparent background mode: conditional header, card bg with configurable color/opacity.
- `RenderSnapshot`: captures delegate state for render functions.
- Extracted helpers: `header_bar`, `render_preview`, `render_label`, `preview_tile`, `placeholder_frame`.

### `src/app/input.rs`

- Keyboard event handling: arrow navigation, tab cycling, enter/escape actions.

### `src/app/live_preview.rs`

- CG live preview loop: captures selected window + one round-robin window every 500ms.
- Pipeline: `wait_for_visible` → `read_snapshot` → `pick_targets` → `run_capture` → `diff_captures` → `push_updates`.
- `LoopState` tracks previous hashes and round-robin position.

### `src/picker/mod.rs`

- `open_picker()`: the main entry point for showing the picker.
- Handles reuse path (same window, update data) and fresh-open path.
- `PickerState` (inline `mod state`): owns window list, selection, hover, label config, preview/icon/surface caches.

### `src/picker/gather.rs`

- Window gathering, preview capture, icon fill.
- `build_icon_cache()`: converts raw BGRA icon data to `Arc<RenderImage>` keyed by app name.

### `src/picker/reuse.rs`

- `try_reuse`: reuses existing picker window, repositions on monitor change via NSWindow API.

### `src/picker/create.rs`

- `create_new`: creates a fresh picker window when reuse is not possible.

### `src/picker/run.rs`

- Daemon run loop: socket listener, command dispatch, prewarm scheduling.
- Prewarm loop: refreshes window/icon caches periodically.

### `src/config.rs`

- Config discovery/loading from install-scoped paths.
- `DisplayConfig`, `LabelConfig`, `ActionMode`, `OpenBehavior` types.

### `src/shared/layout.rs`

- Sizing/grid math constants + functions (`picker_dimensions`, grid card sizes).

### `src/shared/preview.rs`

- `bgra_to_render_image()`: converts raw BGRA bytes to `Arc<RenderImage>` via image crate.
- `fast_pixel_hash()`: cheap hash for change detection in live preview loop.

### `src/daemon.rs`

- Socket endpoint and command dispatch (Show/ShowReverse/Kill/Ping).

### `src/discovery/platform/mod.rs`

- Platform facade: `get_open_windows`, `get_on_screen_windows`, `on_screen_window_ids`.

### `src/discovery/platform/macos/`

- `mod.rs` — CG window list parsing (`CgWindow`, `CgKeys` with RAII Drop, `fetch_cg_windows`, `parse_cg_entry`), orchestration (`get_open_windows`, `get_on_screen_windows`).
- `ffi.rs` — CG/CF type aliases, extern blocks, constants, dictionary helpers (`cfstr`, `dict_get_*`, `cfstring_to_string`).
- `ax.rs` — AX window queries (`ax_windows`, `ax_find_window`, `ax_is_window_minimized`), dedup logic (`dedup_by_ax`). Uses `AxAttrs` with RAII Drop for attribute keys.
- `window_enum.rs` — Window enumeration pipeline: `WindowEnumeration` state, `KnownWindowTracker` persistence, `collect_on_screen_windows` + `collect_minimized_windows` with budget-based filtering (`BudgetContext`, `AxData`).
- `process.rs` — Process identity helpers, regular app detection, known window ID cache.

### `src/discovery/platform/linux.rs`

- Linux/X11: window enumeration via pipelined X11 property queries, `_NET_WM_ICON` icon extraction.

### `src/capture/platform/mod.rs`

- Platform facade: `capture_previews_cg`, `get_app_icons`.

### `src/capture/platform/macos.rs`

- `CGWindowListCreateImage` capture, `BlitSource` + `ScaledRect` for scaled BGRA blitting.
- `NSImage` icon extraction via `CGBitmapContext`.

### `src/capture/platform/linux.rs`

- X11 `GetImage` capture.

### `src/actions/platform/mod.rs`

- Platform facade: `activate_window`, `close_window`, `quit_app`, `minimize_window_by_id`.

### `src/actions/platform/macos.rs`

- AX-based window actions (raise, unminimize, close, minimize).
- `NSRunningApplication` for app activation/termination.
- `cg_window_pid_and_title` for CG→AX window lookup.

### `src/actions/platform/linux.rs`

- X11 window activation.

### `src/picker/platform/`

- `mod.rs` — Facade: `dismiss_picker`, `is_modifier_held`.
- `macos.rs` — NSWindow resize-to-1x1, `CGEventSourceFlagsState` modifier check.
- `linux.rs` — Minimize, X11 modifier check.

### `ui/`

- Settings page (HTML/JS/CSS) served by qol-tray.

## Navigation Model

- Arrow keys move in visual grid space using runtime column count.
- Vertical moves preserve the current column when possible.
- `Tab`/`Shift+Tab` provide fast cyclic stepping.

## Performance Characteristics

- Picker open uses cached previews instantly; only missing windows are captured synchronously.
- App icons are fetched asynchronously after picker opens (~50ms).
- Window reuse path avoids GPU window recreation cost.
- Picker repositions without close/reopen on monitor change.
- CG live loop: selected + 1 round-robin every 500ms, diff-based updates only.
