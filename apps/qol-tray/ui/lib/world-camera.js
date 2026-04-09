import { createDebug } from './debug.js';

const log = createDebug('qol:camera');
const STORAGE_KEY = 'qoltray.camera';
const SAVE_DEBOUNCE_MS = 300;

function loadSaved() {
    try {
        const raw = localStorage.getItem(STORAGE_KEY);
        if (!raw) return null;
        const s = JSON.parse(raw);
        if (typeof s.x === 'number' && typeof s.y === 'number') return s;
    } catch {}
    return null;
}

function saveTo(x, y, zoom) {
    try { localStorage.setItem(STORAGE_KEY, JSON.stringify({ x, y, zoom })); } catch {}
}

export function createCamera() {
    const saved = loadSaved();
    let x = saved?.x ?? 0;
    let y = saved?.y ?? 0;
    let zoom = saved?.zoom ?? 1.0;
    let layer = 0;
    let worldEl = null;
    let animId = 0;
    let animFrom = null;
    let animTarget = null;
    let animStart = 0;
    let animDuration = 0;
    let animComplete = null;
    const listeners = new Set();
    let saveTimer = 0;
    let dirty = false;

    window.addEventListener('beforeunload', () => { if (dirty) saveTo(x, y, zoom); });

    function notify() {
        for (const fn of listeners) fn({ x, y, zoom, layer });
    }

    function apply() {
        if (worldEl) worldEl.style.transform = `scale(${zoom}) translate(${-x}px, ${-y}px)`;
        notify();
        dirty = true;
        clearTimeout(saveTimer);
        saveTimer = setTimeout(() => { saveTo(x, y, zoom); dirty = false; }, SAVE_DEBOUNCE_MS);
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
        setWorldElement(el) { worldEl = el; apply(); },
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
