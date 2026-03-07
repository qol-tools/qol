# Plugins Search Header — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract the store's search bar into a reusable `SearchInput` component, update `PageHeader` to a two-column layout with a straddling badge slot, and wire up a search bar on the Plugins page that filters the installed plugins grid.

**Architecture:** `PageHeader` gains a `right` slot (always-full-width content, e.g. search bar) and an optional `badge` slot (pill straddling the bottom border, for page-specific controls). `SearchInput` is a new shared component used by both the store and the plugins pages. Filtering on the plugins page lives in `plugins-view.js` using existing helpers from `store/reducer.js`.

**Tech Stack:** Preact (via `preact.module.js`), htm tagged template literals (via `lib/html.js`), plain CSS custom properties.

---

### Task 1: Create `SearchInput` component

**Files:**
- Create: `ui/components/SearchInput.js`
- Modify: `ui/styles/common-controls.css` (replace `.search-bar` input rules with `.search-input`)

**Step 1: Create the component**

```js
import { html } from '../lib/html.js';

export function SearchInput({ searchRef, value, onInput, placeholder }) {
    return html`<input class="search-input" type="text" ref=${searchRef}
        value=${value} onInput=${onInput} placeholder=${placeholder ?? 'Search...'} />`;
}
```

**Step 2: Add `.search-input` CSS to `ui/styles/common-controls.css`**

Add after the existing `.badge-build-skip` block:

```css
.search-input {
    width: 100%;
    padding: var(--space-3) var(--space-4);
    font-size: var(--fs-base);
    border: var(--border-w-2) solid var(--border-subtle);
    border-radius: var(--radius-lg);
    background: var(--bg-surface);
    color: var(--text-secondary);
    box-sizing: border-box;
    transition: border-color var(--dur-slow);
    font-family: inherit;
}

.search-input:focus {
    outline: none;
    border-color: var(--accent);
    color: var(--text-primary);
}

.search-input::placeholder {
    color: var(--text-faint);
}
```

Then **remove** the now-unused `.search-bar`, `.store-search-bar`, `.search-bar input`, and `.search-bar input:focus` blocks entirely (they will be replaced by `.search-input` and are no longer referenced).

**Step 3: Commit**

```bash
git add ui/components/SearchInput.js ui/styles/common-controls.css
git commit -m "feat: add SearchInput component and search-input CSS"
```

---

### Task 2: Update `PageHeader` — props and DOM structure

**Files:**
- Modify: `ui/components/PageHeader.js`

**Current:**
```js
export function PageHeader({ title, subtitle = '', actions = null, className = '' }) {
    const cls = className ? `page-header ${className}` : 'page-header';
    return html`
        <div class=${cls}>
            <div class="page-header-main">
                <h1>${title}</h1>
                <p>${subtitle}</p>
            </div>
            ${actions ? html`<div class="page-header-actions">${actions}</div>` : ''}
        </div>
    `;
}
```

**Replace with:**
```js
export function PageHeader({ title, subtitle = '', right = null, badge = null, className = '' }) {
    const cls = className ? `page-header ${className}` : 'page-header';
    return html`
        <div class=${cls}>
            <div class="page-header-main">
                <h1>${title}</h1>
                <p>${subtitle}</p>
            </div>
            ${right ? html`<div class="page-header-right">${right}</div>` : ''}
            ${badge ? html`<div class="page-header-badge">${badge}</div>` : ''}
        </div>
    `;
}
```

**Step 2: Commit**

```bash
git add ui/components/PageHeader.js
git commit -m "refactor: rename PageHeader actions→right, add badge slot"
```

---

### Task 3: Update `page-header.css` — layout and badge styles

**Files:**
- Modify: `ui/styles/page-header.css`

**Replace the entire file with:**

```css
.page-header {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-5);
    padding: var(--space-4) var(--space-7);
    border-bottom: var(--border-w-1) solid var(--border-subtle);
    backdrop-filter: blur(2px);
    flex-shrink: 0;
    min-height: var(--size-page-header);
    box-sizing: border-box;
}

.page-header-main {
    display: grid;
    grid-template-rows: auto 1.05em;
    row-gap: var(--space-2);
    min-width: 0;
    flex-shrink: 0;
}

.page-header-main h1 {
    color: var(--text-primary);
    font-size: clamp(1.05rem, 1.5vw, 1.4rem);
    line-height: var(--lh-tight);
    text-transform: uppercase;
    letter-spacing: var(--ls-3xl);
    margin: 0;
}

.page-header-main p {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-sm-plus);
    letter-spacing: var(--ls-tight);
    line-height: var(--lh-tight);
    min-height: 0;
}

.page-header-main p:empty {
    visibility: hidden;
}

.page-header-right {
    flex: 1;
    display: flex;
    align-items: center;
    min-width: 0;
}

.page-header-badge {
    position: absolute;
    bottom: 0;
    right: var(--space-7);
    transform: translateY(50%);
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-3);
    background: var(--bg-elevated);
    border: var(--border-w-1) solid var(--border-subtle);
    border-radius: var(--radius-pill);
    z-index: var(--z-1);
}

.cache-age {
    color: var(--text-faint);
    font-size: var(--fs-md-minus);
}
```

Key changes from the old file:
- Added `position: relative` to `.page-header` (required for badge absolute positioning)
- Changed `align-items: flex-end` → `align-items: center`
- Changed `padding: var(--space-4) var(--space-7) var(--space-2)` → `var(--space-4) var(--space-7)` (symmetric, badge handles bottom spacing)
- Renamed `.page-header-actions` → `.page-header-right` with `flex: 1` so it stretches
- Added `.page-header-badge` pill styles

**Step 2: Commit**

```bash
git add ui/styles/page-header.css
git commit -m "feat: update page-header layout with right slot and badge pill"
```

---

### Task 4: Update `store/layout.js` — use `SearchInput`, wire `right` and `badge`

**Files:**
- Modify: `ui/views/store/layout.js`

**Replace the entire file with:**

```js
import { html } from '../../lib/html.js';
import { formatCacheAge } from './reducer.js';
import { Feedback } from '../../components/FeedbackPreact.js';
import { PageHeader } from '../../components/PageHeader.js';
import { SearchInput } from '../../components/SearchInput.js';
import { StoreTokenPanel } from './token-panel.js';
import { StoreGrid } from './grid.js';

export function StoreLayout({ ctrl }) {
    return html`
        <div class="view-container">
            <${PageHeader} title="Plugin Store" subtitle="Browse and install plugins for QoL Tray"
                right=${html`<${SearchInput} searchRef=${ctrl.searchRef} value=${ctrl.searchQuery}
                    onInput=${ctrl.handleSearch} placeholder="Search plugins..." />`}
                badge=${html`<${StoreBadge} ...${ctrl} />`} />
            <div class="view-body">
                <${StoreTokenPanel} showTokenInput=${ctrl.showTokenInput} hasToken=${ctrl.hasToken}
                    rateLimited=${ctrl.rateLimited} tokenInputRef=${ctrl.tokenInputRef}
                    onSave=${ctrl.saveToken} onDelete=${ctrl.deleteToken}
                    onCancel=${ctrl.closeTokenInput} onShow=${ctrl.openTokenInput} />
                <${Feedback} feedback=${ctrl.feedback} />
                <${StoreGrid} plugins=${ctrl.filtered} loading=${ctrl.loading}
                    selectedIndex=${ctrl.selectedIndex} isInstalling=${ctrl.isInstalling}
                    onCardClick=${ctrl.handleCardClick} />
            </div>
        </div>
    `;
}

function StoreBadge({ cacheAgeSecs, loading, openTokenInput, refreshPlugins }) {
    return html`
        <span class="cache-age">${formatCacheAge(cacheAgeSecs)}</span>
        <button class="btn btn-ghost btn-sm" title="Manage GitHub token"
                onClick=${openTokenInput}>Token</button>
        <button class="refresh-btn ${loading ? 'spinning' : ''}" title="Refresh (r)"
                aria-label="Refresh" disabled=${loading} onClick=${refreshPlugins}></button>
    `;
}
```

Note: `StoreActions` is renamed `StoreBadge` to reflect its new placement. `StoreSearchBar` is removed entirely.

**Step 2: Commit**

```bash
git add ui/views/store/layout.js
git commit -m "refactor: move store search to PageHeader right slot, actions to badge"
```

---

### Task 5: Update `plugins-view.js` — search state, filter, SearchInput in header

**Files:**
- Modify: `ui/views/plugins-view.js`

**Replace the entire file with:**

```js
import { html } from '../lib/html.js';
import { useState, useMemo, useCallback, useEffect, useRef } from 'preact/hooks';
import { useFeedback } from '../hooks/useFeedback.js';
import { useFooterShortcuts } from '../hooks/useFooterShortcuts.js';
import { Feedback } from '../components/FeedbackPreact.js';
import { PageHeader } from '../components/PageHeader.js';
import { SearchInput } from '../components/SearchInput.js';
import { UninstallConfirmModal } from './plugins/confirm-modal.js';
import { PluginsGrid } from './plugins/grid.js';
import { usePluginsList } from './plugins/use-list.js';
import { usePluginsModal } from './plugins/use-modal.js';
import { usePluginActions } from './plugins/use-actions.js';
import { usePluginsKeyHandler } from './plugins/key-router.js';
import { useCardClickHandler } from './plugins/click-router.js';
import { getFilteredPlugins, normalizeSearchQuery } from './store/reducer.js';

const SHORTCUTS = [
    { key: '←↑↓→', label: 'navigate' },
    { key: 'Enter', label: 'settings' },
    { key: 'u', label: 'update' },
    { key: 'd', label: 'delete' },
    { key: 'm', label: 'menu' }
];

export function PluginsView({ onOpenPluginConfig }) {
    const { feedback, setFeedback, clearFeedback } = useFeedback();
    const list = usePluginsList(setFeedback);
    const [searchQuery, setSearchQuery] = useState('');
    const filtered = useMemo(
        () => getFilteredPlugins(list.plugins, searchQuery),
        [list.plugins, searchQuery]
    );
    const filteredRef = useRef(filtered);
    filteredRef.current = filtered;
    const handleSearch = useCallback(e => setSearchQuery(normalizeSearchQuery(e.target.value)), []);
    const searchRef = useRef(null);
    useEffect(() => {
        list.setSelectedIndex(prev => Math.min(prev, Math.max(0, filtered.length - 1)));
    }, [filtered.length]);
    const filteredList = { ...list, plugins: filtered, pluginsRef: filteredRef };
    const modal = usePluginsModal(filtered);
    const actions = usePluginActions(filteredList, modal, setFeedback, clearFeedback, onOpenPluginConfig);
    useFooterShortcuts(SHORTCUTS);
    PluginsView.handleKey = usePluginsKeyHandler(filteredList, modal, actions);
    PluginsView.isBlocking = actions.isBlocking;
    const handleCardClick = useCardClickHandler(filteredList, modal, actions);
    return html`<div class="view-container" onClick=${modal.closeAll}>
        <${PageHeader} title="Plugins"
            right=${html`<${SearchInput} searchRef=${searchRef} value=${searchQuery}
                onInput=${handleSearch} placeholder="Filter installed..." />`} />
        <div class="view-body">
            <${Feedback} feedback=${feedback} />
            <${PluginsGrid}
                plugins=${filtered} ghostPlugins=${list.ghostPlugins}
                selectedIndex=${list.selectedIndex} contextMenuOpen=${modal.contextMenuOpen}
                updating=${actions.updating} onCardClick=${handleCardClick} />
        </div>
        <${UninstallConfirmModal} plugin=${modal.confirmPlugin} pluginId=${modal.confirmPluginId}
            onClose=${modal.clearConfirm} onConfirm=${actions.confirmUninstall} />
    </div>`;
}
```

Key changes:
- Added `searchQuery`, `filtered`, `filteredRef`, `handleSearch`, `searchRef`
- `filteredList` spreads `list` but overrides `plugins` and `pluginsRef` with filtered versions so `use-actions.js` and `key-router.js` operate on visible items
- `usePluginsModal` and `usePluginActions` receive `filteredList` so actions look up plugins by index in the filtered set
- `PluginsGrid` receives `filtered` instead of `list.plugins`
- `PageHeader` gets `right=${SearchInput}`

**Step 2: Check `usePluginsModal` signature — make sure it accepts an array (not the list object)**

Read `ui/views/plugins/use-modal.js` to confirm it takes `plugins` array directly.
If it receives `list.plugins` today, pass `filtered` instead (already done above).

**Step 3: Commit**

```bash
git add ui/views/plugins-view.js
git commit -m "feat: add search bar to plugins header with client-side filtering"
```

---

### Task 6: Verify visually

Open the plugin store UI in a browser (or via the dev server if one exists). Check:

1. **Plugins page** — header shows full-width search input; typing filters the grid; clearing restores all plugins
2. **Store page** — header shows full-width search input; badge pill (Token · Xm ago · ↻) straddles the bottom border of the header; badge does not shift the search bar
3. **Both pages** — search bar is the same width and vertical position
4. **Badge** — visible only on store page; on plugins page the header bottom border is uninterrupted

No automated tests exist for the UI layer.
