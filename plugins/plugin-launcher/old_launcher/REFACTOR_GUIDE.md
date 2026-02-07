# Refactoring Guide: 16_multi_monitor.rs

## Current State

The file is ~1450 lines with platform-specific code mixed throughout via `#[cfg]` attributes. This violates the principle of isolating platform differences at the abstraction layer.

## Target Structure

```
src/
├── bin/
│   └── 16_multi_monitor.rs    # UI only, ~400 lines
└── lib.rs                      # re-exports

src/platform/
├── mod.rs                      # Platform trait definitions
├── linux.rs                    # Linux implementations
├── macos.rs                    # macOS implementations
└── unsupported.rs              # Fallback stubs
```

## Platform Abstraction Layer

### Define traits in `platform/mod.rs`:

```rust
pub struct FocusSnapshot {
    pub window_id: Option<u64>,
    pub bounds: Bounds<Pixels>,
}

pub struct BackendStatus {
    pub summary: String,
    pub ok: bool,
}

pub trait HotkeyBackend: Send + 'static {
    fn receiver(&self) -> &mpsc::Receiver<()>;
}

pub trait FocusBackend {
    fn poll(&mut self) -> (Option<FocusSnapshot>, BackendStatus);
}

pub trait ClickBackend {
    fn poll(&mut self) -> (Option<Point<Pixels>>, BackendStatus);
}

pub trait DisplayBackend {
    fn get_display_bounds(displays: &[Rc<dyn PlatformDisplay>]) -> Vec<DisplayInfo>;
}
```

### Platform modules implement traits:

**linux.rs**: X11 hotkey grab, xdotool focus, xinput clicks
**macos.rs**: CGEventTap hotkey, AppleScript focus, CoreGraphics display bounds
**unsupported.rs**: Return error statuses

### Conditional compilation at module level:

```rust
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use unsupported::*;
```

## Shared Logic to Extract

These functions have no platform-specific code:

- `resolve_active_display`
- `display_union_from_infos`
- `display_for_point_from_infos`
- `focused_display_from_infos`
- `intersection_area`
- `bounds_area`
- `scale_bounds`
- `map_point`
- `px_i64`

Move to `src/geometry.rs` or similar.

## Property Tests to Add

### Display geometry:

```rust
fn prop_union_contains_all_displays(displays: Vec<DisplayInfo>)
fn prop_point_in_display_finds_correct_one(displays: Vec<DisplayInfo>, point: Point)
fn prop_focused_display_has_max_overlap(window: Bounds, displays: Vec<DisplayInfo>)
```

### Active display resolution:

```rust
fn prop_most_recent_event_wins(focus_time: Option<Instant>, click_time: Option<Instant>)
fn prop_returns_none_when_no_events()
```

### Bounds scaling:

```rust
fn prop_scaled_bounds_fit_in_map(bounds: Bounds, union: Bounds, map_size: Size)
fn prop_scaling_preserves_relative_positions(bounds_a: Bounds, bounds_b: Bounds)
```

## UI Component: Simplified Main File

After extraction, `16_multi_monitor.rs` contains only:

1. `MultiMonitorView` struct and impl
2. `LauncherPopup` struct and impl
3. `open_launcher_popup_at` function
4. `main` function
5. Action definitions

All platform detection, polling, and geometry math imported from modules.

## Migration Steps

1. Create `src/platform/mod.rs` with trait definitions
2. Move Linux code to `src/platform/linux.rs`
3. Move macOS code to `src/platform/macos.rs`
4. Create `src/platform/unsupported.rs` with stubs
5. Extract geometry functions to `src/geometry.rs`
6. Add property tests for geometry module
7. Update `16_multi_monitor.rs` to use imports
8. Remove all `#[cfg]` from main file

## Notes

- Keep FFI declarations inside platform modules (not in shared code)
- Each platform module is self-contained
- Main file has zero conditional compilation
- Property tests cover pure logic only (no gpui context needed)
