
## 2026-06-04
- On Linux X11, qol-tray's `is_ignored_pid` is gated `#[cfg(target_os = "macos")]` so the Linux focus query never consults it - daemon PIDs leak into `focused_window_bounds`.
- GPUI `WindowKind::PopUp` on X11 is NOT override-redirect; it sets `_NET_WM_WINDOW_TYPE_NOTIFICATION` and `activate_window()` sends `_NET_ACTIVE_WINDOW` + `set_input_focus`, so popups become the WM active window.
- Plugin daemons spawn via `command.spawn()` + `libc::setsid()` with a different PID than qol-tray, so `_NET_WM_PID == own_pid` checks won't catch picker windows.
