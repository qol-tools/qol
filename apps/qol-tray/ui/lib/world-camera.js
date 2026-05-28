import { createDebug } from './debug.js';

const log = createDebug('qol:camera');

const MAX_ZOOM = 8;
const UNBOUNDED_MIN_ZOOM = 0.1;

export function createCamera(options = {}) {
    const getViewportSize = options.getViewportSize || (() => ({ w: 800, h: 600 }));
    let x = 0;
    let y = 0;
    let zoom = typeof options.zoom === 'number' ? options.zoom : 1.0;
    let layer = 0;
    let worldEl = null;
    let animId = 0;
    let animFrom = null;
    let animTarget = null;
    let animStart = 0;
    let animDuration = 0;
    let animComplete = null;
    let bounds = null;
    const listeners = new Set();

    function clampPanTarget(tx, ty, zoomOverride) {
        if (!bounds) return { x: tx, y: ty };
        if (typeof bounds.layer === 'number' && bounds.layer !== layer) return { x: tx, y: ty };
        const vp = getViewportSize();
        const z = zoomOverride ?? zoom;
        const visibleW = vp.w / z;
        const visibleH = vp.h / z;
        let nx, ny;
        if (visibleW >= bounds.width) {
            nx = bounds.x + bounds.width / 2 - visibleW / 2;
        } else {
            nx = Math.max(bounds.x, Math.min(tx, bounds.x + bounds.width - visibleW));
        }
        if (visibleH >= bounds.height) {
            ny = bounds.y + bounds.height / 2 - visibleH / 2;
        } else {
            ny = Math.max(bounds.y, Math.min(ty, bounds.y + bounds.height - visibleH));
        }
        return { x: nx, y: ny };
    }

    function clampZoom(nz) {
        const activeBounds = bounds && (typeof bounds.layer !== 'number' || bounds.layer === layer);
        const vp = getViewportSize();
        const minZoom = activeBounds
            ? Math.min(vp.w / bounds.width, vp.h / bounds.height) * 0.5
            : UNBOUNDED_MIN_ZOOM;
        return Math.max(minZoom, Math.min(nz, MAX_ZOOM));
    }

    function setBounds(rect) {
        bounds = rect;
        if (!bounds) return;
        const clamped = clampPanTarget(x, y);
        if (clamped.x === x && clamped.y === y) return;
        x = clamped.x;
        y = clamped.y;
        apply();
    }

    function notify() {
        for (const fn of listeners) fn({ x, y, zoom, layer });
    }

    function apply() {
        if (worldEl) worldEl.style.transform = `scale(${zoom}) translate(${-x}px, ${-y}px)`;
        notify();
    }

    function panTo(nx, ny) {
        cancelSmooth();
        const clamped = clampPanTarget(nx, ny);
        x = clamped.x;
        y = clamped.y;
        apply();
    }

    function panSmooth(tx, ty, duration, onComplete) {
        cancelSmooth();
        const clamped = clampPanTarget(tx, ty);
        animFrom = { x, y, zoom };
        animTarget = { x: clamped.x, y: clamped.y, zoom };
        animStart = performance.now();
        animDuration = duration;
        animComplete = onComplete || null;
        animId = requestAnimationFrame(tick);
    }

    function zoomTo(nz) {
        cancelSmooth();
        zoom = clampZoom(nz);
        const clamped = clampPanTarget(x, y);
        x = clamped.x;
        y = clamped.y;
        apply();
    }

    function zoomAround(anchorScreenX, anchorScreenY, nz) {
        cancelSmooth();
        const clampedZ = clampZoom(nz);
        if (clampedZ === zoom) return;
        const anchorWorldX = x + anchorScreenX / zoom;
        const anchorWorldY = y + anchorScreenY / zoom;
        zoom = clampedZ;
        const clamped = clampPanTarget(
            anchorWorldX - anchorScreenX / clampedZ,
            anchorWorldY - anchorScreenY / clampedZ,
        );
        x = clamped.x;
        y = clamped.y;
        apply();
    }

    function zoomSmooth(tx, ty, tz, duration, onComplete) {
        cancelSmooth();
        const clampedZ = clampZoom(tz);
        const vp = getViewportSize();
        const worldCenterX = tx + vp.w / (2 * clampedZ);
        const worldCenterY = ty + vp.h / (2 * clampedZ);
        const screenStartX = (worldCenterX - x) * zoom;
        const screenStartY = (worldCenterY - y) * zoom;
        log('zoomSmooth →', Math.round(tx), Math.round(ty), 'z:', clampedZ.toFixed(3), 'dur:', duration);
        animFrom = { zoom, screenX: screenStartX, screenY: screenStartY };
        animTarget = {
            worldCenterX,
            worldCenterY,
            zoom: clampedZ,
            screenEndX: vp.w / 2,
            screenEndY: vp.h / 2,
        };
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
        zoom = animFrom.zoom + (animTarget.zoom - animFrom.zoom) * e;
        if (animTarget.worldCenterX != null) {
            const sx = animFrom.screenX + (animTarget.screenEndX - animFrom.screenX) * e;
            const sy = animFrom.screenY + (animTarget.screenEndY - animFrom.screenY) * e;
            const clamped = clampPanTarget(
                animTarget.worldCenterX - sx / zoom,
                animTarget.worldCenterY - sy / zoom,
                zoom,
            );
            x = clamped.x;
            y = clamped.y;
        } else {
            x = animFrom.x + (animTarget.x - animFrom.x) * e;
            y = animFrom.y + (animTarget.y - animFrom.y) * e;
        }
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
        const clamped = clampPanTarget(x + dx, y + dy);
        x = clamped.x;
        y = clamped.y;
        apply();
    }

    function setLayer(n) {
        log('setLayer:', layer, '→', n, `listeners=${listeners.size}`);
        layer = n;
        notify();
    }

    return {
        get x() { return x; },
        get y() { return y; },
        get zoom() { return zoom; },
        get layer() { return layer; },
        get animating() { return animTarget !== null; },
        setWorldElement(el) { worldEl = el; apply(); },
        panTo,
        panSmooth,
        zoomTo,
        zoomAround,
        zoomSmooth,
        cancelSmooth,
        nudge,
        setLayer,
        setBounds,
        subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); },
    };
}
