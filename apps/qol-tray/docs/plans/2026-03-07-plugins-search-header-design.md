# Design: Plugins Search Header

## Goal

Add a search bar to the Plugins page that filters the installed plugins grid. Extract the store's search bar into a reusable component. Update PageHeader to a proper two-column layout with an optional straddling badge for contextual page controls.

## Final Layout

```
PLUGINS
┌─────────────────────────────────────────────────────────┐
│ PLUGINS               [🔍 filter installed...         ] │
│                                                         │
└─────────────────────────────────────────────────────────┘

STORE
┌─────────────────────────────────────────────────────────┐
│ PLUGIN STORE          [🔍 filter store...            ]  │
│ Browse and install...                                   │
└──────────────────────────────────────────╔═════════════╝
                                           ║ t · 5m · ↻
                                           ╚═════════════
```

The badge straddles the bottom border of the header — positioned at bottom-right, translateY(50%). It is a pill-shaped container that visually pops out of the header. Zero layout impact on the search bar or other content.

## PageHeader API

```js
PageHeader({ title, subtitle, right, badge })
```

- `right` — content for the right column (search bar, or any JSX). Full width of col2, always.
- `badge` — optional pill anchored to the bottom-right border of the header. Store passes token/cache/refresh here. Plugins passes nothing.

## Component: SearchInput

New file: `ui/components/SearchInput.js`

Extracted from `StoreSearchBar` in `store/layout.js`. Props: `{ searchRef, value, onInput, placeholder }`. Renders a `.search-bar` div with a full-width input.

## Data Flow: Plugins Page

```
usePluginsList() → plugins[]
useState('')     → searchQuery + handleSearch
useMemo          → getFilteredPlugins(plugins, searchQuery)  ← reused from store/reducer.js
                 → filtered[]

PageHeader right=${SearchInput}
PluginsGrid plugins=${filtered}
```

## Files Changed

| File | Change |
|---|---|
| `ui/components/SearchInput.js` | new — extracted from StoreSearchBar |
| `ui/components/PageHeader.js` | add `right` + `badge` props; remove `actions` |
| `ui/styles/page-header.css` | 2-col layout; `.page-header-right`; `.page-header-badge` pill styles |
| `ui/views/store/layout.js` | use `SearchInput`; `right=` + `badge=` on PageHeader; remove StoreSearchBar |
| `ui/views/plugins-view.js` | search state + filter + SearchInput in header |
