export const NEIGHBOR_HARD_CAP = 4;

export function computePeripheralSlots(anchorId, siblings, maxNeighbors) {
    if (!anchorId || !Array.isArray(siblings)) return [];
    const idx = siblings.indexOf(anchorId);
    if (idx < 0) return [];
    if (!Number.isFinite(maxNeighbors) || maxNeighbors <= 0) return [];
    const n = Math.min(maxNeighbors, NEIGHBOR_HARD_CAP);
    const slots = [];
    for (let d = 1; d <= n; d++) {
        slots.push({ id: siblings[idx - d] ?? null, side: 'prev', distance: d });
        slots.push({ id: siblings[idx + d] ?? null, side: 'next', distance: d });
    }
    return slots;
}

export function computeSiblingCoverage(sibling, camera, viewport) {
    const area = sibling.width * sibling.height;
    if (area <= 0) return 0;
    const visW = viewport.w / camera.zoom;
    const visH = viewport.h / camera.zoom;
    const ix = Math.max(sibling.x, camera.x);
    const iy = Math.max(sibling.y, camera.y);
    const ix2 = Math.min(sibling.x + sibling.width, camera.x + visW);
    const iy2 = Math.min(sibling.y + sibling.height, camera.y + visH);
    const iw = Math.max(0, ix2 - ix);
    const ih = Math.max(0, iy2 - iy);
    return (iw * ih) / area;
}

export function handleSlotClick(slot, navigation, resetZoom = 1) {
    if (!slot?.id) return;
    navigation?.gotoAnchor?.({ pageId: slot.id }, { resetZoom });
}

export function shouldHidePeripheralSide({ side, activeEntry, camera, viewport, hysteresisPx = 16 }) {
    const activeLeftPx = (activeEntry.x - camera.x) * camera.zoom;
    const activeRightPx = (activeEntry.x + activeEntry.width - camera.x) * camera.zoom;
    if (side === 'next') return activeRightPx >= viewport.w - hysteresisPx;
    if (side === 'prev') return activeLeftPx <= hysteresisPx;
    return false;
}

export function isGhostScale(apparentScale, threshold) {
    return apparentScale < threshold;
}

export function pageMode(zoom, threshold) {
    return isGhostScale(zoom, threshold) ? 'ghost' : 'interactive';
}

export function pickCenteredEntry(entries, camera, viewport) {
    if (!entries?.length) return null;
    if (entries.length === 1) return entries[0];
    const z = camera.zoom || 1;
    const cx = camera.x + viewport.w / (2 * z);
    const cy = camera.y + viewport.h / (2 * z);
    let best = null;
    let bestDist = Infinity;
    for (const e of entries) {
        const ex = e.x + e.width / 2;
        const ey = e.y + e.height / 2;
        const d = Math.hypot(cx - ex, cy - ey);
        if (d < bestDist) { best = e; bestDist = d; }
    }
    return best;
}
