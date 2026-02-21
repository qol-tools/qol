# Architecture

## Runtime Flow

1. QoL Tray triggers `alt-tab --show` (or action `open`).
2. If daemon is already alive, the command is forwarded over local socket.
3. Daemon receives `Show` and opens/reopens the picker window.
4. Picker opens immediately with estimated dimensions.
5. Window list is fetched in background.
6. UI updates with discovered windows.
7. Preview captures are dispatched per-window in parallel.
8. Each preview is processed and applied incrementally to the UI.

## Core Components

### `src/main.rs`

- Owns picker window lifecycle and rendering.
- Implements keyboard navigation and selection activation.
- Resizes picker height after real window count is known.
- Performs async orchestration for:
  - window discovery (`get_open_windows`)
  - per-window preview capture (`capture_preview`)

### `src/x11_utils.rs`

- Enumerates visible/normal windows from X11 properties.
- Captures raw window image data.
- Converts BGRX-like X11 buffers to RGBA.
- Downscales with aspect preservation and writes a fixed-size preview canvas.
- Persists previews to cache for GPUI image loading.

### `src/daemon.rs`

- Owns socket endpoint and command dispatch (`Show`/`Kill`).
- Prevents repeated process startup cost by keeping app hot.

### `src/monitor/`

- Tracks active monitor heuristics (cursor/focus).
- Provides bounds used to center picker on the likely active display.

## Navigation Model

- Arrow keys move in visual grid space using runtime column count.
- Vertical moves preserve the current column when possible.
- `Tab`/`Shift+Tab` provide fast cyclic stepping.

## Performance Characteristics

- No blocking full-window scan before opening picker.
- Preview generation is parallelized and streamed.
- UI repaints happen incrementally as previews arrive.
