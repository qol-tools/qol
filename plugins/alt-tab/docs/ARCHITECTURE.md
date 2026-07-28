# Architecture

Alt Tab is a long-lived GPUI picker with feature-owned platform adapters. The
Rust source root contains only the process entrypoint; each directory owns one
cohesive capability.

## Source map

```text
src/
├── main.rs                    # module wiring and process entrypoint
├── runtime/                   # argument routing and daemon transport
├── config/                    # typed qol-config contract
├── app/                       # GPUI view, input, rendering, live previews
│   └── live_preview/          # capture loop, first-fill gate, lane scheduler
├── picker/                    # retained-window orchestration
│   ├── state.rs               # selection and retained presentation state
│   ├── layout.rs              # card/grid geometry
│   ├── monitor_listener/      # monitor/data refresh routing
│   └── platform/              # retained-window behavior per OS
├── discovery/                 # window identity, ordering, and enumeration
├── capture/                   # preview capture and live-frame adapters
├── actions/                   # activate, close, quit, and minimize adapters
├── preview_plane/             # optional compositor-owned live previews
└── rendering/                 # render flow, image conversion/lifetime, traces
```

Every feature-owned `platform/` directory performs its own target selection in
`platform/mod.rs`. OS-specific code stays below that boundary. A platform
directory uses either all OS files or all OS directories, never a mixture.

## Runtime flow

1. QoL Tray invokes the plugin with a show, reverse-show, settings, or kill
   argument.
2. `runtime/` forwards the command to the existing daemon when possible.
3. The daemon owns GPUI initialization, a keepalive surface, retained picker
   windows, monitor tracking, and the command loop.
4. A show request reloads configuration, refreshes window metadata, selects a
   monitor placement, and either cycles or updates the retained picker.
5. `picker/` gathers cached images immediately and schedules missing icon or
   preview work through the relevant capability.
6. `app/` owns input, selection presentation, dismissal, and live preview
   updates until the picker is hidden again.

## Ownership boundaries

### Runtime and configuration

- `main.rs` only declares top-level modules and delegates arguments.
- `runtime/mod.rs` owns command routing, daemon startup, and browser settings
  fallback.
- `runtime/daemon.rs` owns the socket command protocol.
- `config/mod.rs` owns contract-backed settings types and defaults.

### GPUI application

- `app/mod.rs` owns `AltTabApp`, focus/dismissal state, and retained-view
  updates.
- `app/input.rs` owns keyboard navigation and explicit actions.
- `app/render.rs` owns GPUI elements and card presentation.
- `app/live_preview/` owns live capture coordination. Its first-fill gate and
  lane scheduler are private implementation details, not crate-wide helpers.

### Picker orchestration

- `picker/mod.rs` owns the show/reuse/create decision and picker cache types.
- `picker/state.rs` owns window selection and retained preview/icon state.
- `picker/layout.rs` is the single source of truth for card and grid geometry.
- `picker/gather.rs` owns discovery-to-presentation gathering and async fills.
- `picker/create.rs` and `picker/reuse.rs` own the two retained-window paths.
- `picker/run.rs` owns GPUI application startup and daemon command dispatch.
- `picker/monitor_listener/` owns topology and data-refresh events.
- `picker/platform/` owns compositor-specific picker window behavior.

### Discovery, capture, actions, and rendering

- `discovery/` returns stable window identity and ordering.
- `capture/` returns preview pixels or native live frames for those identities.
- `actions/` performs operations on another application window.
- `preview_plane/` integrates an external compositor preview surface when one
  is available.
- `rendering/` selects the preview renderer and owns GPUI image conversion,
  atlas lifetime accounting, and debug-only preview traces.

Discovery and capture remain separate even when one platform API can provide
both. Rendering owns image lifetime; every retained image cache must release
through the registry so a GPUI image ID is never dropped twice.

## Retained-window invariants

- The daemon initializes GPUI once and reuses retained picker windows.
- The keepalive surface must not appear as an empty desktop or Alt-Tab window.
- Every show reloads config and refreshes window metadata before presentation.
- Reuse reapplies monitor placement, bounds, transparency, shadow, focus, and
  first-frame reveal requirements.
- Dismissal hides the picker without terminating the daemon.
- Browser settings are only a fallback for the shared native settings panel.

## Navigation model

- Arrow keys move in the current visual grid.
- Vertical movement preserves the column when the destination row permits it.
- Tab and Shift-Tab cycle through the current window order.
- Hold-to-switch and sticky confirmation share one selection state machine.
