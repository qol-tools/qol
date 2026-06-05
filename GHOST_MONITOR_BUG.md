# Ghost popup drifts / freezes on the wrong monitor (Linux/X11)

## Symptom
On Linux (Cinnamon/Muffin, multi-monitor), the alt-tab picker and the launcher
each keep a hidden "ghost" window alive and reposition it onto the active monitor
as the cursor moves, so the next show is instant. The two ghosts intermittently
end up on **different monitors**, and one will **freeze on the wrong monitor for
seconds** before correcting.

## Measured behaviour
A read-only watcher (`/tmp/qol-divwatch.sh`) polled real window geometry
(`xwininfo`) vs cursor on a 3-monitor setup and logged every divergence/sync
transition (`/tmp/qol-divergence.log`). Two distinct classes:

- **Sub-second transients** during fast cursor sweeps: the two windows lag each
  other by a frame, then resync. Roughly symmetric between the plugins.
- **Multi-second freezes**: one ghost stuck on the wrong monitor for **7-31s**,
  "resolving" only when the cursor happens to return to where the stuck window
  is. Both plugins exhibit it (alt-tab ~70%, launcher ~30%).

## What has been ruled out (proven, not assumed)
A cross-process probe (`RECORD` the event monitor, `SYNC` the committed bounds,
tagged by pid) plus the plugin logs establish that, on every reposition:

1. Both plugins receive the **identical** `ActiveMonitorChanged` event and record
   the **identical** monitor. The runtime's single source of truth
   (`apps/qol-tray/src/runtime/state.rs::pick_active_monitor`) is **not**
   diverging.
2. Both compute the **identical** target placement and issue the **identical**
   X11 move, which returns `ok=true`.
3. Placement math is locked by unit tests (`libs/qol-gpui/tests/placement.rs`).

So the source, event delivery, placement logic, and the X11 request itself are
all correct and identical across both plugins. **The failure is strictly below
the X11 call.**

## Root cause
The ghost is repositioned by sending an X11 `ConfigureWindow` (move + resize) to a
WM-managed window of type `_NET_WM_WINDOW_TYPE_DOCK`
(`libs/qol-gpui/src/popup_window/platform/linux.rs` -> `set_window_bounds` /
`move_window`; type set in `configure_popup_window`). Muffin intercepts these as
`ConfigureRequest`s and **intermittently does not honour the cross-monitor move**:
the X server acks the request (`ok=true`) but the window does not actually move.

Because repositioning is purely event-driven with **no reconciliation**, once the
cursor comes to rest no further `ActiveMonitorChanged` fires, so a dropped move is
never corrected - the ghost stays on the wrong monitor until the next monitor
change.

The design is the real problem: a single window is **imperatively chased** across
monitors via side-effecting X11 moves, and correctness depends on every single
move being honoured by the WM. It is not honoured reliably.

## Why "retry the move N times" is not acceptable
Re-issuing the same `ConfigureWindow` only raises the probability the WM happens
to honour one attempt. It does not make placement deterministic, it adds
timing-dependent churn, and it preserves the fragile chase-with-moves design. It
treats the symptom.

## Correct fix (direction)
Eliminate the cross-monitor move. Make active-monitor selection a **visibility
choice over pre-placed windows**, not a geometry mutation:

- Pre-create one hidden ghost window **per monitor**, each created on - and never
  moved from - its own monitor. On `ActiveMonitorChanged`, **show the target
  monitor's window and hide the others**. Map/opacity show-hide is same-monitor
  and reliable; no cross-monitor `ConfigureWindow` is ever issued, so there is
  nothing for the WM to drop. This is declarative (UI derived from state, per the
  repo's frontend rules) instead of imperative sync.
- alt-tab already has the infrastructure: `PickerWindowState` is keyed by
  `MonitorKey` and `picker_window_title(target)` encodes the monitor. The
  regression is `reuse_picker_across_targets() == true`
  (`plugins/plugin-alt-tab/src/picker/platform/linux.rs`) collapsing it to one
  moved window. The launcher is single-window and needs the same per-monitor set.
- Put the per-monitor ghost set in the shared `qol_gpui::ghost` layer so alt-tab
  and launcher use **one identical mechanism** (hard requirement: both must share
  the logic).

Alternative if per-monitor windows are undesirable: create the ghost as an
**override-redirect** window so the WM never manages or clamps it and
`ConfigureWindow` is applied verbatim by the X server. Keeps single-window moves
but removes WM interception. Trade-off: loses WM-managed services and needs
verification that gpui can create override-redirect popups.

**Recommendation:** per-monitor windows in shared `qol_gpui::ghost`. Deterministic,
declarative, unifies both plugins, and the alt-tab infra already supports it.

## Tooling / probes currently in the tree (REMOVE before merge)
- `/tmp/qol-divwatch.sh` -> `/tmp/qol-divergence.log`: read-only divergence watcher.
- `qol_gpui::ghost` `record_active_monitor` / `sync_window_layout`: append
  `RECORD`/`SYNC` lines to `/tmp/qol-ghost-probe.log`.
- qol-tray `log_probe` (`apps/qol-tray/src/logging/file_logger.rs`) and
  `probe_active_change` (`apps/qol-tray/src/runtime/server/poll/events.rs`):
  persist active-monitor decisions to the rotating log.

## Related fixes already landed (keep - they are correct)
- launcher ghost was created `is_movable: false`, which made Muffin refuse **all**
  moves of it (a separate, total failure mode); changed to `true`
  (`plugins/plugin-launcher/src/ui/window_host.rs`).
- qol-tray self-recompile killed its own process: the orphan sweep's
  `is_host_binary` did not strip the ` (deleted)` suffix that `/proc/<pid>/exe`
  gets after `cargo build` replaces the binary, so qol-tray classified itself as a
  stray plugin and SIGTERM'd before re-exec
  (`apps/qol-tray/src/plugins/daemon_tracker/mod.rs`).
- alt-tab had a second, asymmetric reposition path on data-refresh
  (`plugins/plugin-alt-tab/src/picker/monitor_listener.rs`); removed so both
  plugins reposition only on `ActiveMonitorChanged`.
