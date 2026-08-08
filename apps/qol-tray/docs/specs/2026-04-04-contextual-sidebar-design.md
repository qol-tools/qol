# Contextual Sidebar

## Problem

The main sidebar (`SidebarNav`) hardcodes mode-switching logic: it reads plugin config context directly, computes items differently for views vs config vs UI modes, and owns the data flow. This makes it impossible for other views (Components catalog, future features) to push their own sidebar content without adding more hardcoded modes.

## Design

Replace SidebarNav's internal mode logic with a context-driven push model. Any view can set sidebar items. SidebarNav becomes a pure renderer.

### SidebarContext

New file: `ui/components/app/sidebar-context.js`

```js
// Shape
{
  items: [{ id, label, active, onClick, trailing? }],
  setItems: (items) => void,
  header: VNode | null,
  setHeader: (header) => void,
  resetSidebar: () => void,   // restores default view list
}
```

- `App.js` wraps the app in `SidebarContext.Provider`
- Default items = the view list (Plugins, Store, Hotkeys, etc.) computed from `viewOrder` + `activeViewId`
- `resetSidebar()` clears overrides, restoring default items and header

### SidebarNav changes

`SidebarNav.js` becomes a pure renderer:
- Reads `items` and `header` from `SidebarContext`
- Renders `.sidebar-item` elements with `.active` class — same visual output as today
- Keeps the materialize animation (triggers when items reference changes)
- Loses: `usePluginConfigContext` import, mode computation, internal item building
- Gains: simplicity — ~40 lines instead of ~100

### View integration pattern

Each view that needs a contextual sidebar follows this pattern:

```js
function MyView() {
    const { setItems, setHeader, resetSidebar } = useSidebarContext();
    const [activeId, setActiveId] = useState('first');

    useEffect(() => {
        setHeader(html`<button class="sidebar-back" ...>← Back</button>`);
        return () => resetSidebar();
    }, []);

    useEffect(() => {
        setItems(MY_ITEMS.map(item => ({
            id: item.id,
            label: item.label,
            active: item.id === activeId,
            onClick: () => setActiveId(item.id),
        })));
    }, [activeId]);

    // render content based on activeId
}
```

### Plugin config integration

`PluginConfigView` pushes section items via context instead of SidebarNav reading config context:

```js
useEffect(() => {
    setItems(ctx.sections.map((s, i) => ({
        id: s.id,
        label: s.label || prettyLabel(s.id),
        active: i === ctx.activeSectionIndex,
        onClick: () => ctx.setActiveSectionIndex(i),
    })));
}, [ctx.sections, ctx.activeSectionIndex]);
```

The back button header is pushed via `setHeader`. On unmount, `resetSidebar()`.

### Components catalog integration

When the "Components" tab is active in DevLayout, push catalog items:

```js
const CATALOG_ITEMS = [
    { id: 'buttons', label: 'Buttons' },
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
    { id: 'status', label: 'Status' },
    { id: 'spinner', label: 'Spinner' },
    { id: 'empty-state', label: 'Empty State' },
];
```

`ComponentsCatalog` receives `activeId` as a prop and renders only the selected showcase. No more vertical scroll through all components. Each showcase gets full width — the two-column `catalog-showcase` layout (interactable | states) works without cramping.

### Default sidebar restoration

When no view has pushed items, the sidebar shows the default view list. This is computed in App.js from `viewOrder`, `activeViewId`, and `profileSyncHealth` — the same data SidebarNav uses today, just lifted up.

`resetSidebar()` is called:
- When plugin config unmounts
- When Components tab deactivates (switch to Dev tab)
- As cleanup in `useEffect` return

### What does NOT change

- `#sidebar` DOM element and CSS layout
- `.sidebar-item` / `.active` visual styling
- `.sidebar-back` button styling
- `materializeIn` animation on item transitions
- Main app sidebar width (`var(--size-sidebar)`)
- Keyboard navigation — sidebar items are not surfaces today and stay that way

## Files

| File | Change |
|------|--------|
| `ui/components/app/sidebar-context.js` | New — context + provider + hook |
| `ui/components/SidebarNav.js` | Simplify to pure renderer of context items |
| `ui/components/App.js` | Wrap in SidebarContext.Provider, compute default items |
| `ui/views/plugin-config/view.js` | Push section items via sidebar context |
| `ui/views/dev/components/DevLayout.js` | Push catalog items when Components tab active |
| `ui/views/dev/components/ComponentsCatalog.js` | Receive activeId, render single showcase |
