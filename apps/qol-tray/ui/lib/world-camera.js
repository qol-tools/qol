import { createDebug } from './debug.js';

const log = createDebug('qol:camera');

export function createCamera(options = {}) {
    const getViewportSize = options.getViewportSize || (() => ({ w: 800, h: 600 }));
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
        if (!bounds) return nz;
        if (typeof bounds.layer === 'number' && bounds.layer !== layer) return nz;
        const vp = getViewportSize();
        const minZoom = Math.max(vp.w / bounds.width, vp.h / bounds.height);
        return Math.max(nz, minZoom);
    }

    function setBounds(rect) {
        bounds = rect;
        if (bounds) {
            const clamped = clampPanTarget(x, y);
            x = clamped.x;
            y = clamped.y;
            apply();
        }
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

    function zoomSmooth(tx, ty, tz, duration, onComplete) {
        cancelSmooth();
        const clampedZ = clampZoom(tz);
        const clamped = clampPanTarget(tx, ty, clampedZ);
        log('zoomSmooth →', Math.round(clamped.x), Math.round(clamped.y), 'z:', clampedZ.toFixed(3), 'dur:', duration);
        animFrom = { x, y, zoom };
        animTarget = { x: clamped.x, y: clamped.y, zoom: clampedZ };
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
        zoomSmooth,
        cancelSmooth,
        nudge,
        setLayer,
        setBounds,
        subscribe(fn) { listeners.add(fn); return () => listeners.delete(fn); },
    };
}
