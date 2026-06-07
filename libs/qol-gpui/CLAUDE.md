# qol-gpui

Shared GPUI helpers for qol-tray plugins: popup-window placement, ghost
keepalive, monitor tracking, runtime event routing.

## Ghost popup architecture (per-monitor, never cross-monitor move)

The alt-tab picker and the launcher each keep a hidden "ghost" window warm so the
next show is instant. The active-monitor decision is owned by qol-runtime
(`pick_active_monitor`); plugins hold zero monitor state and must not re-derive
it.

The hard rule: a warm ghost is **one pre-placed hidden window per monitor**, and
following the active monitor is a **visibility choice (show the target, hide the
rest), never a geometry move**. Both plugins go through the shared `ghost` layer
(`reconcile`, `reconcile_active`, `show_ghost_window`, `dismiss_to_ghost`) so the
mechanism is identical.

Topology changes invalidate the set: on `RuntimeEvent::MonitorsChanged`
(resolution change, monitor added/removed) the shared `ghost::rebuild_on_topology`
flushes the cached active monitor (`refresh_active_monitor_from_state`), destroys
the plugin's hidden ghost set (`ActiveWindows::destroy_all`), and invokes the
plugin's boot pre-create - the ordering lives in the lib; plugins pass only their
visibility bool and pre-create closure.
Keys are geometry (`MonitorKey`), so any geometry change is a new key; stale
ghosts the OS displaced during reconfiguration must never be shown again.
`MonitorsChanged` needs its **own** event-router subscription - the router
coalesces a burst to the latest event, and the server batches
`ActiveMonitorChanged` after `MonitorsChanged` in the same tick, so sharing one
subscription would swallow the rebuild. Rebuilds are skipped while the popup is
visible (next `MonitorsChanged` or fresh-key show self-heals).

Why, not preference: chasing a single window across monitors with an X11
`ConfigureWindow` (move/resize) on a WM-managed `_NET_WM_WINDOW_TYPE_DOCK` window
is unreliable. Muffin (Cinnamon) intermittently drops the cross-monitor move - the
X server acks `ok=true` but the window does not move - and because repositioning
is event-driven with no reconciliation, a dropped move is never corrected until
the next `ActiveMonitorChanged`, so the ghost freezes on the wrong monitor for
seconds. Same-monitor map/opacity show-hide has nothing for the WM to drop.
Retrying the move only raises the odds one attempt lands; it does not make
placement deterministic. Do not reintroduce a cross-monitor move path (e.g. a
single shared window with `reuse_picker_across_targets`).

Related gotcha: a ghost created `is_movable: false` makes Muffin refuse **all**
moves of it (a separate total-failure mode) - keep ghost windows `is_movable:
true`.

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
