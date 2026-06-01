# World Layers — Zoomable Depth Navigation

## Goal

Replace the modal system with spatial depth. All content exists at fixed coordinates in a layered world. "Opening a modal" becomes descending to a sub-page that was always there. The camera gains zoom, pages gain wide spacing, and one page (Hotkeys) gets a working dive to prove the concept end-to-end.

## Architecture

Camera: `(x, y, zoom)`. The world uses CSS `scale(zoom) translate(x, y)` on `#world`. All existing DOM content (Preact components, Surface hierarchy, form inputs) renders unchanged — CSS transforms handle the scaling. Pan speed divides by zoom for consistency.

Three rendering layers:
- **Canvas background** (`<canvas>` replacing `#world-bg`) — dot grid, depth-dependent visual effects, layer transition ambiance. Redrawn on camera change.
- **DOM world** (`#world`, CSS-transformed) — all view content. Pages, sub-pages, surfaces, forms. Positioned in world-space coordinates.
- **DOM overlay** (screen-space, no transform) — command palette, minimap, wedge cursor, toasts. Already exists at this layer.

## Registry as Scene Graph

The world registry gains depth awareness. Each entry has:
- `id` — unique identifier
- `x, y` — world-space position
- `layer` — depth level (0 = surface, -1 = first sub-layer, ...)
- `parent` — id of the parent entry (null for layer-0 pages)
- `width, height` — dimensions

Layer-0 pages are spaced 10,000px apart (currently 1,200px). At the surface zoom level (~0.12), they appear adjacent. Sub-pages are positioned within their parent page's 10,000px territory on deeper layers, spaced ~5,000px apart within that territory.

The registry pre-computes all positions at startup from a declarative manifest:
```
plugins:  { sub: [config, ui-panel] }
hotkeys:  { sub: [editor] }
store:    { sub: [install-confirm] }
...
```

Each page declares its sub-pages. The registry allocates positions: sub-pages go directly below their parent's origin, offset vertically by 2,000px, spaced horizontally by 5,000px within the territory.

## Camera Model

```js
createCamera() → { x, y, zoom, layer,
    panTo, panSmooth, zoomTo, zoomSmooth,
    nudge, subscribe, setWorldElement }
```

- `zoom` — float, 1.0 = normal. Layer 0 overview uses ~0.12. Layer -1 uses 1.0.
- `layer` — current depth level (integer, 0, -1, -2, ...).
- `zoomSmooth(targetZoom, duration)` — animated zoom to a target level.
- CSS transform: `scale(${zoom}) translate(${-x}px, ${-y}px)`.
- Pan speed: `dx / zoom`, `dy / zoom` — consistent feel at any zoom level.
- Camera subscribe notifies on any property change (position or zoom).

## Dive and Ascend

**Dive (Enter on an activatable surface that has a sub-page):**
1. The surface declares a sub-page target: `data-dive-target="hotkey-editor"`.
2. The keyboard routing detects Enter on a surface with `data-dive-target`.
3. The registry looks up the target entry (layer -1, specific coordinates).
4. Camera transition (approach A, default): smooth zoom from current zoom to 1.0, while panning to the sub-page position. Duration ~400ms, cubic ease-out.
5. `camera.layer` updates to -1.
6. Focus moves to the first surface in the sub-page.

**Ascend (Escape when on layer < 0):**
1. Escape at the top of a sub-page's surface hierarchy triggers ascend.
2. Camera transitions back: zoom from 1.0 to overview (~0.12), pan to parent page position.
3. `camera.layer` updates to 0.
4. Focus returns to the surface that initiated the dive (tracked in a dive stack).

**Approach B (dissolve) as alternative:**
Same coordinate math, but instead of visible zoom animation, crossfade between layers. Toggle via a preference or programmatically.

## Dive Stack

A stack tracks the dive path:
```
[{ layer: 0, viewId: 'hotkeys', surfaceIndex: 3, cameraState: {x, y, zoom} }]
```

On dive: push current state. On ascend: pop and restore. Supports arbitrary depth (-1, -2, ...) though milestone 1 only uses one level.

## Visibility and Culling

Only content on the current layer (±1 for transition) is rendered. Off-layer view slots get `display: none` (not `inert` — we learned that lesson). The camera subscribe callback updates visibility when `camera.layer` changes.

Layer-0 content at overview zoom is visible but tiny. Sub-page content on layer -1 is hidden until the camera descends. During the zoom transition, both layers are briefly visible (the parent shrinks away, the sub-page grows in).

## Minimap

The minimap shows the current layer's entries. A small depth indicator (e.g., "L0", "L-1") shows which layer the camera is on. Clicking a minimap region on the current layer pans to it. Layer switching via minimap is out of scope for milestone 1.

## Milestone 1 Scope

1. **Camera gains zoom** — `(x, y, zoom)` model, CSS `scale(zoom) translate(x, y)`.
2. **Page spacing widens** — 10,000px gaps, overview zoom ~0.12 on layer 0.
3. **Canvas background** — `<canvas>` replaces `#world-bg` for zoom-dependent dot grid.
4. **Registry gains layers** — entries have `layer`, `parent`. Manifest declares sub-pages.
5. **One working dive** — Hotkeys page → Hotkey Editor sub-page on layer -1. Enter descends, Escape ascends. Existing modal content rendered as-is in the sub-page container.
6. **Dive stack** — push/pop camera state on dive/ascend.
7. **Visibility culling** — only current layer's content renders.
8. **Pan speed scales with zoom** — consistent feel at any depth.

## Out of Scope

- Multiple sub-page layouts per page (just one for Hotkeys).
- Migrating all modals to sub-pages (incremental, after milestone 1).
- Layer switching via minimap.
- Semantic zoom (content changes form at different zoom levels).
- Layer +1 (above surface).
- Approach B (dissolve transition) — can be added later as the same coordinate math applies.

## Files Modified

| File | Change |
|------|--------|
| `lib/world-camera.js` | Add `zoom`, `layer`, `zoomSmooth`, `zoomTo`. Transform becomes `scale(z) translate(x, y)`. Pan divides by zoom. |
| `lib/world-registry.js` | Entries gain `layer`, `parent`. `createWorldRegistry` accepts manifest with sub-pages. New methods: `getEntriesForLayer(n)`, `getSubPages(parentId)`, `diveTarget(id)`. Spacing changes to 10,000px. |
| `lib/viewport-spatial.js` | `slotAtCenter` and `nearestSurfaceToCenter` filter by current camera layer. |
| `components/app/WorldViewport.js` | CSS transform includes zoom. Pan speed divides by zoom. Dive/ascend keyboard handling (Enter/Escape at layer boundary). Canvas background element. |
| `components/app/useAppKeyboardRouting.js` | Enter on `[data-dive-target]` triggers dive. Escape at layer < 0 triggers ascend. |
| `components/SelectionCursorOverlay.js` | Wedge positioning accounts for zoom in `cursorStyle`. CTRL preview accounts for zoom. |
| `components/app/Minimap.js` | Layer indicator. Scale-aware drawing. |
| `components/app/views.js` | `renderWorldViews` includes sub-page slots. Visibility culling by layer. |
| `components/App.js` | Pass layer state. Dive stack management. |
| `views/hotkeys-view.js` | Hotkey edit form rendered as a sub-page slot instead of a modal. Surface declares `data-dive-target`. |
| `styles/world.css` | Canvas background element. Zoom-dependent styles. |

## Verification

1. Layer 0: pages appear side-by-side at overview zoom. Arrow/Tab navigation works.
2. CTRL+arrow pan at overview zoom moves at consistent speed.
3. Enter on a hotkey row → camera zooms in and pans to the editor sub-page on layer -1.
4. Escape from the editor → camera zooms out and returns to the hotkey row.
5. Minimap shows current layer, click-to-pan works.
6. Wedge tracks correctly at all zoom levels.
7. Command palette "Go to X" commands work at overview zoom.
