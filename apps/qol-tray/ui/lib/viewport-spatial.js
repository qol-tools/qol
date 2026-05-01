import { createDebug } from './debug.js';

const log = createDebug('qol:spatial');

const SURFACE_SELECTOR = '[data-selected-surface]';
const SLOT_SELECTOR = '.world-view-slot';

export function slotAtCenter(viewport) {
    const vr = viewport.getBoundingClientRect();
    const cx = vr.left + vr.width / 2;
    const cy = vr.top + vr.height / 2;

    const el = document.elementFromPoint(cx, cy);
    let slot = el?.closest(SLOT_SELECTOR);
    let method = 'elementFromPoint';

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

export function nearestSurfaceToCenter(viewport) {
    const vp = viewport || document.getElementById('viewport');
    if (!vp) return { surface: null, slot: null, viewId: null, dist: Infinity, count: 0 };

    const vr = vp.getBoundingClientRect();
    const { slot, viewId, center } = slotAtCenter(vp);
    const root = slot || vp;

    let best = null;
    let bestDist = Infinity;
    let count = 0;
    for (const el of root.querySelectorAll(SURFACE_SELECTOR)) {
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) continue;
        if (r.bottom < vr.top || r.top > vr.bottom || r.right < vr.left || r.left > vr.right) continue;
        count++;
        const d = Math.hypot(r.left + r.width / 2 - center.x, r.top + r.height / 2 - center.y);
        if (d < bestDist) { best = el; bestDist = d; }
    }

    return { surface: best, slot, viewId, dist: bestDist, count };
}

export function isInViewport(el, viewport) {
    if (!el || !viewport) return false;
    const vr = viewport.getBoundingClientRect();
    const er = el.getBoundingClientRect();
    return er.width > 0 && er.height > 0 &&
        er.bottom > vr.top && er.top < vr.bottom &&
        er.right > vr.left && er.left < vr.right;
}
