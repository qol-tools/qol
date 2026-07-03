# macOS Window Resize Performance Notes

## Scope

- Goal: improve speed of macOS resize actions.
- Actions measured: `snap-left`, `snap-right`, `snap-bottom`, `center`, `maximize`.
- Out of scope: minimize/restore behavior, activation reliability, and window identity changes.
- Method: trace-bounded local runs against a temporary TextEdit document, with before/after tables recorded before accepting the implementation.

## Summary

The winning change caches macOS visible-screen geometry for short bursts of window actions. The resize path launches as a short-lived process per action, and repeated `NSScreen` visible-frame lookup was costing roughly 39ms per invocation. A 10s cache drops that lookup to near-zero for repeated left/right/bottom/center/maximize actions while keeping the cache short enough to recover quickly from monitor-layout changes.

## Accepted Change: 10s Screen Geometry Cache

Baseline A/B run: `qol-resize-baseline-ab-1783068370`.
Accepted cold-cache run: `qol-resize-h6-screen-cache-10s-cold-1783068765`.

| Action | Baseline Avg ms | Baseline Median ms | After Avg ms | After Median ms | Avg Delta ms | Median Delta ms | Verdict |
|---|---:|---:|---:|---:|---:|---:|---|
| `center` | 82.3 | 80.5 | 49.2 | 50.5 | -33.1 | -30.0 | pass |
| `maximize` | 84.7 | 82.5 | 50.8 | 51.0 | -33.9 | -31.5 | pass |
| `snap-bottom` | 85.8 | 86.0 | 50.3 | 51.0 | -35.5 | -35.0 | pass |
| `snap-left` | 87.8 | 87.0 | 60.7 | 50.0 | -27.1 | -37.0 | pass |
| `snap-right` | 81.8 | 81.0 | 46.0 | 49.5 | -35.8 | -31.5 | pass |

| Operation | Before Avg ms | Before Median ms | After Avg ms | After Median ms | Result |
|---|---:|---:|---:|---:|---|
| `screen_for_point` | 38.9 | 38.0 | 1.9 | 0.0 | Cache removes repeated cold `NSScreen` lookup |

Final rebuilt smoke run: `qol-resize-final-screen-cache-smoke-1783068818`; `screen_for_point` averaged 3.2ms with cold cache included, median 0.0ms.

## Rejected Hypotheses

| Hypothesis | Result | Reason |
|---|---|---|
| Reuse front AX target for read and write | Rejected | Halved `front_target` count, but action timings were neutral or worse. |
| Trust focused AX window fast path | Rejected | Removed one targetability path, but `front_window_rect` stayed effectively unchanged. |
| Read initial bounds from `CGWindowListCopyWindowInfo` | Rejected | Shifted the cold AX lookup into `set_pos_size`, making total action time worse. |

## Tradeoff

The cache can be stale for up to 10 seconds after a monitor layout, resolution, Dock/menu-bar, or visible-frame change. The short TTL keeps resize bursts fast while limiting the stale-layout window. If this still feels risky, reduce the TTL or invalidate it from a future monitor-change signal.
