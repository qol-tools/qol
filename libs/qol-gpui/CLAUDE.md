# qol-gpui

Shared GPUI helpers for qol-tray plugins: popup-window placement, ghost
keepalive, monitor tracking, runtime event routing.

## Verifying popup / ghost window behavior

Popup hide/show/configure drive live X11 / `NSWindow` state, so do NOT verify
them by creating windows on the running session. A test that calls
`configure_popup_window` (makes a window an always-on-top dock) or
`show_window_by_title` (forces activation via `_NET_ACTIVE_WINDOW`) wedges a live
Cinnamon/Muffin session (work-area struts, focus, panel layer) until `cinnamon
--replace`. There is no safe live-session integration test for these paths, and
a previous one had to be removed after it broke a running desktop.

Verify these paths through the runtime tracer (`qol trace`) instead: it reads the
real `_NET_WM_WINDOW_OPACITY`, map state, and ghost role of every popup window
without mutating session state. Pure geometry (placement, monitor math) stays in
ordinary unit tests like `tests/placement.rs`.
