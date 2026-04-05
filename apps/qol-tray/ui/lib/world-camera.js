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
