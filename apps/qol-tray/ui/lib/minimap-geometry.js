export function computeMinimapViewportRect({
    sortedEntries,
    slots,
    cameraX,
    cameraZoom,
    viewportWidthPx,
}) {
    if (!Array.isArray(sortedEntries) || !Array.isArray(slots)) return { x: 0, width: 0 };
    if (sortedEntries.length === 0 || slots.length === 0) return { x: 0, width: 0 };
    const visibleWorldWidth = viewportWidthPx / (cameraZoom || 1);
    const vpStart = cameraX;
    const vpEnd = cameraX + visibleWorldWidth;
    let first = -1;
    let last = -1;
    for (let i = 0; i < sortedEntries.length; i++) {
        const e = sortedEntries[i];
        if (e.x < vpEnd && e.x + e.width > vpStart) {
            if (first === -1) first = i;
            last = i;
        }
    }
    if (first === -1) return { x: 0, width: 0 };
    const a = slots[first];
    const b = slots[last];
    return { x: a.x, width: b.x + b.w - a.x };
}
