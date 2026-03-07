# Dev View Preact Migration

## Problem

The dev view replaces the entire DOM on every state change via full DOM replacement on every render call, including SSE events arriving every 1-2 seconds (CPU snapshots, build progress). This destroys and recreates the entire DOM, causing:

1. **Broken clicks** — DOM replaced between mousedown and click events
2. **Broken DevTools** — Element tree collapses on every re-render
3. **State loss** — Scroll position, spinner timestamps, hover states lost on each render
4. **Workaround bloat** — `view-dom.js` exists solely to save/restore DOM state that Preact would preserve automatically

## Approach

Migrate the dev view from string templates to Preact components, matching the patterns already used by StoreView, HotkeysView, and other views in the codebase.

## Architecture

### Rendering Layer

Replace string templates with Preact components using `html` (htm + preact/h):

| Current file | Becomes |
|---|---|
| `template.js` | `components/DevLayout.js` — page shell, sections |
| `plugin-row-template.js` | `components/PluginRow.js` — single plugin row |
| `plugin-row/menu.js` | `components/PluginMenu.js` — context menu per plugin |
| `plugin-row/cpu.js` | `components/CpuStrip.js` — CPU sparkline + percentage |
| `plugin-row/status.js` | `components/StatusBadges.js` — status badge rendering |
| `core-log-template.js` | `components/CoreLogSection.js` — core log controls |

### State Management

Keep the existing mutable `state` object and controllers. Wrap in a custom `useDevController` hook (matching the `useStoreController` pattern) that:

1. Creates controller instances once via `useRef`
2. Exposes a `tick` counter via `useState` — controllers call `onNeedsRender` which increments it
3. Returns a stable interface of state snapshots and action callbacks for components

This avoids rewriting controllers while giving Preact the render trigger it needs.

### Composition Root

`index.js` shrinks to a thin Preact composition root:
- Creates the controller hook
- Passes controller output as props to `DevLayout`
- Subscribes to SSE via `useEffect`
- Exposes `handleKey` and `isBlocking` for the view router

`view.js` stays as-is (it already renders a Preact component).

### Event Handling

Replace `data-action` event delegation with direct Preact `onClick` handlers on each component. The `action-router.js` and `key-router.js` files dissolve — their logic moves into controller methods and component handlers.

`key-router.js` becomes a pure function called from the controller's `handleKey`, similar to `handleStoreKey` in the store view.

### Files Removed After Migration

| File | Reason |
|---|---|
| `view-dom.js` | DOM save/restore unnecessary — Preact preserves DOM |
| `action-router.js` | Logic absorbed by component onClick handlers |
| `template.js` | Replaced by `DevLayout.js` |
| `plugin-row-template.js` | Replaced by `PluginRow.js` |
| `core-log-template.js` | Replaced by `CoreLogSection.js` |

### Files Preserved Unchanged

| File | Reason |
|---|---|
| `plugin-model.js` | Pure data transform, no DOM |
| `cpu-controller.js` | Service layer, no DOM |
| `build-controller.js` | Service layer (build overlay DOM ops may need adaptation) |
| `discovery-controller.js` | Service layer, no DOM |
| `mock-controller.js` | Service layer, no DOM |
| `plugin-actions-controller.js` | Service layer, no DOM |
| `core-log-actions.js` | Service layer, no DOM |
| `cpu/*` | Pure data/sync utilities |
| `build/*` | Pure data/sync utilities |
| `mock/*` | Pure data utilities |
| `discovery/*` | Pure data utilities |
| `plugin-actions/*` | Pure data/API utilities |

### Build Overlay Consideration

`build-controller.js` currently does direct DOM manipulation (`cacheRows`, `syncAll`) for build progress overlays. Two options:

**A) Keep imperative overlays** — The build overlay is a CSS animation system that fills progress bars. It operates on `.plugin-build-overlay-host` elements. Keep this as a `useEffect` that runs after render, finding overlay hosts in the new Preact DOM. Minimal change to build-overlay code.

**B) Convert overlays to Preact** — Would require converting `build-overlay/*.js` to reactive state. Higher risk, can be done as a follow-up.

Recommendation: **Option A** for this migration. The overlay system is complex and self-contained; converting it is a separate task.

## Component Tree

```
DevView (view.js — existing, unchanged)
  DevLayout
    PageHeader (static)
    PluginsSection
      SectionHeader (discover button, add-link button)
      PluginRow[] (keyed by plugin.id)
        PluginInfo (name, path, build meta)
          StatusBadges
          CpuStrip (sparkline + percentage)
        ActionColumn (link/unlink button)
        PluginMenu (context menu)
      LinkInput (conditional)
    CoreLogSection
      CoreLogRow[]
    ActionsSection
      ReloadCard
      MockCard
```

## Migration Strategy

Incremental, one section at a time. Each step produces a working commit:

1. Create `useDevController` hook — wraps existing state + controllers, exposes render trigger
2. Convert `index.js` to Preact mount — replace updateView with Preact render
3. Convert `template.js` to `DevLayout.js` — top-level shell
4. Convert `ActionsSection` — simplest section (ReloadCard + MockCard)
5. Convert `PluginsSection` — plugin rows, menus, CPU strips
6. Convert `CoreLogSection`
7. Wire build overlay `useEffect`
8. Remove dead files (view-dom.js, action-router.js, old templates)
9. Wire keyboard handling through controller

## Constraints

- Max 20 non-empty lines per production function
- No parameter bag objects — use named positional args or component props
- Every file has one owner and one responsibility
- Preserve all behavior, UX, keyboard shortcuts, and event semantics
- Preserve controller queueing/cancellation/lifecycle semantics
- Reuse existing hooks (useSSE, useRefreshOnFocus, etc.) where applicable
- Early returns, shallow control flow
- No comments in code
