# Contextual Sidebar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace SidebarNav's hardcoded mode logic with a context-driven push model so any view can control the sidebar.

**Architecture:** A `SidebarContext` at the app root holds `items` and `header`. SidebarNav reads context and renders. Views push their own items via `setItems`/`setHeader` and clean up via `resetSidebar`. Default items (view list) are computed in App.js.

**Tech Stack:** Preact + htm (no JSX, no build step), createContext/useContext/useState hooks.

---

### File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `ui/components/app/sidebar-context.js` | Create | Context, provider hook, consumer hook |
| `ui/components/SidebarNav.js` | Rewrite | Pure renderer — reads context, renders items |
| `ui/components/App.js` | Modify | Wrap in sidebar context, compute default items |
| `ui/views/plugin-config/view.js` | Modify | Push section items to sidebar on mount/change |
| `ui/views/dev/components/DevLayout.js` | Modify | Push catalog items when Components tab active |
| `ui/views/dev/components/ComponentsCatalog.js` | Modify | Receive `activeId`, render single showcase |

---

### Task 1: Create SidebarContext

**Files:**
- Create: `ui/components/app/sidebar-context.js`

- [ ] **Step 1: Create context file**

```js
import { createContext } from 'preact';
import { useContext, useState, useCallback, useMemo } from 'preact/hooks';

const SidebarContext = createContext(null);

export function useSidebarContext() {
    return useContext(SidebarContext);
}

export function useSidebarProvider({ defaultItems, defaultHeader }) {
    const [override, setOverride] = useState(null);
    const [header, setHeaderState] = useState(null);

    const setItems = useCallback((items) => setOverride(items), []);
    const setHeader = useCallback((h) => setHeaderState(h), []);
    const resetSidebar = useCallback(() => {
        setOverride(null);
        setHeaderState(null);
    }, []);

    const items = override || defaultItems;
    const value = useMemo(() => ({
        items,
        setItems,
        header: header !== null ? header : defaultHeader,
        setHeader,
        resetSidebar,
        isOverridden: override !== null,
    }), [items, header, defaultHeader, defaultItems, setItems, setHeader, resetSidebar, override]);

    return { SidebarContext, value };
}
```

- [ ] **Step 2: Syntax check**

Run: `node --check ui/components/app/sidebar-context.js`
Expected: no output (clean)

- [ ] **Step 3: Commit**

```bash
git add ui/components/app/sidebar-context.js
git commit -m "feat: add SidebarContext for push-model sidebar control"
```

---

### Task 2: Rewrite SidebarNav as pure renderer

**Files:**
- Modify: `ui/components/SidebarNav.js`

- [ ] **Step 1: Rewrite SidebarNav**

Replace the entire file. SidebarNav reads from context and renders — no mode logic, no plugin config import.

```js
import { html } from '../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import { useSidebarContext } from './app/sidebar-context.js';
import { materializeIn } from '../lib/dissolve.js';

let prevItemsRef = null;

export function SidebarNav() {
    const { items, header } = useSidebarContext();
    const itemsRef = useRef(null);

    useEffect(() => {
        const el = itemsRef.current;
        if (prevItemsRef !== null && prevItemsRef !== items && el?.offsetHeight > 0) {
            materializeIn(el);
        }
        prevItemsRef = items;
    });

    return html`
        ${header || html`<div class="sidebar-header"><span class="sidebar-logo">QoL Tray</span></div>`}
        <div class="sidebar-nav">
            <div class="sidebar-items" ref=${itemsRef}>
                ${items.map(item => html`
                    ${item.type === 'divider'
                        ? html`<div key=${item.key || item.id} class="sidebar-divider" aria-hidden="true"></div>`
                        : html`
                            <div key=${item.key || item.id}
                                class="sidebar-item ${item.active ? 'active' : ''}"
                                onClick=${item.onClick}>
                                <div class="sidebar-item-inner">
                                    <span>${item.label}</span>
                                    ${item.trailing}
                                </div>
                            </div>
                        `}
                `)}
            </div>
        </div>
    `;
}
```

- [ ] **Step 2: Syntax check**

Run: `node --check ui/components/SidebarNav.js`
Expected: clean

- [ ] **Step 3: Commit**

```bash
git add ui/components/SidebarNav.js
git commit -m "refactor: SidebarNav to pure context renderer"
```

---

### Task 3: Wire SidebarContext into App.js

**Files:**
- Modify: `ui/components/App.js`

- [ ] **Step 1: Add imports**

Add at top of file:

```js
import { useSidebarProvider } from './app/sidebar-context.js';
```

- [ ] **Step 2: Compute default items and wire provider in AppShell**

Inside `AppShell`, after the `useApp` destructure, add:

```js
const defaultHeader = null;
const defaultItems = useMemo(() => {
    const LABELS = {
        plugins: 'Plugins', store: 'Store', hotkeys: 'Hotkeys',
        shortcuts: 'Shortcuts', 'task-runner': 'Task Runner',
        profile: 'Profile', logs: 'Logs', dev: 'Developer'
    };
    const DIVIDER_BEFORE = new Set(['hotkeys', 'profile', 'dev']);
    return viewOrder.flatMap(id => {
        const out = [];
        if (DIVIDER_BEFORE.has(id)) out.push({ type: 'divider', key: `divider:${id}` });
        out.push({
            type: 'item',
            key: id,
            id,
            label: LABELS[id] || id,
            active: id === activeViewId,
            onClick: () => handleViewClick(id),
            trailing: id === 'profile'
                ? html`<span class="sidebar-status-dot" data-health=${syncStatus.health}
                    title=${syncStatus.health === 'healthy' ? 'Cloud sync healthy'
                        : syncStatus.health === 'attention' ? 'Cloud sync needs review'
                        : syncStatus.health === 'error' ? 'Cloud sync error'
                        : 'Cloud sync not configured'}></span>`
                : null,
        });
        return out;
    });
}, [viewOrder, activeViewId, handleViewClick, syncStatus.health]);

const { SidebarContext, value: sidebarValue } = useSidebarProvider({ defaultItems, defaultHeader });
```

Add `useMemo` to the preact/hooks import.

- [ ] **Step 3: Wrap render in provider, simplify SidebarNav props**

Wrap the outermost render element inside `AppShell` with the provider. Replace the `SidebarNav` line:

Before:
```js
<aside id="sidebar"><${SidebarNav} activeViewId=${activeViewId} viewOrder=${viewOrder}
    pluginOpen=${!!activePluginId} onViewClick=${handleViewClick}
    onBack=${closePluginConfig} profileSyncHealth=${syncStatus.health} /></aside>
```

After:
```js
<aside id="sidebar"><${SidebarNav} /></aside>
```

The full `AppShell` return becomes:
```js
return html`
    <${SidebarContext.Provider} value=${sidebarValue}>
    <${ModifierStateProvider}>
    <${PluginConfigProvider} pluginId=${activePluginId} mode=${activePluginMode}>
        ...everything unchanged inside...
    <//>
    <//>
    <//>
`;
```

- [ ] **Step 4: Syntax check**

Run: `node --check ui/components/App.js`
Expected: clean

- [ ] **Step 5: Verify app loads with default sidebar**

At this point the default view list should render identically to before. Plugin config sidebar will be broken (fixed in Task 4).

- [ ] **Step 6: Commit**

```bash
git add ui/components/App.js
git commit -m "feat: wire SidebarContext provider with default view items"
```

---

### Task 4: Plugin config pushes sidebar items

**Files:**
- Modify: `ui/views/plugin-config/view.js`

- [ ] **Step 1: Add sidebar context push**

Add imports:

```js
import { useSidebarContext } from '../../components/app/sidebar-context.js';
import { html } from '../../lib/html.js';
```

Note: `html` is already imported. Just add `useSidebarContext`.

In `PluginConfigView`, after `const ctx = usePluginConfigContext();`, add:

```js
const { setItems, setHeader, resetSidebar } = useSidebarContext();

useEffect(() => {
    if (!ctx?.sections?.length) return;
    setItems(ctx.sections.map((s, i) => ({
        type: 'item',
        key: s.id,
        id: s.id,
        label: s.label || prettyLabel(s.id),
        active: i === ctx.activeSectionIndex,
        onClick: () => ctx.setActiveSectionIndex(i),
    })));
}, [ctx?.sections, ctx?.activeSectionIndex, setItems]);

useEffect(() => {
    if (!ctx) return;
    setHeader(html`<div class="sidebar-header"><button class="sidebar-back" tabIndex="-1" onClick=${onClose}>${'\u2190'} Back</button></div>`);
    return () => resetSidebar();
}, [ctx?.pluginId, setHeader, resetSidebar, onClose]);
```

Add `useEffect` to the preact/hooks import (currently has `useCallback, useRef`).

- [ ] **Step 2: Syntax check**

Run: `node --check ui/views/plugin-config/view.js`
Expected: clean

- [ ] **Step 3: Verify plugin config sidebar works**

Open a plugin config — sidebar should show section items with active highlight, back button. Switching sections should update the active state. Closing config should restore default view list.

- [ ] **Step 4: Commit**

```bash
git add ui/views/plugin-config/view.js
git commit -m "feat: plugin config pushes sidebar items via context"
```

---

### Task 5: Components catalog sidebar + single showcase rendering

**Files:**
- Modify: `ui/views/dev/components/DevLayout.js`
- Modify: `ui/views/dev/components/ComponentsCatalog.js`

- [ ] **Step 1: DevLayout pushes catalog items when Components tab active**

Add imports to DevLayout.js:

```js
import { useSidebarContext } from '../../../components/app/sidebar-context.js';
```

Add `useEffect` to the preact/hooks import.

Inside `DevLayout`, after `const vtRef = useRef(null);`:

```js
const { setItems, resetSidebar } = useSidebarContext();
const [catalogId, setCatalogId] = useState('buttons');

useEffect(() => {
    if (vtRef.current?.activeTab !== 'components') {
        resetSidebar();
        return;
    }
    const CATALOG_ITEMS = [
        { id: 'buttons', label: 'Buttons' },
        { id: 'status', label: 'Status' },
        { id: 'spinner', label: 'Spinner' },
        { id: 'empty-state', label: 'Empty State' },
        { id: 'dropdown', label: 'Dropdown' },
        { id: 'expander', label: 'Expander' },
        { id: 'toggle', label: 'Toggle' },
        { id: 'modal', label: 'Modal' },
        { id: 'depth-diver', label: 'Depth Diver' },
        { id: 'dev-plugin-row', label: 'Dev Plugin Row' },
        { id: 'log-row', label: 'Log Row' },
        { id: 'suppressed-row', label: 'Suppressed Row' },
        { id: 'backup-row', label: 'Backup Row' },
        { id: 'hotkey-row', label: 'Hotkey Row' },
        { id: 'shortcut-row', label: 'Shortcut Row' },
        { id: 'store-card', label: 'Store Card' },
    ];
    setItems(CATALOG_ITEMS.map(item => ({
        type: 'item',
        key: item.id,
        id: item.id,
        label: item.label,
        active: item.id === catalogId,
        onClick: () => setCatalogId(item.id),
    })));
    return () => resetSidebar();
}, [vtRef.current?.activeTab, catalogId, setItems, resetSidebar]);
```

Add `useState` and `useEffect` to imports. Pass `catalogId` to ComponentsCatalog:

Change the Components tab render from:
```js
${vt.activeTab === 'components' && html`<${ComponentsCatalog} />`}
```
To:
```js
${vt.activeTab === 'components' && html`<${ComponentsCatalog} activeId=${catalogId} />`}
```

- [ ] **Step 2: Update ComponentsCatalog to render single showcase**

In ComponentsCatalog.js, change the export to accept `activeId` and render only the matching showcase:

```js
export function ComponentsCatalog({ activeId }) {
    const showcases = {
        'buttons': ButtonShowcase,
        'status': StatusShowcase,
        'spinner': SpinnerShowcase,
        'empty-state': EmptyStateShowcase,
        'dropdown': DropdownShowcase,
        'expander': ExpanderShowcase,
        'toggle': ToggleShowcase,
        'modal': ModalShowcase,
        'depth-diver': DepthDiver,
        'dev-plugin-row': DevPluginRowShowcase,
        'log-row': LogRowShowcase,
        'suppressed-row': SuppressedRowShowcase,
        'backup-row': BackupRowShowcase,
        'hotkey-row': HotkeyTableShowcase,
        'shortcut-row': ShortcutTableShowcase,
        'store-card': StoreCardShowcase,
    };
    const Showcase = showcases[activeId];
    if (!Showcase) return null;
    return html`<div class="catalog"><${Showcase} /></div>`;
}
```

Remove the `CatalogGroup` component and its usage (no longer needed — each showcase renders standalone). Keep `CatalogSection`, `StateLabel`, and all individual showcase functions unchanged.

- [ ] **Step 3: Syntax check both files**

Run: `node --check ui/views/dev/components/DevLayout.js && node --check ui/views/dev/components/ComponentsCatalog.js`
Expected: clean

- [ ] **Step 4: Verify catalog works**

Navigate to Dev → Components tab. Sidebar should show component list. Clicking an item shows that showcase. Switching to Dev tab restores the default sidebar.

- [ ] **Step 5: Commit**

```bash
git add ui/views/dev/components/DevLayout.js ui/views/dev/components/ComponentsCatalog.js
git commit -m "feat: components catalog uses sidebar picker for showcase selection"
```

---

### Task 6: Cleanup

**Files:**
- Verify: all modified files

- [ ] **Step 1: Verify no stale imports**

Check that `SidebarNav.js` no longer imports from `plugin-config/context.js` or `auto-config/heuristics.js`.

Check that `App.js` no longer passes props to `SidebarNav` that it doesn't accept.

- [ ] **Step 2: Full syntax check**

Run:
```bash
node --check ui/components/app/sidebar-context.js && \
node --check ui/components/SidebarNav.js && \
node --check ui/components/App.js && \
node --check ui/views/plugin-config/view.js && \
node --check ui/views/dev/components/DevLayout.js && \
node --check ui/views/dev/components/ComponentsCatalog.js
```

Expected: all clean

- [ ] **Step 3: Final commit if any cleanup needed**

```bash
git add -A && git commit -m "chore: cleanup stale imports after sidebar context migration"
```
