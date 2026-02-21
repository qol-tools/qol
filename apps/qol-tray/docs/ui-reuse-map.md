# UI Reuse Map

This document tracks shared UI primitives in QoL Tray and the next high-impact reuse extractions.

## Already centralized

- API request primitives
  - File: `ui/api/client.js`
  - Shared utilities: `apiJson`, `apiText`, `apiResponse`, `jsonRequest`, `readResponseText`
  - In use by: `ui/views/plugins.js`, `ui/views/store.js`, `ui/views/hotkeys.js`, `ui/views/dev.js`, `ui/main.js`

- Progress formatting and normalization
  - File: `ui/utils/progress.js`
  - Shared utilities: `clampPercent`, `normalizePercent`, `toProgressScale`, `formatDownloadingProgress`, `formatPhaseProgress`, `formatBuildOverlayDetail`
  - In use by: `ui/components/sidebar.js`, `ui/main.js`, `ui/views/dev.js`

- Modal primitives
  - File: `ui/components/modal.js`
  - Shared utilities: `openModal`, `closeModal`, `matchModalAction`
  - In use by: `ui/views/plugins.js`, `ui/views/hotkeys.js`, `ui/features/task-runner/view.js`

- Feedback rendering + escaping
  - File: `ui/components/feedback.js`
  - Shared utilities: `renderFeedback`, `escapeHtml`
  - In use by: `ui/views/plugins.js`, `ui/views/store.js`, `ui/features/task-runner/view.js`

- Installed plugin payload parsing
  - File: `ui/utils/plugins.js`
  - Shared utilities: `parseInstalledPayload`, `parseInstalledPlugins`
  - In use by: `ui/views/plugins.js`, `ui/views/hotkeys.js`

## Next reuse extractions (recommended)

1. View bootstrap helper
- Problem: most views repeat `render`, register listeners, initial load, subscribe/unsubscribe lifecycle.
- Target: create `ui/view-runtime.js` with a tiny lifecycle adapter used by `plugins`, `store`, `hotkeys`, `dev`.

2. Keyboard command maps
- Problem: each view has local key handling patterns for arrows/enter/escape with similar guard logic.
- Target: create `ui/input/keymap.js` for declarative key-to-handler mapping + common modifier guards.

3. Status row / badge renderer
- Problem: status chips and inline metadata are assembled ad hoc in multiple screens.
- Target: create `ui/components/status.js` to emit normalized badge markup and avoid per-view style drift.

4. Async action state helper
- Problem: repeated loading/error/success flags and timer cleanup logic across views.
- Target: create `ui/state/async-flow.js` for common flow transitions and clear-timer behavior.

5. Fetch + stale-response guard utility
- Problem: stale-request token checks are implemented manually (for example in store).
- Target: create `ui/api/request-guard.js` to standardize latest-only request semantics.

## Refactor safety rules

- Prefer extracting pure functions first, then wiring call sites.
- Keep endpoint behavior unchanged when migrating to shared API helpers.
- Run `node --check` on every touched UI file before committing.
- Avoid style/layout changes in reuse commits unless required by extraction.
