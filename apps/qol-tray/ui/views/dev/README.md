# Dev View Notes

## Mock Flow Execution

- `Test mock flows` uses backend mock targets when available.
- If backend plugin-build mocks are unavailable, it falls back to local plugin-build simulation.
- Backend execution is required for hard-reload continuity of active mock runs.

## Plugin Row Build Overlay

- Plugin build overlays are tracked per plugin row.
- Completion playback is phase-based (`ramp -> hold -> fade`) and should run once per row.
- Completion state is restored on rerender so normal UI updates do not reset active row playback.
