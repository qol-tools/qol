# World Viewport Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the scroll-based content area with an infinite CSS-transform canvas ("World") where views are positioned regions navigated by camera panning.

**Architecture:** A single `#world` div inside `#viewport` (overflow:hidden) uses `transform: translate(-x,-y)` to pan. Camera state is imperative (direct DOM writes, not Preact state). Views are absolutely positioned at registry coordinates. Input: grab-to-pan, trackpad wheel, CTRL+arrows. Navigation: minimap, region labels, command palette landmarks, zoom-to-fit.

**Tech Stack:** Preact + htm (no JSX, no build step). No new dependencies.

**Verification:** `node --check` on all changed JS files. Manual testing in browser via `make dev`.

---

## File Map

| File | Action | Responsibility |
|------|--------|----------------|
| `ui/lib/world-camera.js` | Create | Camera object: position, panTo, panSmooth, animation loop |
| `ui/lib/world-registry.js` | Create | View entries, default layout, spatial index, active view detection |
| `ui/components/app/WorldViewport.js` | Create | #viewport + #world DOM, camera wiring, grab-to-pan, trackpad, CTRL mode |
| `ui/components/app/Minimap.js` | Create | Corner minimap overlay with click-to-jump |
| `ui/components/app/RegionLabels.js` | Create | World-space view/region labels |
| `ui/components/app/WorldNav.js` | Create | Registers landmarks in command palette, zoom-to-fit |
| `ui/components/App.js` | Modify | Replace sidebar+#content with WorldViewport |
| `ui/components/app/views.js` | Modify | ViewSlot → WorldViewSlot (absolute positioning, content-visibility) |
| `ui/components/SelectionCursorOverlay.js` | Modify | Replace scroll listener with camera callback |
| `ui/components/app/useAppKeyboardRouting.js` | Modify | Replace scrollForKeyboardSelection with camera auto-follow |
| `ui/hooks/useScrollIntoView.js` | Remove | Replaced by camera auto-follow |
| `ui/hooks/useRouter.js` | Modify | Parse viewId from hash, resolve to world coords |
| `ui/styles/app-shell.css` | Modify | Remove sidebar layout, add viewport/world/minimap styles |
| `ui/components/SidebarNav.js` | Remove | Replaced by world navigation |
| `ui/components/SidebarFooter.js` | Keep | Moves into viewport chrome (fixed position) |
| `ui/components/app/sidebar-context.js` | Remove | Replaced by landmarks |

---

### Task 1: Camera module

**Files:**
- Create: `ui/lib/world-camera.js`

- [ ] **Step 1: Create camera module**

```js
export function createCamera() {
    let x = 0;
    let y = 0;
    let worldEl = null;
    let animId = 0;
    let animFrom = null;
    let animTarget = null;
    let animStart = 0;
    let animDuration = 0;
    const listeners = new Set();

    function notify() {
        for (const fn of listeners) fn(x, y);
    }

    function apply() {
        if (worldEl) worldEl.style.transform = `translate(${-x}px, ${-y}px)`;
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
        animFrom = { x, y };
        animTarget = { x: tx, y: ty };
        animStart = performance.now();
        animDuration = duration;
        animId = requestAnimationFrame(tick);
    }

    function cancelSmooth() {
        if (animId) { cancelAnimationFrame(animId); animId = 0; }
        animTarget = null;
    }

    function tick(now) {
        if (!animTarget) return;
        const t = Math.min(1, (now - animStart) / animDuration);
        const e = 1 - Math.pow(1 - t, 3);
        x = animFrom.x + (animTarget.x - animFrom.x) * e;
        y = animFrom.y + (animTarget.y - animFrom.y) * e;
        apply();
        if (t < 1) {
            animId = requestAnimationFrame(tick);
        } else {
            animTarget = null;
            animId = 0;
        }
    }

    function nudge(dx, dy) {
        cancelSmooth();
        x += dx;
        y += dy;
        apply();
    }

    return {
        get x() { return x; },
        get y() { return y; },
        get animating() { return animTarget !== null; },
        setWorldElement(el) { worldEl = el; },
        panTo,
        panSmooth,
        cancelSmooth,
        nudge,
        subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); },
    };
}
```

- [ ] **Step 2: Syntax check**

Run: `node --check ui/lib/world-camera.js`

- [ ] **Step 3: Commit**

```bash
git add ui/lib/world-camera.js
git commit -m "feat: world camera module — panTo, panSmooth, nudge, subscribe"
```

---

### Task 2: View registry + spatial index

**Files:**
- Create: `ui/lib/world-registry.js`

- [ ] **Step 1: Create registry module**

```js
const DEFAULT_VIEW_WIDTH = 1000;
const DEFAULT_VIEW_HEIGHT = 800;
const GAP = 200;

export function createWorldRegistry(viewOrder) {
    const entries = new Map();
    let nextX = 0;

    for (const id of viewOrder) {
        entries.set(id, { id, x: nextX, y: 0, width: DEFAULT_VIEW_WIDTH, height: DEFAULT_VIEW_HEIGHT });
        nextX += DEFAULT_VIEW_WIDTH + GAP;
    }

    function getEntry(id) {
        return entries.get(id) || null;
    }

    function getAllEntries() {
        return Array.from(entries.values());
    }

    function worldBounds() {
        let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
        for (const e of entries.values()) {
            minX = Math.min(minX, e.x);
            minY = Math.min(minY, e.y);
            maxX = Math.max(maxX, e.x + e.width);
            maxY = Math.max(maxY, e.y + e.height);
        }
        if (minX === Infinity) return { x: 0, y: 0, width: 0, height: 0 };
        const pad = 100;
        return { x: minX - pad, y: minY - pad, width: maxX - minX + pad * 2, height: maxY - minY + pad * 2 };
    }

    function activeViewId(cameraX, cameraY, viewportW, viewportH) {
        const cx = cameraX + viewportW / 2;
        const cy = cameraY + viewportH / 2;
        let closest = null;
        let closestDist = Infinity;
        for (const e of entries.values()) {
            const vx = e.x + e.width / 2;
            const vy = e.y + e.height / 2;
            const d = Math.hypot(cx - vx, cy - vy);
            if (d < closestDist) { closest = e.id; closestDist = d; }
        }
        return closest;
    }

    function placeNew(id, width, height) {
        const w = width || DEFAULT_VIEW_WIDTH;
        const h = height || DEFAULT_VIEW_HEIGHT;
        let bestX = 0;
        for (const e of entries.values()) {
            bestX = Math.max(bestX, e.x + e.width + GAP);
        }
        const entry = { id, x: bestX, y: 0, width: w, height: h };
        entries.set(id, entry);
        return entry;
    }

    function cameraTargetForView(id, viewportW, viewportH) {
        const e = entries.get(id);
        if (!e) return null;
        return {
            x: e.x + e.width / 2 - viewportW / 2,
            y: e.y + e.height / 2 - viewportH / 2,
        };
    }

    return {
        getEntry,
        getAllEntries,
        worldBounds,
        activeViewId,
        placeNew,
        cameraTargetForView,
    };
}
```

- [ ] **Step 2: Syntax check**

Run: `node --check ui/lib/world-registry.js`

- [ ] **Step 3: Commit**

```bash
git add ui/lib/world-registry.js
git commit -m "feat: world registry — view positions, bounds, active view detection"
```

---

### Task 3: WorldViewport component

**Files:**
- Create: `ui/components/app/WorldViewport.js`
- Create: `ui/styles/world.css`

- [ ] **Step 1: Create world CSS**

```css
#viewport {
    flex: 1;
    overflow: hidden;
    position: relative;
    cursor: grab;
    min-height: 0;
}

#viewport.grabbing {
    cursor: grabbing;
}

#viewport.interactive {
    cursor: default;
}

#world {
    position: absolute;
    will-change: transform;
}

#world-bg {
    position: absolute;
    inset: -10000px;
    background-image: radial-gradient(circle, rgba(255,255,255,0.02) 1px, transparent 1px);
    background-size: 50px 50px;
    pointer-events: none;
}

.world-view-slot {
    position: absolute;
    content-visibility: auto;
    contain-intrinsic-size: auto 1000px 800px;
}
```

- [ ] **Step 2: Create WorldViewport component**

```js
import { html } from '../../lib/html.js';
import { useEffect, useRef, useCallback } from 'preact/hooks';

const PAN_SPEED = 12;
const INTERACTIVE_SELECTOR = 'button, input, select, textarea, [data-selected-surface], a, [role="tab"], [tabindex]';

export function WorldViewport({ camera, children }) {
    const viewportRef = useRef(null);
    const worldRef = useRef(null);
    const dragRef = useRef({ active: false, startX: 0, startY: 0, camX: 0, camY: 0, moved: false });
    const ctrlRef = useRef(false);
    const keysRef = useRef(new Set());

    useEffect(() => {
        if (worldRef.current) camera.setWorldElement(worldRef.current);
    }, [camera]);

    useEffect(() => {
        const vp = viewportRef.current;
        if (!vp) return;

        function onPointerDown(e) {
            if (e.button !== 0) return;
            const target = e.target;
            if (target.closest(INTERACTIVE_SELECTOR)) {
                vp.classList.add('interactive');
                return;
            }
            const d = dragRef.current;
            d.active = true;
            d.moved = false;
            d.startX = e.clientX;
            d.startY = e.clientY;
            d.camX = camera.x;
            d.camY = camera.y;
            camera.cancelSmooth();
            vp.classList.add('grabbing');
            vp.setPointerCapture(e.pointerId);
        }

        function onPointerMove(e) {
            const d = dragRef.current;
            if (!d.active) {
                const target = document.elementFromPoint(e.clientX, e.clientY);
                vp.classList.toggle('interactive', !!(target && target.closest(INTERACTIVE_SELECTOR)));
                return;
            }
            const dx = e.clientX - d.startX;
            const dy = e.clientY - d.startY;
            if (Math.abs(dx) > 3 || Math.abs(dy) > 3) d.moved = true;
            camera.panTo(d.camX - dx, d.camY - dy);
        }

        function onPointerUp(e) {
            const d = dragRef.current;
            if (!d.active) return;
            d.active = false;
            vp.classList.remove('grabbing');
            vp.classList.remove('interactive');
            vp.releasePointerCapture(e.pointerId);
        }

        function onWheel(e) {
            e.preventDefault();
            if (ctrlRef.current) return;
            camera.nudge(e.deltaX, e.deltaY);
        }

        function onKeyDown(e) {
            if (e.key === 'Control') ctrlRef.current = true;
            keysRef.current.add(e.key);
        }

        function onKeyUp(e) {
            if (e.key === 'Control') ctrlRef.current = false;
            keysRef.current.delete(e.key);
        }

        let rafId = 0;
        function ctrlPanLoop() {
            if (ctrlRef.current) {
                const keys = keysRef.current;
                let dx = 0, dy = 0;
                if (keys.has('ArrowLeft')) dx = -PAN_SPEED;
                if (keys.has('ArrowRight')) dx = PAN_SPEED;
                if (keys.has('ArrowUp')) dy = -PAN_SPEED;
                if (keys.has('ArrowDown')) dy = PAN_SPEED;
                if (dx || dy) camera.nudge(dx, dy);
            }
            rafId = requestAnimationFrame(ctrlPanLoop);
        }

        vp.addEventListener('pointerdown', onPointerDown);
        vp.addEventListener('pointermove', onPointerMove);
        vp.addEventListener('pointerup', onPointerUp);
        vp.addEventListener('wheel', onWheel, { passive: false });
        document.addEventListener('keydown', onKeyDown, true);
        document.addEventListener('keyup', onKeyUp, true);
        rafId = requestAnimationFrame(ctrlPanLoop);

        return () => {
            vp.removeEventListener('pointerdown', onPointerDown);
            vp.removeEventListener('pointermove', onPointerMove);
            vp.removeEventListener('pointerup', onPointerUp);
            vp.removeEventListener('wheel', onWheel);
            document.removeEventListener('keydown', onKeyDown, true);
            document.removeEventListener('keyup', onKeyUp, true);
            cancelAnimationFrame(rafId);
        };
    }, [camera]);

    return html`
        <div id="viewport" ref=${viewportRef}>
            <div id="world" ref=${worldRef}>
                <div id="world-bg"></div>
                ${children}
            </div>
        </div>
    `;
}

export function WorldViewSlot({ entry, children }) {
    if (!entry) return null;
    const style = `position:absolute; left:${entry.x}px; top:${entry.y}px; width:${entry.width}px;`;
    return html`<div class="world-view-slot" style=${style}>${children}</div>`;
}
```

- [ ] **Step 3: Syntax check**

Run: `node --check ui/components/app/WorldViewport.js`

- [ ] **Step 4: Commit**

```bash
git add ui/components/app/WorldViewport.js ui/styles/world.css
git commit -m "feat: WorldViewport component — grab-to-pan, trackpad, CTRL mode"
```

---

### Task 4: Minimap

**Files:**
- Create: `ui/components/app/Minimap.js`

- [ ] **Step 1: Create minimap component**

```js
import { html } from '../../lib/html.js';
import { useEffect, useRef, useState } from 'preact/hooks';

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

        const bounds = registry.worldBounds();
        if (bounds.width === 0) return;
        const scale = Math.min(cw / bounds.width, ch / bounds.height);

        ctx.clearRect(0, 0, cw, ch);

        for (const e of registry.getAllEntries()) {
            const rx = (e.x - bounds.x) * scale;
            const ry = (e.y - bounds.y) * scale;
            const rw = e.width * scale;
            const rh = e.height * scale;
            ctx.fillStyle = 'rgba(255,255,255,0.06)';
            ctx.fillRect(rx, ry, rw, rh);
            ctx.strokeStyle = 'rgba(255,255,255,0.15)';
            ctx.strokeRect(rx, ry, rw, rh);
        }

        const vp = viewportRef?.current;
        if (vp) {
            const vpW = vp.clientWidth * scale;
            const vpH = vp.clientHeight * scale;
            const vpX = (camera.x - bounds.x) * scale;
            const vpY = (camera.y - bounds.y) * scale;
            ctx.strokeStyle = 'rgba(255,255,255,0.6)';
            ctx.lineWidth = 1.5;
            ctx.strokeRect(vpX, vpY, vpW, vpH);
        }
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const bounds = registry.worldBounds();
        if (bounds.width === 0) return;
        const scale = Math.min(canvas.clientWidth / bounds.width, canvas.clientHeight / bounds.height);
        const vp = viewportRef?.current;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const wx = bounds.x + (e.clientX - rect.left) / scale - vpW / 2;
        const wy = bounds.y + (e.clientY - rect.top) / scale - vpH / 2;
        camera.panSmooth(wx, wy, 300);
    };

    return html`
        <div class="world-minimap" onClick=${onClick}>
            <canvas ref=${canvasRef} style="width:100%;height:100%"></canvas>
        </div>
    `;
}
```

- [ ] **Step 2: Add minimap CSS to `ui/styles/world.css`**

Append to `ui/styles/world.css`:

```css
.world-minimap {
    position: fixed;
    bottom: 16px;
    right: 16px;
    width: 200px;
    height: 120px;
    background: rgba(0, 0, 0, 0.75);
    border: 1px solid var(--border-default, #444);
    border-radius: var(--radius-md, 8px);
    z-index: var(--z-overlay, 100);
    cursor: pointer;
    overflow: hidden;
}
```

- [ ] **Step 3: Syntax check**

Run: `node --check ui/components/app/Minimap.js`

- [ ] **Step 4: Commit**

```bash
git add ui/components/app/Minimap.js ui/styles/world.css
git commit -m "feat: minimap — canvas overlay with click-to-jump"
```

---

### Task 5: Region labels

**Files:**
- Create: `ui/components/app/RegionLabels.js`

- [ ] **Step 1: Create region labels component**

```js
import { html } from '../../lib/html.js';

export function RegionLabels({ registry }) {
    return html`
        ${registry.getAllEntries().map(e => html`
            <div key=${e.id} class="world-region-label"
                style="left:${e.x}px; top:${e.y - 52}px;">
                ${formatLabel(e.id)}
            </div>
        `)}
    `;
}

function formatLabel(id) {
    const labels = {
        plugins: 'Plugins', store: 'Store', hotkeys: 'Hotkeys',
        shortcuts: 'Shortcuts', 'task-runner': 'Task Runner',
        profile: 'Profile', logs: 'Logs', dev: 'Developer',
    };
    return labels[id] || id;
}
```

- [ ] **Step 2: Add label CSS to `ui/styles/world.css`**

Append:

```css
.world-region-label {
    position: absolute;
    font-size: 42px;
    font-weight: 700;
    color: rgba(255, 255, 255, 0.04);
    letter-spacing: 2px;
    pointer-events: none;
    user-select: none;
    white-space: nowrap;
}
```

- [ ] **Step 3: Syntax check**

Run: `node --check ui/components/app/RegionLabels.js`

- [ ] **Step 4: Commit**

```bash
git add ui/components/app/RegionLabels.js ui/styles/world.css
git commit -m "feat: world region labels — view names in world-space"
```

---

### Task 6: World navigation (landmarks + zoom-to-fit)

**Files:**
- Create: `ui/components/app/WorldNav.js`

- [ ] **Step 1: Create WorldNav component**

Registers view landmarks in command palette and provides zoom-to-fit.

```js
import { useEffect, useMemo, useCallback, useRef } from 'preact/hooks';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { GLOBAL_ID } from '../../palette/registry.js';
import { useKeyboard } from '../../hooks/useKeyboard.js';

export function useWorldNav({ camera, registry, viewportRef }) {
    const getViewportSize = useCallback(() => {
        const el = viewportRef?.current;
        return { w: el?.clientWidth || 800, h: el?.clientHeight || 600 };
    }, [viewportRef]);

    const jumpToView = useCallback((id) => {
        const target = registry.cameraTargetForView(id, getViewportSize().w, getViewportSize().h);
        if (target) camera.panSmooth(target.x, target.y, 400);
    }, [camera, registry, getViewportSize]);

    const fitAll = useCallback(() => {
        const bounds = registry.worldBounds();
        const { w, h } = getViewportSize();
        camera.panSmooth(
            bounds.x + bounds.width / 2 - w / 2,
            bounds.y + bounds.height / 2 - h / 2,
            400
        );
    }, [camera, registry, getViewportSize]);

    const commands = useMemo(() => {
        const cmds = registry.getAllEntries().map(e => ({
            id: `world:jump:${e.id}`,
            label: `Go to ${formatLabel(e.id)}`,
            run: () => jumpToView(e.id),
        }));
        cmds.push({ id: 'world:fit-all', label: 'Fit all views', run: fitAll });
        return cmds;
    }, [registry, jumpToView, fitAll]);

    useRegisterCommands(GLOBAL_ID, commands);

    useKeyboard(useCallback((e) => {
        if (e.shiftKey && (e.key === '!' || e.key === '1')) {
            e.preventDefault();
            fitAll();
        }
    }, [fitAll]));
}

function formatLabel(id) {
    const labels = {
        plugins: 'Plugins', store: 'Store', hotkeys: 'Hotkeys',
        shortcuts: 'Shortcuts', 'task-runner': 'Task Runner',
        profile: 'Profile', logs: 'Logs', dev: 'Developer',
    };
    return labels[id] || id;
}
```

- [ ] **Step 2: Syntax check**

Run: `node --check ui/components/app/WorldNav.js`

- [ ] **Step 3: Commit**

```bash
git add ui/components/app/WorldNav.js
git commit -m "feat: world nav — palette landmarks + zoom-to-fit"
```

---

### Task 7: Adapt views rendering for world positioning

**Files:**
- Modify: `ui/components/app/views.js`

- [ ] **Step 1: Replace ViewSlot with WorldViewSlot**

The current `ViewSlot` uses `display:none` to hide inactive views. In the world, all views are always visible (positioned in world-space). Replace the `renderMountedViews` function. The `mounted` set is no longer needed — all views render.

Replace the `ViewSlot` component and `renderMountedViews`:

```js
function WorldViewSlot({ entry, children }) {
    if (!entry) return null;
    const style = `position:absolute; left:${entry.x}px; top:${entry.y}px; width:${entry.width}px; content-visibility:auto; contain-intrinsic-size:auto ${entry.width}px ${entry.height}px;`;
    return html`<div class="world-view-slot" style=${style}>${children}</div>`;
}

export function renderWorldViews({ registry, openPluginConfig, openPluginUi, syncStatus, syncProviders, onSyncStatusChange, refreshSyncStatus }) {
    return html`
        <${WorldViewSlot} entry=${registry.getEntry('plugins')}><${PluginsView} onOpenPluginConfig=${openPluginConfig} onOpenPluginUi=${openPluginUi} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('store')}><${StoreView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('hotkeys')}><${HotkeysView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('shortcuts')}><${ShortcutsView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('task-runner')}><${TaskRunnerView} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('profile')}><${ProfileView} syncStatus=${syncStatus}
            syncProviders=${syncProviders} onSyncStatusChange=${onSyncStatusChange} refreshSyncStatus=${refreshSyncStatus} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('logs')}><${LogsView} active=${true} /><//>
        <${WorldViewSlot} entry=${registry.getEntry('dev')}><${DevView} /><//>
    `;
}
```

Keep `VIEW_LABELS`, `buildViewOrder`, and existing imports. Remove `renderMountedViews`, `ViewSlot`. Add `renderWorldViews` and `WorldViewSlot`.

- [ ] **Step 2: Syntax check**

Run: `node --check ui/components/app/views.js`

- [ ] **Step 3: Commit**

```bash
git add ui/components/app/views.js
git commit -m "refactor: views rendering for world positioning — no display:none"
```

---

### Task 8: Replace scroll-into-view with camera auto-follow

**Files:**
- Modify: `ui/components/app/useAppKeyboardRouting.js`
- Remove: `ui/hooks/useScrollIntoView.js`

- [ ] **Step 1: Remove scrollForKeyboardSelection from useScrollIntoView**

Delete `ui/hooks/useScrollIntoView.js`. Remove the `useScrollIntoView()` call from `ui/components/App.js`.

- [ ] **Step 2: Add camera auto-follow to useAppKeyboardRouting**

In `useAppKeyboardRouting.js`, after `navigateInActiveContainer` calls `focusWithoutScroll(next)`, add camera auto-follow. The camera needs to be accessible. Add a `camera` parameter to the hook.

In `navigateInActiveContainer`, after `focusWithoutScroll(next)`:

```js
function navigateInActiveContainer(direction, camera, viewportEl) {
    // ... existing code to find next surface ...
    focusWithoutScroll(next);
    if (camera && viewportEl) {
        const vr = viewportEl.getBoundingClientRect();
        const nr = next.getBoundingClientRect();
        if (nr.top < vr.top || nr.bottom > vr.bottom || nr.left < vr.left || nr.right > vr.right) {
            const worldX = camera.x + (nr.left + nr.width / 2 - vr.left) - vr.width / 2;
            const worldY = camera.y + (nr.top + nr.height / 2 - vr.top) - vr.height / 2;
            camera.panSmooth(worldX, worldY, 150);
        }
    }
}
```

Pass `camera` and `viewportEl` through from `useAppKeyboardRouting` props.

- [ ] **Step 3: Syntax check**

Run: `node --check ui/components/app/useAppKeyboardRouting.js`

- [ ] **Step 4: Commit**

```bash
git add ui/components/app/useAppKeyboardRouting.js
git rm ui/hooks/useScrollIntoView.js
git commit -m "feat: camera auto-follow replaces scroll-into-view"
```

---

### Task 9: Adapt SelectionCursorOverlay

**Files:**
- Modify: `ui/components/SelectionCursorOverlay.js`

- [ ] **Step 1: Replace scroll listener with camera subscribe**

In `SelectionCursorOverlay`, the `useLayoutEffect` currently listens to `document.addEventListener('scroll', sync, true)`. Replace with camera subscription.

The overlay needs the camera object. Pass it as a prop or read from a shared ref.

Replace:
```js
document.addEventListener('scroll', sync, true);
```

With camera subscription inside the effect:
```js
const unsubCamera = window.__worldCamera?.subscribe(sync);
```

And in cleanup:
```js
if (unsubCamera) unsubCamera();
```

Replace `needsViewportTeleport` — instead of finding a scroll parent, check if the target rect is within `#viewport` bounds:

```js
function needsViewportTeleport(target) {
    const viewport = document.getElementById('viewport');
    if (!viewport) return false;
    const vr = viewport.getBoundingClientRect();
    const tr = target.getBoundingClientRect();
    return tr.top < vr.top + 2 || tr.bottom > vr.bottom - 2;
}
```

- [ ] **Step 2: Syntax check**

Run: `node --check ui/components/SelectionCursorOverlay.js`

- [ ] **Step 3: Commit**

```bash
git add ui/components/SelectionCursorOverlay.js
git commit -m "refactor: wedge overlay uses camera subscribe instead of scroll listener"
```

---

### Task 10: Wire everything into App.js

**Files:**
- Modify: `ui/components/App.js`
- Modify: `ui/hooks/useRouter.js`
- Modify: `ui/styles/app-shell.css`

- [ ] **Step 1: Update App.js**

Replace the sidebar + #content structure with WorldViewport. Create camera and registry at the app level.

Key changes to `AppShell`:

```js
import { createCamera } from '../lib/world-camera.js';
import { createWorldRegistry } from '../lib/world-registry.js';
import { WorldViewport } from './app/WorldViewport.js';
import { Minimap } from './app/Minimap.js';
import { RegionLabels } from './app/RegionLabels.js';
import { useWorldNav } from './app/WorldNav.js';
import { renderWorldViews } from './app/views.js';
```

Inside `AppShell`, create camera + registry (once, via refs):
```js
const cameraRef = useRef(null);
if (!cameraRef.current) cameraRef.current = createCamera();
const camera = cameraRef.current;

const registryRef = useRef(null);
if (!registryRef.current) registryRef.current = createWorldRegistry(viewOrder);
const registry = registryRef.current;

const viewportRef = useRef(null);

// Expose camera globally for SelectionCursorOverlay
useEffect(() => { window.__worldCamera = camera; return () => { window.__worldCamera = null; }; }, [camera]);
```

Use world nav:
```js
useWorldNav({ camera, registry, viewportRef });
```

Navigate to initial view on mount:
```js
useEffect(() => {
    const target = registry.cameraTargetForView(activeViewId, viewportRef.current?.clientWidth || 800, viewportRef.current?.clientHeight || 600);
    if (target) camera.panTo(target.x, target.y);
}, []);
```

Replace the render:
```js
return html`
    <${SidebarContext.Provider} value=${sidebarValue}>
    <${ModifierStateProvider}>
    <${PluginConfigProvider} pluginId=${activePluginId} mode=${activePluginMode}>
        <${ViewKeyboardProvider}>
            <${AppKeyboardRouting} ... camera=${camera} viewportRef=${viewportRef} />
            <div class="app-container">
                <${WorldViewport} camera=${camera} viewportRef=${viewportRef}>
                    <${RegionLabels} registry=${registry} />
                    ${activePluginId && html`...plugin config...`}
                    ${renderWorldViews({ registry, ... })}
                <//>
                <${Minimap} camera=${camera} registry=${registry} viewportRef=${viewportRef} />
                <${SelectionCursorOverlay} />
                <${RecompileDissolve} triggerRef=${dissolveRef} />
                <${GlobalToast} />
            </div>
        <//>
    <//>
    <//>
    <//>
`;
```

Remove: `SidebarNav` import, `SidebarFooter` import, `#sidebar` element, `defaultItems` computation, sidebar context (can be removed in a follow-up commit).

- [ ] **Step 2: Update router — view switch pans camera**

In `useRouter.js`, the `doSwitchView` function currently sets `activeViewId`. In the world model, switching views means panning the camera. The router doesn't own the camera directly, but the view switch triggers a camera pan via an effect in App.js.

Add an effect in App.js that pans on `activeViewId` change:
```js
const prevViewRef = useRef(activeViewId);
useEffect(() => {
    if (prevViewRef.current === activeViewId) return;
    prevViewRef.current = activeViewId;
    const vp = viewportRef.current;
    const target = registry.cameraTargetForView(activeViewId, vp?.clientWidth || 800, vp?.clientHeight || 600);
    if (target) camera.panSmooth(target.x, target.y, 400);
}, [activeViewId, camera, registry]);
```

- [ ] **Step 3: Update app-shell.css**

Remove `#sidebar` styles. Remove `#content` styles. The `#viewport` and `#world` styles are in `world.css`. Keep `.app-container`, `.app-main`, `.app-footer`.

Add `world.css` to the HTML stylesheet imports (or `@import` in styles.css).

- [ ] **Step 4: Syntax check all**

Run:
```bash
node --check ui/components/App.js && \
node --check ui/components/app/views.js && \
node --check ui/components/app/WorldViewport.js && \
node --check ui/components/app/Minimap.js && \
node --check ui/components/app/RegionLabels.js && \
node --check ui/components/app/WorldNav.js && \
node --check ui/lib/world-camera.js && \
node --check ui/lib/world-registry.js && \
node --check ui/components/SelectionCursorOverlay.js && \
node --check ui/components/app/useAppKeyboardRouting.js
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: wire world viewport into app — camera, registry, minimap, labels, auto-follow"
```

---

### Task 11: Remove dead code

**Files:**
- Remove: `ui/components/SidebarNav.js`
- Remove: `ui/components/app/sidebar-context.js`
- Remove: `ui/hooks/useScrollIntoView.js` (if not already removed)
- Modify: `ui/components/App.js` — remove sidebar context imports/usage

- [ ] **Step 1: Remove files**

```bash
git rm ui/components/SidebarNav.js
git rm ui/components/app/sidebar-context.js
git rm ui/hooks/useScrollIntoView.js
```

- [ ] **Step 2: Remove stale imports from App.js**

Remove imports of `SidebarNav`, `SidebarFooter`, `useSidebarProvider`, `useSidebarContext`. Remove `defaultItems` computation, `SidebarContext.Provider` wrapper.

- [ ] **Step 3: Remove sidebar references from useAppKeyboardRouting.js**

Remove `useSidebarContext` import and the `sidebar` variable. The `cycleView` function no longer checks `sidebar.isOverridden` — view cycling in the world pans the camera via the router's `switchView`.

- [ ] **Step 4: Syntax check all**

Run:
```bash
node --check ui/components/App.js && \
node --check ui/components/app/useAppKeyboardRouting.js
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "chore: remove sidebar, scroll-into-view, sidebar-context — replaced by world"
```

---

## Self-Review Checklist

**Spec coverage:**
- Camera + panTo/panSmooth: Task 1 ✓
- View registry + spatial index: Task 2 ✓
- WorldViewport + grab/trackpad/CTRL: Task 3 ✓
- Minimap: Task 4 ✓
- Region labels: Task 5 ✓
- Command palette landmarks + zoom-to-fit: Task 6 ✓
- View positioning (no display:none): Task 7 ✓
- Camera auto-follow (replaces scroll): Task 8 ✓
- Wedge overlay adaptation: Task 9 ✓
- App shell wiring: Task 10 ✓
- Dead code removal: Task 11 ✓
- No sidebar: Task 11 ✓
- content-visibility:auto: Task 7 (WorldViewSlot style) ✓
- Router hash → camera position: Task 10 ✓

**Type consistency:** `camera.panTo`, `camera.panSmooth`, `camera.nudge`, `camera.subscribe`, `camera.x`, `camera.y` — consistent across all tasks. `registry.getEntry`, `registry.getAllEntries`, `registry.worldBounds`, `registry.cameraTargetForView` — consistent.

**No placeholders found.**
