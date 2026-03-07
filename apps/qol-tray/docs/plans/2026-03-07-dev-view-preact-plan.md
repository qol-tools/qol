# Dev View Preact Migration — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace the dev view's full-DOM string template rendering with incremental Preact components to fix broken clicks, DevTools inspection, and DOM state loss.

**Architecture:** Keep all existing controllers (cpu, build, discovery, mock, actions, coreLog) unchanged. Create a `useDevController` hook that wraps the mutable state object and wires controllers, exposing a tick-based re-render trigger. Convert string templates to Preact components one section at a time.

**Tech Stack:** Preact + htm (via `ui/lib/html.js`), existing hooks (`useSSE`, `useSSEReconnect`), existing controller pattern from StoreView.

**Worktree:** `/home/user/git/qol-tools/qol-tray/.worktrees/hide-cpu-button`

**All file paths below are relative to** `ui/views/dev/` unless stated otherwise.

---

### Task 1: Create useDevController hook

**Files:**
- Create: `use-controller.js`
- Create: `keys.js`

The hook wraps the existing mutable `state` object and all controllers. It exposes a `tick` counter that controllers increment via `onNeedsRender`, which triggers Preact re-renders. Follow the `useStoreController` pattern from `ui/views/store/use-controller.js`.

**Step 1: Write `keys.js`**

Port keyboard routing from `key-router.js` as pure functions. Functions: `handleDevKey`, `handleEscapeKey`, `handleArrowKey`, `handleMenuToggleKey`, `nextSelectedIndex`.

Key reference — current behavior in `key-router.js`:
- `Ctrl+R` / `Cmd+R`: reload plugins
- `Escape`: close open menu
- `ArrowDown` / `ArrowUp`: navigate selected index
- `Space` / `Enter`: activate selected item
- `r` / `R`: trigger discovery
- `m` / `M`: toggle menu on selected plugin

Signature: `handleDevKey(event, state, ctrl, bump)` where `ctrl` has `actionsController` and `discoveryController`, and `bump` triggers re-render.

**Step 2: Write `use-controller.js`**

Core structure:
```
createInitialState() — returns the state object (same shape as current)
useDevController(containerRef) — the hook
  initDataControllers(state, bump) — creates discovery, cpu, coreLog controllers
  initBuildControllers(state, containerRef, dataCtrl, bump) — creates build, mock, actions controllers
  syncMergedList(state, ctrl) — recomputes merged plugin list (runs each render)
  useSSESubscription(state, ctrl, bump) — subscribes to SSE events
  useReconnectSubscription(state, ctrl) — subscribes to reconnect events
  useHydration(state, ctrl) — initial data load on mount
  useFocusLifecycle(state, ctrl, bump) — focus/blur handlers
  buildStateProps(state) — extracts state snapshot for components
  buildActionCallbacks(state, ctrl, bump) — creates action callbacks for components
```

Every function must be under 20 non-empty lines. Split `initControllers` into `initDataControllers` and `initBuildControllers`.

The `handleSSEEvent` function routes events identically to the current `handleEvent` in `index.js`:
- `discovery_started`: apply `nextDiscoveryStartedState()`, bump
- `discovery_complete`: apply `nextDiscoveryCompletedState(event.plugins)`, bump
- `plugins_changed`: call `discoveryController.loadLinkedPlugins()`
- CPU events: delegate to `cpuController.handleEvent`
- Build/mock events: delegate to respective controllers

`buildControllerInterface` merges `buildStateProps` + `buildActionCallbacks` plus exposes `buildController`, `cpuController`, `handleKey`, `onFocus`, `onBlur`.

**Step 3: Verify the hook compiles**

Load the page. The hook won't be wired yet, but import it in a throwaway test to check for syntax errors.

**Step 4: Commit**

```bash
git add ui/views/dev/use-controller.js ui/views/dev/keys.js
git commit -m "feat: add useDevController hook and key handler for Preact dev view"
```

---

### Task 2: Create ActionsSection component

**Files:**
- Create: `components/ActionsSection.js`
- Create: `components/BuildResults.js`

**Step 1: Write `BuildResults.js`**

Converts `renderBuildResults` from `plugin-model.js` to a Preact component. Same logic: check for null, all-skipped, all-success, or failure. Returns `<span>` with appropriate class.

**Step 2: Write `ActionsSection.js`**

Contains: `ActionsSection`, `ReloadCard`, `MockCard`.

`ActionsSection` renders the "Actions" section with two cards.

`ReloadCard` props: `building`, `buildResults`, `lastReload`, `error`, `reloadPlugins`. Uses `<BuildResults>` component.

`MockCard` props: `mockTesting`, `triggerMockFlows`. Click handler calls `triggerMockFlows`.

Both cards use direct `onClick` handlers instead of `data-action` delegation.

**Step 3: Commit**

```bash
git add ui/views/dev/components/ActionsSection.js ui/views/dev/components/BuildResults.js
git commit -m "feat: add ActionsSection and BuildResults Preact components"
```

---

### Task 3: Create plugin row sub-components

**Files:**
- Create: `components/StatusBadges.js`
- Create: `components/BuildMeta.js`
- Create: `components/CpuStrip.js`
- Create: `components/PluginMenu.js`

**Step 1: Write `StatusBadges.js`**

Converts `plugin-row/status.js`. Contains `StatusBadges`, `StatusBadge`, `BuildBadge`. Same conditional logic, returns Preact vdom instead of HTML strings.

**Step 2: Write `BuildMeta.js`**

Converts `renderPluginBuildMeta` from `plugin-model.js`. Contains `BuildMeta` and `buildMetaParts` helper. Includes `shortFingerprint` utility.

**Step 3: Write `CpuStrip.js`**

Converts `plugin-row/cpu.js`. Props: `plugin`, `cpuMonitoring`, `cpuByPlugin`. Guards on `plugin.status !== 'linked'` and `!cpuMonitoring[plugin.id]`. Uses `renderCpuSparkline` from `cpu/sparkline.js` — this returns an SVG string, so use Preact's `dangerouslySetInnerHTML` for the graph div. The sparkline is computed from trusted internal data (not user input), so this is safe.

**Step 4: Write `PluginMenu.js`**

Converts `plugin-row/menu.js`. Contains `PluginMenu`, `MenuIcon`, `MuteLogsAction`, `EditFiltersAction`, `CpuAction`. Each is a small component under 20 lines.

Props for `PluginMenu`: `plugin`, `menuOpen`, `onToggleMenu`, `onToggleLogs`, `onEditFilters`, `onToggleCpu`.

The CPU action button is conditionally rendered only when `plugin.status === 'linked'`.

**Step 5: Commit**

```bash
git add ui/views/dev/components/StatusBadges.js ui/views/dev/components/BuildMeta.js ui/views/dev/components/CpuStrip.js ui/views/dev/components/PluginMenu.js
git commit -m "feat: add StatusBadges, BuildMeta, CpuStrip, and PluginMenu Preact components"
```

---

### Task 4: Create PluginRow component

**Files:**
- Create: `components/PluginRow.js`

**Step 1: Write `PluginRow.js`**

Contains: `PluginRow`, `PluginInfo`, `ActionColumn`.

`PluginRow` computes derived state (statusToken, isSelected, isBuilding, isLinking, menuOpen, rebuildActive) from props, renders `.plugin-row` div with data attributes for build overlay compatibility.

`PluginInfo` renders name, path, `<BuildMeta>`, `<StatusBadges>`, `<CpuStrip>`.

`ActionColumn` renders the link/unlink button and `<PluginMenu>`. Click handler on link button: sets selected index and calls `handleItemActivation`. Menu action handlers call `ctrl.closeMenus()` then the specific action, with `e.preventDefault()` and `e.stopPropagation()`.

Keep each component under 20 non-empty lines. If `ActionColumn` exceeds the limit, extract the menu event handler creation into a factory function.

**Step 2: Commit**

```bash
git add ui/views/dev/components/PluginRow.js
git commit -m "feat: add PluginRow Preact component"
```

---

### Task 5: Create PluginsSection, LinkInput, and DevLayout

**Files:**
- Create: `components/PluginsSection.js`
- Create: `components/LinkInput.js`
- Create: `components/DevLayout.js`

**Step 1: Write `LinkInput.js`**

Conditional component: returns null when `showLinkInput` is false. Renders input with `onInput` and `onKeyDown` (Enter confirms, Escape cancels), confirm button, cancel button, optional error message.

**Step 2: Write `PluginsSection.js`**

Renders the "Plugins" section: header with discover button and add-link button, plugin list with `<PluginRow>` keyed by `plugin.id`, and `<LinkInput>`.

**Step 3: Write `DevLayout.js`**

Top-level shell: `DevPageHeader` (static), `.view-body` containing `<PluginsSection>`, `<CoreLogSection>`, `<ActionsSection>`. Matches the structure in current `template.js`.

**Step 4: Commit**

```bash
git add ui/views/dev/components/PluginsSection.js ui/views/dev/components/LinkInput.js ui/views/dev/components/DevLayout.js
git commit -m "feat: add DevLayout, PluginsSection, and LinkInput Preact components"
```

---

### Task 6: Create CoreLogSection component

**Files:**
- Create: `components/CoreLogSection.js`

**Step 1: Write `CoreLogSection.js`**

Contains: `CoreLogSection`, `CoreLogRow`, `CoreLogMenu`.

`CORE_SECTIONS` array is defined at module level (same as `core-log-template.js`).

`CoreLogSection` maps over CORE_SECTIONS rendering `<CoreLogRow>` keyed by section id.

`CoreLogRow` computes muted/filterCount/menuOpen from ctrl props, renders the row structure.

`CoreLogMenu` renders menu trigger + dropdown with mute/filter actions. Click handlers call `ctrl.toggleCoreMenu`, `ctrl.toggleCoreLogs`, `ctrl.editCoreLogFilters` with appropriate event prevention.

Re-use the `MenuIcon` SVG from `PluginMenu.js` — extract to a shared `components/MenuIcon.js` if needed, or just duplicate the 3-line SVG (it's tiny, DRY doesn't justify a new file for 3 lines).

**Step 2: Commit**

```bash
git add ui/views/dev/components/CoreLogSection.js
git commit -m "feat: add CoreLogSection Preact component"
```

---

### Task 7: Wire composition root to Preact

**Files:**
- Rewrite: `index.js`
- Modify: `view.js`

**Step 1: Rewrite `index.js`**

Replace the entire imperative module with a `DevViewInner` Preact component:
- Uses `useDevController(containerRef)` hook
- Uses `useFooterShortcuts(SHORTCUTS)` for the shortcut legend
- Renders `<DevLayout ctrl={ctrl} />` inside a ref'd container div
- Exposes `DevViewInner.handleKey` and `DevViewInner.isBlocking` as static properties (matching the pattern in `view.js`)
- Adds a `useBuildOverlaySync` effect that calls `ctrl.buildController.cacheRows()` and `ctrl.buildController.syncAll()` after every render

**Step 2: Update `view.js`**

Change imports: instead of importing `* as devModule` from `./index.js` and calling `devModule.render(el)`, import `DevViewInner` and render it as a Preact component.

New `view.js`:
```
DevView renders <DevViewInner />
DevView.handleKey delegates to DevViewInner.handleKey
DevView.isBlocking delegates to DevViewInner.isBlocking
```

Remove the old `containerRef` + `useEffect` + `devModule.render(el)` pattern. The footer shortcut legend is now handled inside `DevViewInner` via `useFooterShortcuts`.

**Step 3: Verify the page loads**

Open the dev view. All sections should render. SSE events should update CPU data without full DOM replacement.

**Step 4: Commit**

```bash
git add ui/views/dev/index.js ui/views/dev/view.js
git commit -m "refactor: wire dev view composition root to Preact rendering"
```

---

### Task 8: Remove dead files

**Files:**
- Delete: `view-dom.js`
- Delete: `action-router.js`
- Delete: `template.js`
- Delete: `plugin-row-template.js`
- Delete: `core-log-template.js`
- Delete: `key-router.js`
- Delete: `plugin-row/menu.js`
- Delete: `plugin-row/cpu.js`
- Delete: `plugin-row/status.js`
- Remove: `plugin-row/` directory if empty

**Step 1: Verify no remaining imports**

Search all `.js` files under `ui/views/dev/` for imports of deleted files. Fix any remaining references. Check `plugin-model.js` still exports `mergePlugins` (used by `use-controller.js`) — it does, and `renderPluginBuildMeta` / `renderBuildResults` are no longer needed as exports (replaced by Preact components), but they may be imported elsewhere. Check and remove if dead.

**Step 2: Delete files and commit**

```bash
cd /home/user/git/qol-tools/qol-tray/.worktrees/hide-cpu-button
rm ui/views/dev/view-dom.js ui/views/dev/action-router.js ui/views/dev/template.js
rm ui/views/dev/plugin-row-template.js ui/views/dev/core-log-template.js ui/views/dev/key-router.js
rm ui/views/dev/plugin-row/menu.js ui/views/dev/plugin-row/cpu.js ui/views/dev/plugin-row/status.js
rmdir ui/views/dev/plugin-row 2>/dev/null || true
git add -A ui/views/dev/
git commit -m "refactor: remove dead string template and DOM workaround files"
```

---

### Task 9: Clean up plugin-model.js exports

**Files:**
- Modify: `plugin-model.js`

**Step 1: Remove dead exports**

`renderPluginBuildMeta` and `renderBuildResults` are now replaced by Preact components (`BuildMeta.js` and `BuildResults.js`). Check if anything still imports them. If not, remove them from `plugin-model.js`. Keep `mergePlugins` (used by `use-controller.js`).

Also remove the `escapeHtml` import if no remaining functions use it.

**Step 2: Commit**

```bash
git add ui/views/dev/plugin-model.js
git commit -m "refactor: remove dead renderPluginBuildMeta and renderBuildResults from plugin-model"
```

---

### Task 10: Final verification and line count enforcement

**Step 1: Verify all behavior**

Test checklist:
- [ ] Plugin list renders with correct status badges
- [ ] CPU strip shows for linked plugins with monitoring enabled
- [ ] CPU strip hidden for non-linked plugins
- [ ] Context menus open/close on click and keyboard (m key)
- [ ] Keyboard navigation: arrow up/down, Enter, m, Esc, Ctrl+R, r
- [ ] Link input shows/hides, Enter confirms, Escape cancels
- [ ] Build progress overlays animate on linked plugin rows
- [ ] Mock flow card triggers and stops
- [ ] Reload card triggers build, shows results
- [ ] SSE events update CPU data without destroying DOM
- [ ] DevTools DOM tree is stable during SSE updates
- [ ] Clicks always register on first attempt
- [ ] Scroll position preserved during re-renders
- [ ] Focus/blur lifecycle refreshes data correctly

**Step 2: Enforce 20-line function limit**

Count non-empty lines in every function across all new/modified files. Split any that exceed 20 lines.

**Step 3: Commit any cleanup**

```bash
git add -A ui/views/dev/
git commit -m "refactor: enforce 20-line function limit across dev view components"
```

---

## File Summary

### New files (in `ui/views/dev/`)
| File | Responsibility |
|---|---|
| `use-controller.js` | Hook: wraps state + controllers, exposes render trigger |
| `keys.js` | Pure function: keyboard event routing |
| `components/DevLayout.js` | Component: page shell with sections |
| `components/PluginsSection.js` | Component: plugin list + link input |
| `components/PluginRow.js` | Component: single plugin row |
| `components/PluginMenu.js` | Component: per-plugin context menu |
| `components/CpuStrip.js` | Component: CPU sparkline display |
| `components/StatusBadges.js` | Component: status badge rendering |
| `components/BuildMeta.js` | Component: build fingerprint/status display |
| `components/BuildResults.js` | Component: build results summary |
| `components/CoreLogSection.js` | Component: core log controls |
| `components/ActionsSection.js` | Component: reload + mock cards |
| `components/LinkInput.js` | Component: link path input row |

### Deleted files
| File | Replaced by |
|---|---|
| `template.js` | `components/DevLayout.js` |
| `plugin-row-template.js` | `components/PluginRow.js` |
| `core-log-template.js` | `components/CoreLogSection.js` |
| `plugin-row/menu.js` | `components/PluginMenu.js` |
| `plugin-row/cpu.js` | `components/CpuStrip.js` |
| `plugin-row/status.js` | `components/StatusBadges.js` |
| `view-dom.js` | Preact handles DOM preservation |
| `action-router.js` | Component onClick handlers |
| `key-router.js` | `keys.js` |

### Modified files
| File | Change |
|---|---|
| `index.js` | Rewritten to Preact composition root |
| `view.js` | Updated to use DevViewInner |
| `plugin-model.js` | Dead exports removed |

### Unchanged files
All controllers, CPU subsystem, build subsystem, mock subsystem, discovery subsystem, plugin-actions subsystem remain unchanged.
