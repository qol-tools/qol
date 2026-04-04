# World Viewport — Milestone A

## Problem

The current scroll-based document model forces responsive layout gymnastics for every viewport size, creates scroll-follows-focus complexity, and makes the UI feel like a stack of pages instead of a unified workspace.

## Solution

Replace the scroll-based `#content` area with an infinite CSS-transform canvas ("World"). Views are positioned regions in 2D world-space. A camera controls which slice of the world is visible. No scrolling anywhere — the camera pans.

## Architecture

### DOM Structure

```
.app-container
  #viewport              overflow:hidden, full width (no sidebar)
    #world               transform: translate(-camX, -camY)
      [view regions]     position:absolute, left/top from registry
  SelectionCursorOverlay screen-space, reads camera position
  Minimap                screen-space, fixed corner
```

No sidebar. The sidebar is replaced by three wayfinding systems (see Navigation).

### Camera

Imperative object, not Preact state. Camera position changes 60fps during panning — re-rendering the Preact tree per frame is wasteful.

```
camera = { x, y }
```

`panTo(x, y)` — instant jump. Sets `world.style.transform = translate(-x, -y)` directly.

`panSmooth(targetX, targetY, duration)` — interpolates via `requestAnimationFrame`. Used for keyboard nav auto-follow, command palette jumps, edge panning.

Camera state is read by the wedge overlay and spatial nav via a shared ref, not context.

### World Container

A single `<div id="world">` with `transform: translate(-camX, -camY)`. One GPU-composited transform. No layout reflow when the camera moves.

Views inside are positioned with `position: absolute; left: Xpx; top: Ypx` based on their registry coordinates.

### View Registry

Each view has a world-space entry:

```
{ id: 'dev', x: 0, y: 0, width: 1200, height: 900 }
{ id: 'plugins', x: 1400, y: 0, width: 1200, height: 800 }
{ id: 'store', x: 2800, y: 0, width: 1200, height: 700 }
...
```

Default layout: horizontal strip with 200px gaps. The layout engine assigns positions — views do not choose their own.

### Spatial Index

Tracks occupied rectangles in world-space. Used for:
- Placing new views in unoccupied space (O(1) lookup, not scan)
- Detecting the "active view" (which view region contains the viewport center)
- Minimap rendering

The index is derived state — view positions are the source of truth. If the index becomes corrupt (shouldn't happen, but safety net), edit-mode forces the user to resolve overlaps, then the index is rebuilt from actual positions on edit-mode exit.

### Performance

Views outside the viewport get `content-visibility: auto` — the browser skips rendering for off-screen subtrees. DOM stays mounted (state preserved), only the visible region pays paint cost.

## Input Model

### Default Mode — Content Interaction

Everything inside the world is interactive normally. Click buttons, type in inputs, navigate surfaces. The camera is invisible.

Keyboard arrow navigation moves between surfaces (unchanged). When focus moves to an element outside the viewport, the camera pans smoothly to show it — replacing the old `scrollForKeyboardSelection`.

### Edge Panning — Ambient Discovery

When the mouse cursor reaches the viewport edges, the camera pans in that direction. Speed scales with proximity to the edge (closer = faster). No click needed.

Only active when the world extends beyond the viewport in that direction. Dead zone = everything except the edge strip (~20px from edge).

### CTRL Mode — Explicit Viewport Control

Hold CTRL to enter viewport navigation mode. World content becomes inert (pointer-events suppressed).

- CTRL + Arrow keys: pan the camera
- CTRL + Wheel: reserved for zoom (milestone C, inactive in A)

Release CTRL to return to default mode.

## Navigation (Replaces Sidebar)

Three layered wayfinding systems, no sidebar:

### 1. Command Palette (Macro — jump anywhere)

The existing CTRL+E command palette gains world-coordinate awareness. Items include:
- View-level landmarks: "Developer", "Plugins", "Store"
- Region-level landmarks: "Components", "Logs", "Actions"

Selecting a landmark pans the camera smoothly to that region. The command palette is the primary random-access navigation.

### 2. Region Labels (Meso — see what's nearby)

View titles rendered directly in world-space as large, persistent labels above each view region. Visible from a distance — when you're in the Dev view, you can see "Plugins" and "Store" labels at the edges of your viewport. Environmental wayfinding.

Region-level labels (sections within a view) are smaller, visible when closer.

### 3. Minimap (Micro — persistent overview)

Small persistent overview in a viewport corner. Shows:
- All view bounding boxes as colored rectangles
- Current viewport as a highlighted rectangle
- View names as tiny labels

Click-to-jump on the minimap pans the camera to that location. The minimap is always visible, always in screen-space.

### 4. Zoom-to-fit (Reorientation)

Single hotkey (Shift+1 or Home) resets camera to show all view regions. Instant reorientation when lost. Pans smoothly to the overview position.

## Systems That Change

### SelectionCursorOverlay (wedge)

Currently: positions relative to `.app-container` using `getBoundingClientRect()`.

World: `getBoundingClientRect()` on elements inside `#world` already returns screen-space coordinates (the browser accounts for ancestor CSS transforms). The wedge calculation `targetRect.left - appRect.left` continues to work because both rects are in screen-space. The `needsViewportTeleport` check changes — instead of checking scroll parent visibility, check if the target rect is within the viewport bounds.

Scroll event listener (`document.addEventListener('scroll')`) is replaced by a camera change callback. The camera notifies the overlay when it moves.

### Spatial Navigation

`getBoundingClientRect()` returns screen-space coords — these are correct for determining visual proximity regardless of the world transform. The cone-based distance calculations work unchanged.

The `scrollSurfaceIntoView` / `scrollForKeyboardSelection` system is replaced by camera auto-follow: when focus moves to an off-viewport surface, `panSmooth` centers it.

### Dissolve Canvas

Currently fixed-positioned, sized by `innerWidth/innerHeight`. This continues to work — the dissolve canvas is screen-space, not world-space. It covers the viewport regardless of camera position.

### useScrollIntoView Hook

Removed entirely. The camera auto-follow in the navigation system replaces it.

### View Mounting

Currently: all views stay mounted, toggled via `display:none`. 

World: all views stay mounted (positioned in world-space). No `display:none` — off-screen views are handled by `content-visibility: auto`. The `useMountedViews` hook is simplified or removed.

### useRouter

Hash format extends to encode camera position or active view region:
- `#dev` — camera at Dev view's coordinates
- `#dev/components/buttons` — camera at Dev, Components tab, Buttons showcase

On refresh, the router reads the hash, looks up the target view's world coordinates, and sets the initial camera position.

## What Does NOT Change

- Surface trait system (useSurface, Surface component, data-selected-surface)
- Spatial navigation algorithm (cone-based nearest-surface)
- Component hierarchy (DevPluginRow, LogRow, etc.)
- Command palette infrastructure (just gains landmark items)
- View internal structure (each view renders the same content, just no overflow:auto)

## Milestone Boundaries

**A (this spec):** Camera + viewport, view positioning, edge panning, CTRL panning, auto-follow focus, command palette landmarks (view + region level), region labels, minimap, zoom-to-fit. No sidebar.

**B (future):** Jump navigation with landing strategies (center, top-left, etc.), element-level and state-level landmarks, CTRL contextual overlays (action hints, jump target previews).

**C (future):** Uniform scaling / zoom. CTRL+wheel zoom. Sidebar scaling strategy for narrow screens. Responsive elimination.

**Edit mode (future):** User rearranges view positions via drag. Cannot exit while views overlap. Spatial index rebuilt on exit.

## Files

| File | Change |
|------|--------|
| `ui/components/app/world-camera.js` | New — camera state, panTo, panSmooth, edge panning, CTRL mode |
| `ui/components/app/world-registry.js` | New — view positions, spatial index, landmark registry |
| `ui/components/app/WorldViewport.js` | New — #viewport + #world DOM, camera wiring, content-visibility |
| `ui/components/app/Minimap.js` | New — corner minimap overlay |
| `ui/components/app/RegionLabels.js` | New — world-space view/region labels |
| `ui/components/App.js` | Replace sidebar + #content with WorldViewport |
| `ui/components/SidebarNav.js` | Removed |
| `ui/components/app/sidebar-context.js` | Removed (landmarks replace it) |
| `ui/components/SelectionCursorOverlay.js` | Adapt coordinate system, replace scroll listener |
| `ui/components/app/useAppKeyboardRouting.js` | Replace scroll-into-view with camera auto-follow |
| `ui/hooks/useScrollIntoView.js` | Removed |
| `ui/hooks/useRouter.js` | Extend hash to encode world position |
| `ui/styles/app-shell.css` | Remove sidebar layout, add viewport/world styles |
| `ui/palette/registry.js` | Register view + region landmarks |
