
## 2026-06-04
- In qol-tray runtime, focus stamp can outrace cursor stamp because keyboard-only Alt+Tab leaves `cursor_moved=false` (CursorChannel needs >20px), so `update_cursor` short-circuits and `pick_active_monitor` lets focus win.
- `is_own_window` filter in `desktop_state/platform/linux.rs` only matches qol-tray's own pid; plugin daemons (e.g. plugin-alt-tab picker) are separate processes, so their windows DO show up as `focused_window_bounds()`.
- `refresh_focus_synchronously` only restamps `input.focus.at` when the focus monitor actually changes, not on every GET_STATE — so a stale focus monitor keeps its old `Instant` until OS focus moves.
