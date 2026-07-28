# macOS Window Resize Performance Notes

## Scope

- Goal: make macOS window actions react as quickly and reliably as the Cinnamon path.
- Actions measured: `snap-left`, `snap-right`, `center`, `maximize`, `move-monitor-left`, and `move-monitor-right`.
- Method: trace-bounded local runs against real application windows on multiple displays.

## Summary

The macOS path now keeps the development daemon resident, resolves the focused application through Accessibility, reuses screen geometry until the physical display topology or work-area preferences change, and verifies a resize without polling for an impossible exact frame. Final warm center runs settled to 10–25ms, with occasional Accessibility or scheduler spikes; monitor moves measured 4–27ms.

## Accepted Changes

### Resident development daemon

The `.qol-tray-dev-autostart` marker starts Window Actions with the host. This removes the roughly 260ms first-action daemon startup from development-linked installs.

### Topology-keyed screen snapshot

Visible `NSScreen` frames are cached in memory and on disk with a key containing CoreGraphics display bounds and the metadata generation of Dock, global, and Control Center preferences. Repeated actions avoid AppKit startup while display rearrangement, resolution changes, and work-area preference changes invalidate the snapshot immediately.

### Focused application lookup

The system-wide Accessibility element supplies the focused application PID before `NSWorkspace` and WindowServer fallbacks. This avoids stale `NSWorkspace` results observed after a host rebuild, including incorrectly targeting `loginwindow`.

### Constraint-aware geometry verification

The setter reads the window frame once after applying position and size. It accepts exact frames, legitimate macOS work-area constraints, and other real adjustments, while rejecting unchanged or unreadable windows. The old 120ms polling loop waited for exact geometry even when macOS intentionally clamped the requested frame around the menu bar.

## Measured Result

| Path | Before | After |
|---|---:|---:|
| First action with daemon startup | about 329ms | about 4ms dispatch |
| Cold in-daemon action | up to 328ms | 59–81ms |
| Warm resize action | 128–224ms | 10–28ms typical |
| Warm monitor move | 202–224ms | 4–27ms |

## Rejected Hypotheses

| Hypothesis | Result | Reason |
|---|---|---|
| Reuse front AX target for read and write | Rejected | Halved `front_target` count, but action timings were neutral or worse. |
| Trust focused AX window fast path | Rejected | Removed one targetability path, but `front_window_rect` stayed effectively unchanged. |
| Read initial bounds from `CGWindowListCopyWindowInfo` | Rejected | Shifted the cold AX lookup into `set_pos_size`, making total action time worse. |

## Tradeoff

The persistent snapshot depends on CoreGraphics topology plus preference-file metadata rather than a timer. A future work-area change that affects neither signal would require an additional invalidation source, but ordinary display and Dock/menu-bar configuration changes are covered without imposing a recurring latency tax.
