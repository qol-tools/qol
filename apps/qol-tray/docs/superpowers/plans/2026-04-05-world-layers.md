# World Layers — Milestone 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace modals with spatial depth — camera gains zoom, pages spread to 10,000px, one working dive from Hotkeys to an editor sub-page on layer -1.

**Architecture:** CSS `scale(zoom) translate(x, y)` on `#world`. Registry entries gain `layer` and `parent` fields. Dive/ascend animate camera zoom+pan between layers. Canvas background replaces DOM dot grid.

**Tech Stack:** Preact + htm, CSS transforms, Canvas 2D, existing camera/registry/surface-trait system.

**Spec:** `docs/superpowers/specs/2026-04-05-world-layers-design.md`

---

## File Structure

| File | Status | Responsibility |
|------|--------|----------------|
| `ui/lib/world-camera.js` | Modify | Add `zoom`, `layer`, `zoomTo`, `zoomSmooth`, `setLayer`. Transform becomes `scale(z) translate(x,y)`. |
| `ui/lib/world-registry.js` | Modify | Entries gain `layer`, `parent`. 10k stride. Manifest-based sub-page allocation. New queries. |
| `ui/lib/dive-stack.js` | Create | Push/pop camera state for dive/ascend. |
| `ui/lib/world-canvas-bg.js` | Create | Canvas renderer for zoom-dependent dot grid background. |
| `ui/lib/viewport-spatial.js` | Modify | `slotAtCenter` and `nearestSurfaceToCenter` filter by camera layer. |
| `ui/components/app/WorldViewport.js` | Modify | Canvas bg, pan/drag/wheel divide by zoom, CTRL pan scales. |
| `ui/components/app/useAppKeyboardRouting.js` | Modify | Enter on `[data-dive-target]` triggers dive. Escape at layer < 0 ascends. |
| `ui/components/app/views.js` | Modify | Sub-page slots. Layer-based visibility culling. |
| `ui/components/app/Minimap.js` | Modify | Layer indicator, zoom-aware viewport rect. |
| `ui/components/app/WorldNav.js` | Modify | Jump commands pass zoom to cameraTargetForView. |
| `ui/components/App.js` | Modify | Sub-page manifest, overview zoom init, camera layer state, dive/ascend callbacks. |
| `ui/components/SelectionCursorOverlay.js` | Modify | Wedge positioning divides by zoom. CTRL preview divides by zoom. |
| `ui/views/hotkeys-view.js` | Modify | Editor content as sub-page slot. Remove modal rendering. |
| `ui/views/hotkeys/list.js` | Modify | Add `data-dive-target` on HotkeyRow. Always call onEdit on activate. |
| `ui/styles/world.css` | Modify | Canvas bg element styles. Remove old #world-bg dot grid. |

---

### Task 1: Camera zoom model

**Files:**
- Modify: `ui/lib/world-camera.js`

- [ ] **Step 1: Add zoom, layer state and zoomTo**

Replace the full contents of `ui/lib/world-camera.js`:

```js
import { createDebug } from './debug.js';

const log = createDebug('qol:camera');

export function createCamera() {
    let x = 0;
    let y = 0;
    let zoom = 1.0;
    let layer = 0;
    let worldEl = null;
    let animId = 0;
    let animFrom = null;
    let animTarget = null;
    let animStart = 0;
    let animDuration = 0;
    let animComplete = null;
    const listeners = new Set();

    function notify() {
        for (const fn of listeners) fn({ x, y, zoom, layer });
    }

    function apply() {
        if (worldEl) worldEl.style.transform = `scale(${zoom}) translate(${-x}px, ${-y}px)`;
        notify();
    }

    function panTo(nx, ny) {
        cancelSmooth();
        x = nx;
        y = ny;
        apply();
    }

    function panSmooth(tx, ty, duration) {
        cancelSmooth();
        animFrom = { x, y, zoom };
        animTarget = { x: tx, y: ty, zoom };
        animStart = performance.now();
        animDuration = duration;
        animId = requestAnimationFrame(tick);
    }

    function zoomTo(nz) {
        cancelSmooth();
        zoom = nz;
        apply();
    }

    function zoomSmooth(tx, ty, tz, duration, onComplete) {
        cancelSmooth();
        log('zoomSmooth →', Math.round(tx), Math.round(ty), 'z:', tz.toFixed(3), 'dur:', duration);
        animFrom = { x, y, zoom };
        animTarget = { x: tx, y: ty, zoom: tz };
        animStart = performance.now();
        animDuration = duration;
        animComplete = onComplete || null;
        animId = requestAnimationFrame(tick);
    }

    function cancelSmooth() {
        if (animId) { cancelAnimationFrame(animId); animId = 0; }
        animTarget = null;
        animComplete = null;
    }

    function tick(now) {
        if (!animTarget) return;
        const t = Math.min(1, (now - animStart) / animDuration);
        const e = 1 - Math.pow(1 - t, 3);
        x = animFrom.x + (animTarget.x - animFrom.x) * e;
        y = animFrom.y + (animTarget.y - animFrom.y) * e;
        zoom = animFrom.zoom + (animTarget.zoom - animFrom.zoom) * e;
        apply();
        if (t < 1) {
            animId = requestAnimationFrame(tick);
        } else {
            const cb = animComplete;
            animTarget = null;
            animId = 0;
            animComplete = null;
            if (cb) cb();
        }
    }

    function nudge(dx, dy) {
        cancelSmooth();
        x += dx;
        y += dy;
        apply();
    }

    function setLayer(n) {
        layer = n;
        notify();
    }

    return {
        get x() { return x; },
        get y() { return y; },
        get zoom() { return zoom; },
        get layer() { return layer; },
        get animating() { return animTarget !== null; },
        setWorldElement(el) { worldEl = el; },
        panTo,
        panSmooth,
        zoomTo,
        zoomSmooth,
        cancelSmooth,
        nudge,
        setLayer,
        subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); },
    };
}
```

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/lib/world-camera.js`
Expected: no output (clean syntax).

- [ ] **Step 3: Commit**

```bash
git add ui/lib/world-camera.js
git commit -m "feat: add zoom, layer, zoomSmooth to world camera"
```

---

### Task 2: Registry layers and wide spacing

**Files:**
- Modify: `ui/lib/world-registry.js`

- [ ] **Step 1: Rewrite registry with layer support**

Replace the full contents of `ui/lib/world-registry.js`:

```js
const PAGE_WIDTH = 1000;
const PAGE_HEIGHT = 800;
const PAGE_STRIDE = 10000;
const SUB_PAGE_Y_OFFSET = 2000;
const SUB_PAGE_X_SPACING = 5000;

export function createWorldRegistry(viewOrder, manifest = {}) {
    const entries = new Map();

    // Layer 0: pages at PAGE_STRIDE intervals
    for (let i = 0; i < viewOrder.length; i++) {
        const id = viewOrder[i];
        entries.set(id, {
            id, x: i * PAGE_STRIDE, y: 0,
            width: PAGE_WIDTH, height: PAGE_HEIGHT,
            layer: 0, parent: null,
        });
    }

    // Layer -1: sub-pages from manifest
    for (const [parentId, subs] of Object.entries(manifest)) {
        const parent = entries.get(parentId);
        if (!parent) continue;
        for (let i = 0; i < subs.length; i++) {
            const subId = `${parentId}-${subs[i]}`;
            entries.set(subId, {
                id: subId,
                x: parent.x + i * SUB_PAGE_X_SPACING,
                y: parent.y + SUB_PAGE_Y_OFFSET,
                width: PAGE_WIDTH, height: PAGE_HEIGHT,
                layer: -1, parent: parentId,
            });
        }
    }

    function getEntry(id) {
        return entries.get(id) || null;
    }

    function getAllEntries() {
        return Array.from(entries.values());
    }

    function getEntriesForLayer(n) {
        return getAllEntries().filter(e => e.layer === n);
    }

    function getSubPages(parentId) {
        return getAllEntries().filter(e => e.parent === parentId);
    }

    function diveTarget(id) {
        return entries.get(id) || null;
    }

    function worldBounds(layerFilter) {
        const pool = layerFilter !== undefined
            ? getAllEntries().filter(e => e.layer === layerFilter)
            : getAllEntries();
        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const e of pool) {
            minX = Math.min(minX, e.x);
            minY = Math.min(minY, e.y);
            maxX = Math.max(maxX, e.x + e.width);
            maxY = Math.max(maxY, e.y + e.height);
        }
        if (minX === Infinity) return { x: 0, y: 0, width: 0, height: 0 };
        const pad = 100;
        return { x: minX - pad, y: minY - pad, width: maxX - minX + pad * 2, height: maxY - minY + pad * 2 };
    }

    function activeViewId(cameraX, cameraY, viewportW, viewportH, zoom) {
        const z = zoom || 1;
        const cx = cameraX + viewportW / (2 * z);
        const cy = cameraY + viewportH / (2 * z);
        let closest = null;
        let closestDist = Infinity;
        for (const e of entries.values()) {
            if (e.layer !== 0) continue;
            const vx = e.x + e.width / 2;
            const vy = e.y + e.height / 2;
            const d = Math.hypot(cx - vx, cy - vy);
            if (d < closestDist) { closest = e.id; closestDist = d; }
        }
        return closest;
    }

    function placeNew(id, width, height) {
        const w = width || PAGE_WIDTH;
        const h = height || PAGE_HEIGHT;
        let maxRight = 0;
        for (const e of entries.values()) {
            if (e.layer !== 0) continue;
            maxRight = Math.max(maxRight, e.x + PAGE_STRIDE);
        }
        const entry = { id, x: maxRight, y: 0, width: w, height: h, layer: 0, parent: null };
        entries.set(id, entry);
        return entry;
    }

    function cameraTargetForView(id, viewportW, viewportH, zoom) {
        const e = entries.get(id);
        if (!e) return null;
        const z = zoom || 1;
        return {
            x: e.x + e.width / 2 - viewportW / (2 * z),
            y: e.y + e.height / 2 - viewportH / (2 * z),
        };
    }

    return {
        getEntry,
        getAllEntries,
        getEntriesForLayer,
        getSubPages,
        diveTarget,
        worldBounds,
        activeViewId,
        placeNew,
        cameraTargetForView,
    };
}
```

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/lib/world-registry.js`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add ui/lib/world-registry.js
git commit -m "feat: registry gains layers, 10k spacing, sub-page manifest"
```

---

### Task 3: Dive stack

**Files:**
- Create: `ui/lib/dive-stack.js`

- [ ] **Step 1: Create dive stack module**

Create `ui/lib/dive-stack.js`:

```js
export function createDiveStack() {
    const stack = [];

    function push(state) {
        stack.push(state);
    }

    function pop() {
        return stack.pop() || null;
    }

    function peek() {
        return stack[stack.length - 1] || null;
    }

    function clear() {
        stack.length = 0;
    }

    function depth() {
        return stack.length;
    }

    return { push, pop, peek, clear, depth };
}
```

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/lib/dive-stack.js`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add ui/lib/dive-stack.js
git commit -m "feat: add dive stack for camera state push/pop"
```

---

### Task 4: Canvas background

**Files:**
- Create: `ui/lib/world-canvas-bg.js`
- Modify: `ui/components/app/WorldViewport.js`
- Modify: `ui/styles/world.css`

- [ ] **Step 1: Create canvas background renderer**

Create `ui/lib/world-canvas-bg.js`:

```js
const DOT_SPACING = 50;
const DOT_SIZE = 1;
const DOT_ALPHA_BASE = 0.03;
const MIN_SCREEN_SPACING = 4;

export function createWorldCanvasBg(canvas, camera) {
    const ctx = canvas.getContext('2d');
    let mounted = true;

    function draw() {
        if (!mounted) return;
        const dpr = window.devicePixelRatio || 1;
        const w = canvas.clientWidth;
        const h = canvas.clientHeight;
        if (w === 0 || h === 0) return;
        if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
            canvas.width = w * dpr;
            canvas.height = h * dpr;
        }
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, w, h);

        const z = camera.zoom;
        const spacing = DOT_SPACING * z;
        if (spacing < MIN_SCREEN_SPACING) return;

        const alpha = DOT_ALPHA_BASE * Math.min(1, spacing / 20);
        ctx.fillStyle = `rgba(255, 255, 255, ${alpha})`;

        const offsetX = ((-camera.x % DOT_SPACING) + DOT_SPACING) % DOT_SPACING * z;
        const offsetY = ((-camera.y % DOT_SPACING) + DOT_SPACING) % DOT_SPACING * z;

        for (let x = offsetX; x < w; x += spacing) {
            for (let y = offsetY; y < h; y += spacing) {
                ctx.fillRect(x, y, DOT_SIZE, DOT_SIZE);
            }
        }
    }

    const unsub = camera.subscribe(() => draw());
    const ro = new ResizeObserver(() => draw());
    ro.observe(canvas);
    draw();

    return {
        destroy() {
            mounted = false;
            unsub();
            ro.disconnect();
        },
    };
}
```

- [ ] **Step 2: Update WorldViewport to use canvas background**

In `ui/components/app/WorldViewport.js`, add import at top:

```js
import { createWorldCanvasBg } from '../../lib/world-canvas-bg.js';
```

Add a `useEffect` for the canvas background after the existing `useEffect` that sets `camera.setWorldElement`:

```js
    const bgCanvasRef = useRef(null);
    useEffect(() => {
        if (!bgCanvasRef.current) return;
        const bg = createWorldCanvasBg(bgCanvasRef.current, camera);
        return () => bg.destroy();
    }, [camera]);
```

Replace the `#world-bg` div in the render with a canvas element. Change the render from:

```js
    return html`
        <div id="viewport" ref=${viewportRef}>
            <div id="world" ref=${worldRef}>
                <div id="world-bg"></div>
                ${children}
            </div>
        </div>
    `;
```

to:

```js
    return html`
        <div id="viewport" ref=${viewportRef}>
            <canvas id="world-bg" ref=${bgCanvasRef}></canvas>
            <div id="world" ref=${worldRef}>
                ${children}
            </div>
        </div>
    `;
```

Note: the canvas is a sibling of `#world`, not a child — it stays in screen-space (not transformed).

- [ ] **Step 3: Update CSS for canvas background**

In `ui/styles/world.css`, replace the `#world-bg` rule:

```css
#world-bg {
    position: absolute;
    inset: -10000px;
    background-image: radial-gradient(circle, rgba(255,255,255,0.02) 1px, transparent 1px);
    background-size: 50px 50px;
    pointer-events: none;
}
```

with:

```css
#world-bg {
    position: absolute;
    inset: 0;
    pointer-events: none;
    z-index: 0;
}
```

- [ ] **Step 4: Verify syntax**

Run: `node --check ui/lib/world-canvas-bg.js && node --check ui/components/app/WorldViewport.js`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add ui/lib/world-canvas-bg.js ui/components/app/WorldViewport.js ui/styles/world.css
git commit -m "feat: canvas background with zoom-dependent dot grid"
```

---

### Task 5: WorldViewport zoom integration

**Files:**
- Modify: `ui/components/app/WorldViewport.js`

- [ ] **Step 1: Pan speed, drag, and wheel divide by zoom**

In `ui/components/app/WorldViewport.js`, inside the `useEffect` that sets up event handlers:

**Pointer drag** — replace the `onPointerMove` body where it sets camera position. Change:

```js
            camera.panTo(d.camX - dx, d.camY - dy);
```

to:

```js
            camera.panTo(d.camX - dx / camera.zoom, d.camY - dy / camera.zoom);
```

**Wheel** — change the `onWheel` handler from:

```js
            camera.nudge(e.deltaX, e.deltaY);
```

to:

```js
            camera.nudge(e.deltaX / camera.zoom, e.deltaY / camera.zoom);
```

**CTRL pan loop** — change the speed calculation in `ctrlPanLoop` from:

```js
                if (keys.has('ArrowLeft')) dx = -PAN_SPEED;
                if (keys.has('ArrowRight')) dx = PAN_SPEED;
                if (keys.has('ArrowUp')) dy = -PAN_SPEED;
                if (keys.has('ArrowDown')) dy = PAN_SPEED;
```

to:

```js
                const speed = PAN_SPEED / camera.zoom;
                if (keys.has('ArrowLeft')) dx = -speed;
                if (keys.has('ArrowRight')) dx = speed;
                if (keys.has('ArrowUp')) dy = -speed;
                if (keys.has('ArrowDown')) dy = speed;
```

**Camera follow** — change the panSmooth call in `onFocusIn` from:

```js
            camera.panSmooth(camera.x + dx, camera.y + dy, 200);
```

to:

```js
            camera.panSmooth(camera.x + dx / camera.zoom, camera.y + dy / camera.zoom, 200);
```

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/components/app/WorldViewport.js`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add ui/components/app/WorldViewport.js
git commit -m "feat: pan/drag/wheel scale with zoom for consistent feel"
```

---

### Task 6: App.js integration — manifest, overview zoom, layer state

**Files:**
- Modify: `ui/components/App.js`
- Modify: `ui/components/app/WorldNav.js`

- [ ] **Step 1: Add sub-page manifest and overview zoom init**

In `ui/components/App.js`:

Add import for `createDiveStack`:

```js
import { createDiveStack } from '../lib/dive-stack.js';
```

Add import for `useState`:

```js
import { useRef, useCallback, useEffect, useState } from 'preact/hooks';
```

Inside `AppShell`, after `const registry = registryRef.current;`, add dive stack:

```js
    const diveStackRef = useRef(null);
    if (!diveStackRef.current) diveStackRef.current = createDiveStack();
    const diveStack = diveStackRef.current;
```

Add camera layer state:

```js
    const [cameraLayer, setCameraLayer] = useState(0);
```

Change the `createWorldRegistry` call from:

```js
    if (!registryRef.current) registryRef.current = createWorldRegistry(viewOrder);
```

to:

```js
    const SUB_PAGE_MANIFEST = { hotkeys: ['editor'] };
    if (!registryRef.current) registryRef.current = createWorldRegistry(viewOrder, SUB_PAGE_MANIFEST);
```

**Overview zoom init:** Replace the initial camera position effect:

```js
    useEffect(() => {
        const vp = viewportRef.current;
        const target = registry.cameraTargetForView(activeViewId, vp?.clientWidth || 800, vp?.clientHeight || 600);
        if (target) camera.panTo(target.x, target.y);
    }, []);
```

with:

```js
    const OVERVIEW_ZOOM = 0.12;

    useEffect(() => {
        camera.zoomTo(OVERVIEW_ZOOM);
        const vp = viewportRef.current;
        const target = registry.cameraTargetForView(activeViewId, vp?.clientWidth || 800, vp?.clientHeight || 600, OVERVIEW_ZOOM);
        if (target) camera.panTo(target.x, target.y);
    }, []);
```

**View change pan:** Update the `prevViewRef` effect to pass zoom. Change:

```js
            const target = registry.cameraTargetForView(activeViewId, w, h);
```

to:

```js
            const target = registry.cameraTargetForView(activeViewId, w, h, camera.zoom);
```

**Dive and ascend callbacks:**

After the `useWorldNav` call, add:

```js
    const dive = useCallback((targetId, sourceSurface) => {
        const entry = registry.diveTarget(targetId);
        if (!entry) return;
        diveStack.push({
            layer: camera.layer,
            x: camera.x, y: camera.y, zoom: camera.zoom,
            surfaceSelector: sourceSurface ? selectorFor(sourceSurface) : null,
        });
        const vp = viewportRef.current;
        const w = vp?.clientWidth || 800;
        const h = vp?.clientHeight || 600;
        setCameraLayer(entry.layer);
        camera.setLayer(entry.layer);
        const target = registry.cameraTargetForView(targetId, w, h, 1.0);
        if (target) {
            camera.zoomSmooth(target.x, target.y, 1.0, 400, () => {
                const slot = document.querySelector(`.world-view-slot[data-view-id="${targetId}"]`);
                const surface = slot?.querySelector('[data-selected-surface]');
                if (surface) surface.focus({ preventScroll: true });
            });
        }
    }, [camera, registry, diveStack]);

    const ascend = useCallback(() => {
        const prev = diveStack.pop();
        if (!prev) return false;
        setCameraLayer(prev.layer);
        camera.setLayer(prev.layer);
        camera.zoomSmooth(prev.x, prev.y, prev.zoom, 400, () => {
            if (prev.surfaceSelector) {
                const surface = document.querySelector(prev.surfaceSelector);
                if (surface) surface.focus({ preventScroll: true });
            }
        });
        return true;
    }, [camera, diveStack]);
```

Add the `selectorFor` helper (outside `AppShell`, at module level):

```js
function selectorFor(el) {
    if (el.id) return `#${CSS.escape(el.id)}`;
    const viewId = el.closest('[data-view-id]')?.dataset?.viewId;
    const index = el.getAttribute('data-index');
    if (viewId && index != null) {
        return `[data-view-id="${CSS.escape(viewId)}"] [data-selected-surface][data-index="${index}"]`;
    }
    return null;
}
```

Pass `dive`, `ascend`, `cameraLayer` in the template. Update `AppKeyboardRouting`:

```js
                <${AppKeyboardRouting}
                    activePluginId=${activePluginId}
                    activeViewId=${activeViewId}
                    camera=${camera}
                    closePluginConfig=${closePluginConfig}
                    switchView=${switchView}
                    viewOrder=${viewOrder}
                    dive=${dive}
                    ascend=${ascend}
                />
```

Update the `AppKeyboardRouting` component to receive and pass the new props:

```js
function AppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder, dive, ascend }) {
    const palette = usePaletteContext();
    useAppKeyboardRouting({ activePluginId, activeViewId, camera, closePluginConfig, switchView, viewOrder, palette, dive, ascend });
    return null;
}
```

Pass `cameraLayer` to `renderWorldViews`:

```js
                        ${renderWorldViews({
                            registry,
                            cameraLayer,
                            openPluginConfig,
                            openPluginUi,
                            syncStatus,
                            syncProviders,
                            onSyncStatusChange: setSyncStatus,
                            refreshSyncStatus,
                        })}
```

- [ ] **Step 2: Update WorldNav to pass zoom**

In `ui/components/app/WorldNav.js`, update `jumpToView` to pass camera zoom:

Change:

```js
        const target = registry.cameraTargetForView(id, w, h);
```

to:

```js
        const target = registry.cameraTargetForView(id, w, h, camera.zoom);
```

Update `fitAll` to pass zoom:

Change:

```js
        camera.panSmooth(
            bounds.x + bounds.width / 2 - w / 2,
            bounds.y + bounds.height / 2 - h / 2,
            400
        );
```

to:

```js
        const z = camera.zoom;
        camera.panSmooth(
            bounds.x + bounds.width / 2 - w / (2 * z),
            bounds.y + bounds.height / 2 - h / (2 * z),
            400
        );
```

Pass `camera.zoom` for layer-0 bounds:

Change:

```js
        const bounds = registry.worldBounds();
```

to:

```js
        const bounds = registry.worldBounds(0);
```

- [ ] **Step 3: Verify syntax**

Run: `node --check ui/components/App.js && node --check ui/components/app/WorldNav.js`
Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add ui/components/App.js ui/components/app/WorldNav.js
git commit -m "feat: overview zoom init, dive/ascend callbacks, manifest wiring"
```

---

### Task 7: Sub-page slots, visibility culling, shared edit state

**Files:**
- Modify: `ui/components/app/views.js`
- Modify: `ui/views/hotkeys-view.js`
- Modify: `ui/views/hotkeys/modal.js`

**Important context:** `HotkeysView` and `HotkeyEditorSubPage` are in different world-space slots (different DOM subtrees). Calling `useHotkeys()` in both would create two independent state instances. Instead, `HotkeysView` owns the state via `useHotkeys()` and syncs it to a module-level shared object. `HotkeyEditorSubPage` reads from that shared object and subscribes for re-renders.

- [ ] **Step 1: Add shared edit state mechanism to hotkeys-view.js**

In `ui/views/hotkeys-view.js`, add module-level shared state before the `HotkeysView` function:

```js
import { useState, useEffect } from 'preact/hooks';

// Shared state: HotkeysView writes, HotkeyEditorSubPage reads
const _sharedEdit = { modal: null, plugins: [], fieldProps: () => ({}), handlers: {} };
const _editListeners = new Set();
function notifyEditChange() { for (const fn of _editListeners) fn(); }
function subscribeEditState(fn) { _editListeners.add(fn); return () => _editListeners.delete(fn); }
```

Inside `HotkeysView`, after `const hk = useHotkeys();`, add a sync effect:

```js
    useEffect(() => {
        _sharedEdit.modal = hk.editModal;
        _sharedEdit.plugins = hk.plugins;
        _sharedEdit.fieldProps = hk.fieldProps;
        _sharedEdit.handlers = {
            onPluginChange: hk.handlePluginChange,
            onActionChange: hk.handleActionChange,
            onStartRecording: hk.startRecording,
            onClose: hk.closeModal,
            onSave: hk.saveHotkey,
        };
        notifyEditChange();
    }, [hk.editModal, hk.plugins]);
```

Remove the modal rendering from `HotkeysView`. Delete this line:

```js
            ${hk.editModal && html`<${HotkeyEditModal} modal=${hk.editModal} plugins=${hk.plugins}
                fieldProps=${hk.fieldProps} onPluginChange=${hk.handlePluginChange} onActionChange=${hk.handleActionChange}
                onStartRecording=${hk.startRecording} onClose=${hk.closeModal} onSave=${hk.saveHotkey} />`}
```

And remove the `HotkeyEditModal` import since it's no longer used in this file.

- [ ] **Step 2: Create HotkeyEditorSubPage using shared state**

Add to the bottom of `ui/views/hotkeys-view.js`:

```js
export function HotkeyEditorSubPage() {
    const [, bump] = useState(0);
    useEffect(() => subscribeEditState(() => bump(t => t + 1)), []);

    const { modal, plugins, fieldProps, handlers } = _sharedEdit;
    if (!modal) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Hotkey Editor" subtitle="Select a hotkey to edit" />
        </div>`;
    }
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Edit Hotkey" subtitle=${`Editing: ${modal.key || 'new hotkey'}`} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <div class="edit-modal-content">
                            <div class="form-group" ...${fieldProps(0)}>
                                <label>Plugin</label>
                                <${PluginSelect} modal=${modal} plugins=${plugins} onChange=${handlers.onPluginChange} />
                            </div>
                            <div class="form-group" ...${fieldProps(1)}>
                                <label>Action</label>
                                <${ActionSelect} modal=${modal} onChange=${handlers.onActionChange} />
                            </div>
                            <div class="form-group" ...${fieldProps(2)}>
                                <label>Shortcut</label>
                                <${KeyInput} modal=${modal} onStartRecording=${handlers.onStartRecording} />
                            </div>
                        </div>
                    <//>
                </div>
            </div>
        </div>
    `;
}
```

- [ ] **Step 3: Export sub-components from modal.js**

In `ui/views/hotkeys/modal.js`, add `export` to three function declarations:

Change `function PluginSelect(` to `export function PluginSelect(`

Change `function ActionSelect(` to `export function ActionSelect(`

Change `function KeyInput(` to `export function KeyInput(`

Add the import in `hotkeys-view.js`:

```js
import { PluginSelect, ActionSelect, KeyInput } from './hotkeys/modal.js';
```

- [ ] **Step 4: Add layer attribute and visibility culling to WorldViewSlot**

In `ui/components/app/views.js`, update `WorldViewSlot`:

```js
function WorldViewSlot({ entry, cameraLayer, children }) {
    if (!entry) return null;
    const visible = entry.layer === cameraLayer;
    const style = `left:${entry.x}px; top:${entry.y}px; width:${entry.width}px;${visible ? '' : ' display:none;'}`;
    return html`<div class="world-view-slot" data-view-id=${entry.id} data-layer=${entry.layer} style=${style}>${children}</div>`;
}
```

- [ ] **Step 5: Add sub-page slot and pass cameraLayer to renderWorldViews**

Import `HotkeyEditorSubPage` at top of `views.js`:

```js
import { HotkeyEditorSubPage } from '../../views/hotkeys-view.js';
```

Update `renderWorldViews` to accept `cameraLayer` and pass it to all slots. Add the sub-page slot:

```js
export function renderWorldViews({ registry, cameraLayer, openPluginConfig, openPluginUi, syncStatus, syncProviders, onSyncStatusChange, refreshSyncStatus }) {
    const layer = cameraLayer != null ? cameraLayer : 0;
    return html`
        <${WorldViewSlot} entry=${registry.getEntry('plugins')} cameraLayer=${layer}><${PluginsView} onOpenPluginConfig=${openPluginConfig} onOpenPluginUi=${openPluginUi} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('store')} cameraLayer=${layer}><${StoreView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys')} cameraLayer=${layer}><${HotkeysView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('shortcuts')} cameraLayer=${layer}><${ShortcutsView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('task-runner')} cameraLayer=${layer}><${TaskRunnerView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('profile')} cameraLayer=${layer}><${ProfileView} syncStatus=${syncStatus}
            syncProviders=${syncProviders} onSyncStatusChange=${onSyncStatusChange} refreshSyncStatus=${refreshSyncStatus} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('logs')} cameraLayer=${layer}><${LogsView} active=${true} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('dev')} cameraLayer=${layer}><${DevView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys-editor')} cameraLayer=${layer}><${HotkeyEditorSubPage} /><//>
    `;
}
```

- [ ] **Step 6: Verify syntax**

Run: `node --check ui/components/app/views.js && node --check ui/views/hotkeys-view.js && node --check ui/views/hotkeys/modal.js`
Expected: no output.

- [ ] **Step 7: Commit**

```bash
git add ui/components/app/views.js ui/views/hotkeys-view.js ui/views/hotkeys/modal.js
git commit -m "feat: sub-page slots, visibility culling, shared hotkey editor state"
```

---

### Task 8: Dive/ascend keyboard routing

**Files:**
- Modify: `ui/components/app/useAppKeyboardRouting.js`

- [ ] **Step 1: Wire dive and ascend into keyboard routing**

In `ui/components/app/useAppKeyboardRouting.js`:

Add `dive` and `ascend` to the destructured params of `useAppKeyboardRouting`:

```js
export function useAppKeyboardRouting({
    activePluginId,
    activeViewId,
    camera,
    closePluginConfig,
    switchView,
    viewOrder,
    palette,
    dive,
    ascend,
}) {
```

Store them in module-level refs so `globalSurfaceNav` can access them (same pattern as `_cameraRef`):

```js
let _diveRef = { current: null };
let _ascendRef = { current: null };
```

Inside `useAppKeyboardRouting`, set them:

```js
    _diveRef.current = dive;
    _ascendRef.current = ascend;
```

- [ ] **Step 2: Modify activateAndMaybeDescend to check data-dive-target**

Replace `activateAndMaybeDescend`:

```js
function activateAndMaybeDescend() {
    const current = findSelectedSurface();
    if (!current) return;

    if (current.getAttribute('role') === 'tab') {
        activateSurface(current);
        return;
    }

    activateSurface(current);

    const diveTarget = current.getAttribute('data-dive-target');
    if (diveTarget && _diveRef.current) {
        current.setAttribute('data-dive-source', '');
        requestAnimationFrame(() => _diveRef.current(diveTarget, current));
        return;
    }

    if (surfaceContainsChildContainer(current)) {
        requestAnimationFrame(() => descendIntoChild(current));
    }
}
```

- [ ] **Step 3: Modify ascendLayer to check camera layer**

Replace `ascendLayer`:

```js
function ascendLayer() {
    const camera = _cameraRef.current;
    if (camera && camera.layer < 0 && _ascendRef.current) {
        return _ascendRef.current();
    }

    const current = findSelectedSurface();
    const container = current ? activeContainer(current) : null;
    if (!container) return false;
    if (container.closest(MODAL_SELECTOR)) return false;

    const parent = parentContainer(container);
    if (!parent) return false;

    const parentSurfaces = directSurfaces(parent);
    const diveSource = parentSurfaces.find(el => el.hasAttribute('data-dive-source'));
    if (diveSource) diveSource.removeAttribute('data-dive-source');
    const anchor = diveSource
        || parentSurfaces.find(el => el.getAttribute('data-selected') === 'true')
        || parentSurfaces.find(el => el.contains(container))
        || parentSurfaces[0];
    if (!anchor) return false;

    anchor.focus({ preventScroll: true });
    return true;
}
```

- [ ] **Step 4: Verify syntax**

Run: `node --check ui/components/app/useAppKeyboardRouting.js`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add ui/components/app/useAppKeyboardRouting.js
git commit -m "feat: Enter triggers dive on data-dive-target, Escape ascends layers"
```

---

### Task 9: Hotkey dive-target on rows

**Files:**
- Modify: `ui/views/hotkeys/list.js`

- [ ] **Step 1: Add data-dive-target and update onActivate**

In `ui/views/hotkeys/list.js`, update the HotkeyRow rendering to add `data-dive-target` and always call `onEdit` on activate:

Replace the map body inside `HotkeysList`:

```js
        ${hotkeys.map((hk, i) => {
            const plugin = plugins.find(p => p.id === hk.plugin_id);
            return html`
                <${HotkeyRow} key=${hk.id}
                    shortcut=${hk.key}
                    pluginName=${plugin?.name || hk.plugin_id}
                    actionLabel=${getActionLabel(plugin, hk.action)}
                    status=${plugin?.status || 'installed'}
                    index=${i} selected=${i === selectedIndex} onSelect=${onSelect}
                    data-dive-target="hotkeys-editor"
                    onActivate=${() => { if (i !== selectedIndex) onSelect(i); onEdit(hk); }} />
            `;
        })}
```

Key changes:
- Added `data-dive-target="hotkeys-editor"` — this passes through HotkeyRow → TableRow → Surface → DOM.
- Changed `onActivate` to always call `onEdit(hk)` (sets up the editor state before the dive).

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/views/hotkeys/list.js`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add ui/views/hotkeys/list.js
git commit -m "feat: hotkey rows declare data-dive-target for editor sub-page"
```

---

### Task 10: Spatial queries filter by layer

**Files:**
- Modify: `ui/lib/viewport-spatial.js`

- [ ] **Step 1: Filter slots by current camera layer**

In `ui/lib/viewport-spatial.js`, update `slotAtCenter` to skip slots on hidden layers.

Replace `slotAtCenter`:

```js
export function slotAtCenter(viewport) {
    const vr = viewport.getBoundingClientRect();
    const cx = vr.left + vr.width / 2;
    const cy = vr.top + vr.height / 2;

    const el = document.elementFromPoint(cx, cy);
    let slot = el?.closest(SLOT_SELECTOR);
    let method = 'elementFromPoint';

    // elementFromPoint won't hit display:none slots, but verify layer is visible
    if (slot && slot.style.display === 'none') slot = null;

    if (!slot) {
        method = 'overlap';
        let bestOverlap = 0;
        for (const s of viewport.querySelectorAll(SLOT_SELECTOR)) {
            if (s.style.display === 'none') continue;
            const sr = s.getBoundingClientRect();
            const ox = Math.max(0, Math.min(sr.right, vr.right) - Math.max(sr.left, vr.left));
            const oy = Math.max(0, Math.min(sr.bottom, vr.bottom) - Math.max(sr.top, vr.top));
            const overlap = ox * oy;
            if (overlap > bestOverlap) { bestOverlap = overlap; slot = s; }
        }
    }

    log('slotAtCenter:', slot?.dataset?.viewId || 'NONE', 'via', method);
    return { slot, viewId: slot?.dataset?.viewId || null, center: { x: cx, y: cy } };
}
```

The key change: hidden slots (display:none from visibility culling) are skipped in the overlap scan. `elementFromPoint` naturally skips them since they have no layout.

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/lib/viewport-spatial.js`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add ui/lib/viewport-spatial.js
git commit -m "feat: spatial queries skip hidden (off-layer) slots"
```

---

### Task 11: Selection wedge zoom awareness

**Files:**
- Modify: `ui/components/SelectionCursorOverlay.js`

- [ ] **Step 1: Account for zoom in cursorStyle**

In `ui/components/SelectionCursorOverlay.js`, the `cursorStyle` function positions the wedge relative to the app container using `getBoundingClientRect()`. Since CSS transforms affect bounding rects, the coordinates already account for zoom — elements at zoom 0.12 have proportionally smaller rects.

However, the wedge overlay itself is in screen-space (not inside `#world`), so its positioning via `transform: translate(...)` is correct as-is. The wedge CSS size variables (`--selected-surface-wedge-size`, etc.) come from the target element's computed style, which reflects the zoomed size.

The only fix needed: at very low zoom (overview), the wedge should scale down so it doesn't appear disproportionately large relative to the tiny surfaces.

Update `cursorStyle` to apply a zoom scale factor. Change the return statement:

```js
    const camera = window.__worldCamera;
    const z = camera?.zoom || 1;
    const wedgeScale = z < 1 ? Math.max(0.3, z) : 1;

    return {
        width: `${targetRect.width}px`,
        height: `${targetRect.height}px`,
        opacity: 1,
        transform: `translate(${targetRect.left - appRect.left}px, ${targetRect.top - appRect.top}px) scale(${wedgeScale})`,
        transformOrigin: 'top left',
        transition: 'none',
        '--selection-wedge-z': readVar(vars, '--selected-surface-wedge-z', 'var(--z-selection-wedge)'),
        '--selection-wedge-size': readVar(vars, '--selected-surface-wedge-size', '22px'),
        '--selection-wedge-top': `calc(${restY} + ${gapY} + ${top})`,
        '--selection-wedge-left': `calc(${restX} + ${gapX} + ${left})`,
    };
```

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/components/SelectionCursorOverlay.js`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add ui/components/SelectionCursorOverlay.js
git commit -m "feat: wedge scales down at low zoom levels"
```

---

### Task 12: Minimap layer awareness

**Files:**
- Modify: `ui/components/app/Minimap.js`

- [ ] **Step 1: Add layer indicator and zoom-aware viewport rect**

In `ui/components/app/Minimap.js`, update the drawing to show a layer indicator and compute the viewport rect accounting for zoom.

Replace the full contents of `ui/components/app/Minimap.js`:

```js
import { html } from '../../lib/html.js';
import { useEffect, useRef, useState } from 'preact/hooks';
import { VIEW_LABELS } from './views.js';

export function Minimap({ camera, registry, viewportRef }) {
    const canvasRef = useRef(null);
    const [, bump] = useState(0);

    useEffect(() => {
        return camera.subscribe(() => bump(t => t + 1));
    }, [camera]);

    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const dpr = window.devicePixelRatio || 1;
        const cw = canvas.clientWidth;
        const ch = canvas.clientHeight;
        canvas.width = cw * dpr;
        canvas.height = ch * dpr;
        ctx.scale(dpr, dpr);

        const currentLayer = camera.layer;
        const bounds = registry.worldBounds(currentLayer);
        if (bounds.width === 0) return;
        const scale = Math.min(cw / bounds.width, ch / bounds.height);

        const vp = viewportRef?.current;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const z = camera.zoom || 1;
        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH, z);

        ctx.clearRect(0, 0, cw, ch);

        for (const e of registry.getEntriesForLayer(currentLayer)) {
            const rx = (e.x - bounds.x) * scale;
            const ry = (e.y - bounds.y) * scale;
            const rw = e.width * scale;
            const rh = e.height * scale;
            const active = e.id === activeId;

            ctx.fillStyle = active ? 'rgba(255,255,255,0.12)' : 'rgba(255,255,255,0.04)';
            ctx.fillRect(rx, ry, rw, rh);
            ctx.strokeStyle = active ? 'rgba(255,255,255,0.4)' : 'rgba(255,255,255,0.12)';
            ctx.lineWidth = active ? 1 : 0.5;
            ctx.strokeRect(rx, ry, rw, rh);

            const label = VIEW_LABELS[e.id] || e.id;
            ctx.fillStyle = active ? 'rgba(255,255,255,0.7)' : 'rgba(255,255,255,0.3)';
            ctx.font = `${active ? 'bold ' : ''}7px -apple-system, sans-serif`;
            ctx.textAlign = 'center';
            ctx.textBaseline = 'middle';
            ctx.fillText(label, rx + rw / 2, ry + rh / 2, rw - 2);
        }

        // Viewport rect accounts for zoom
        if (vp) {
            const worldVpW = vpW / z;
            const worldVpH = vpH / z;
            const vpX = (camera.x - bounds.x) * scale;
            const vpY = (camera.y - bounds.y) * scale;
            const vpWs = worldVpW * scale;
            const vpHs = worldVpH * scale;
            ctx.strokeStyle = 'rgba(255,255,255,0.6)';
            ctx.lineWidth = 1.5;
            ctx.strokeRect(vpX, vpY, vpWs, vpHs);
        }

        // Layer indicator
        const layerLabel = currentLayer === 0 ? 'L0' : `L${currentLayer}`;
        ctx.fillStyle = 'rgba(255,255,255,0.5)';
        ctx.font = 'bold 9px -apple-system, sans-serif';
        ctx.textAlign = 'right';
        ctx.textBaseline = 'bottom';
        ctx.fillText(layerLabel, cw - 4, ch - 3);
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const currentLayer = camera.layer;
        const bounds = registry.worldBounds(currentLayer);
        if (bounds.width === 0) return;
        const scale = Math.min(canvas.clientWidth / bounds.width, canvas.clientHeight / bounds.height);
        const vp = viewportRef?.current;
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const wx = bounds.x + (e.clientX - rect.left) / scale - vpW / (2 * z);
        const wy = bounds.y + (e.clientY - rect.top) / scale - vpH / (2 * z);
        camera.panSmooth(wx, wy, 300);
    };

    return html`
        <div class="world-minimap" onClick=${onClick}>
            <canvas ref=${canvasRef} style="width:100%;height:100%"></canvas>
        </div>
    `;
}
```

- [ ] **Step 2: Verify syntax**

Run: `node --check ui/components/app/Minimap.js`
Expected: no output.

- [ ] **Step 3: Commit**

```bash
git add ui/components/app/Minimap.js
git commit -m "feat: minimap shows current layer entries and layer indicator"
```

---

## Verification

After all tasks:

1. **Overview zoom:** App starts at zoom ~0.12. Pages appear as small cards spaced far apart. CTRL+arrow pans smoothly.
2. **Canvas background:** Dot grid draws in screen-space, scales with zoom. No dots visible at overview (too dense).
3. **Arrow navigation:** Surface navigation works at any zoom level. Wedge follows.
4. **Tab cycling:** Tab moves to next page view. Camera pans to it.
5. **Hotkey dive:** Navigate to a HotkeyRow, press Enter. Camera zooms to 1.0 and pans to the editor sub-page on layer -1. Editor form is populated.
6. **Hotkey ascend:** Press Escape from editor sub-page. Camera zooms back to overview. Focus returns to the hotkey row.
7. **Minimap:** Shows current layer's entries. Layer indicator reads "L0" or "L-1". Click-to-pan works.
8. **Visibility culling:** Layer -1 sub-page slots are `display:none` when camera is on layer 0, and vice versa.
9. **Wedge at low zoom:** Wedge scales down proportionally at overview zoom. CTRL preview works.
10. **`node --check`:** All modified JS files pass syntax check.
11. **`make build && make test`:** Full repo builds and tests pass.
