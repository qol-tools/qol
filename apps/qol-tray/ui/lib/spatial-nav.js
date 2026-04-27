import { createDebug } from './debug.js';
import { contains } from './world-registry.js';

const log = createDebug('qol:spatial');

export function isSurfaceInsideConfinement(surface, confinement, registry) {
    if (!confinement) return true;
    const viewEl = surface.closest?.('[data-view-id]');
    if (!viewEl) return false;
    const pageId = viewEl.dataset?.viewId;
    if (!pageId) return false;
    const entry = registry?.getEntry?.(pageId);
    if (!entry) return false;
    return contains(confinement, entry);
}

export function filterSurfacesByConfinement(surfaces, confinement, registry) {
    if (!confinement) return surfaces;
    return surfaces.filter(s => isSurfaceInsideConfinement(s, confinement, registry));
}

export function nearestSurfaceInDirection(surfaces, current, direction) {
    const horizontal = direction === 'left' || direction === 'right';
    const coneResult = spatialSearch(surfaces, current, direction, true);
    if (horizontal) return coneResult;
    const openResult = spatialSearch(surfaces, current, direction, false);
    if (!coneResult) return openResult;
    if (!openResult) return coneResult;
    const cr = current.getBoundingClientRect();
    const cdy = Math.abs(coneResult.getBoundingClientRect().top - cr.top);
    const ody = Math.abs(openResult.getBoundingClientRect().top - cr.top);
    return ody < cdy ? openResult : coneResult;
}

function spatialSearch(surfaces, current, direction, useCone) {
    const rect = current.getBoundingClientRect();
    const cx = rect.left;
    const cy = rect.top;
    const horizontal = direction === 'left' || direction === 'right';
    let best = null;
    let bestDist = Infinity;
    for (const el of surfaces) {
        if (el === current) continue;
        const r = el.getBoundingClientRect();
        const dx = r.left - cx;
        const dy = r.top - cy;
        if (direction === 'up' && dy >= 0) continue;
        if (direction === 'down' && dy <= 0) continue;
        if (direction === 'left' && dx >= 0) continue;
        if (direction === 'right' && dx <= 0) continue;
        const primary = horizontal ? Math.abs(dx) : Math.abs(dy);
        const cross = horizontal ? Math.abs(dy) : Math.abs(dx);
        if (useCone && horizontal && (cross > primary / 4 || cross > 100)) continue;
        if (useCone && !horizontal && cross > primary * 3) continue;
        const dist = horizontal ? primary + cross * 5 : primary * 3 + cross;
        log.verbose('  candidate', surfaceLabel(el),
            'pos=(' + Math.round(r.left) + ',' + Math.round(r.top) + ')',
            'dx=' + Math.round(dx), 'dy=' + Math.round(dy),
            'pri=' + Math.round(primary), 'cross=' + Math.round(cross),
            'dist=' + Math.round(dist),
            dist < bestDist ? '<- best' : '');
        if (dist < bestDist) { best = el; bestDist = dist; }
    }
    return best;
}

export function surfaceLabel(el) {
    for (const node of el.childNodes) {
        if (node.nodeType === 3) { const t = node.textContent.trim(); if (t) return t.slice(0, 20); }
    }
    const first = el.querySelector('.btn, .plugin-name, .custom-select-value, span');
    if (first) { const t = first.textContent?.trim(); if (t) return t.slice(0, 20); }
    return el.className?.split(' ')[0] || el.tagName;
}
