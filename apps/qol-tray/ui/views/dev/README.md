# Dev View Notes

## Mock Flow Execution

- `Test mock flows` uses backend mock targets when available.
- If backend plugin-build mocks are unavailable, it falls back to local plugin-build simulation.
- Backend execution is required for hard-reload continuity of active mock runs.

## Plugin Row Build Overlay

- Plugin build overlays are tracked per plugin row.
- Completion playback is phase-based (`ramp -> hold -> fade`) and should run once per row.
- Completion state is restored on rerender so normal UI updates do not reset active row playback.

## CPU Monitor Toggle

- CPU monitoring is synchronized to backend with `/api/dev/plugin-cpu/monitoring`.
- Disabled plugins are removed from backend sampling, not only hidden in UI.
- Enabling a plugin refreshes CPU snapshot state without requiring a browser refresh.
- Monitoring payload IDs are validated and bounded server-side before sampling state is updated.

## Security Notes

- The dev view renders through Preact/htm, which escapes interpolated values; there is no `innerHTML` write path. Status tokens are constrained to an allow-list via `safeStatusToken`.
- API mutation routes reject browser cross-site requests via fetch metadata/origin checks.
