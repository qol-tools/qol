export function computeMinimapLinearLayout({ entries, worldStart, worldEnd, minimapWidth, canvasHeight }) {
    if (!Array.isArray(entries) || entries.length === 0) return null;
    if (!(minimapWidth > 0)) return null;
    const range = worldEnd - worldStart;
    if (!(range > 0)) return null;
    const scale = minimapWidth / range;

    let maxNaturalH = 0;
    for (const e of entries) {
        if (!(e.width > 0) || !(e.height > 0)) continue;
        const slotH = (e.width * scale) * (e.height / e.width);
        if (slotH > maxNaturalH) maxNaturalH = slotH;
    }
    const rowScale = canvasHeight > 0 && maxNaturalH > canvasHeight
        ? canvasHeight / maxNaturalH
        : 1;
    const rowHeight = maxNaturalH * rowScale;
    const rowY = canvasHeight > 0
        ? Math.max(0, (canvasHeight - rowHeight) / 2)
        : 0;

    const slots = entries.map(e => {
        const valid = e.width > 0 && e.height > 0;
        const w = valid ? e.width * scale * rowScale : 0;
        const h = valid ? w * (e.height / e.width) : 0;
        return { x: (e.x - worldStart) * scale, y: rowY, w, h };
    });

    return { slots, scale, rowY, rowHeight, worldStart, worldEnd };
}

export function computeMinimapLinearRect({ cameraX, viewportRange, worldStart, worldEnd, minimapWidth, rowY = 0, rowHeight = 0 }) {
    const range = worldEnd - worldStart;
    if (!(range > 0) || !(minimapWidth > 0) || !(viewportRange > 0)) {
        return { x: 0, y: rowY, width: 0, height: rowHeight };
    }
    const scale = minimapWidth / range;
    return {
        x: (cameraX - worldStart) * scale,
        y: rowY,
        width: viewportRange * scale,
        height: rowHeight,
    };
}

export function computeSlotCoverage(slot, rect) {
    if (!slot || !(slot.w > 0)) return 0;
    if (!rect || !(rect.width > 0)) return 0;
    const slotEnd = slot.x + slot.w;
    const rectEnd = rect.x + rect.width;
    const overlapStart = Math.max(slot.x, rect.x);
    const overlapEnd = Math.min(slotEnd, rectEnd);
    const overlap = overlapEnd - overlapStart;
    if (!(overlap > 0)) return 0;
    const ratio = overlap / slot.w;
    if (ratio >= 1) return 1;
    if (ratio <= 0) return 0;
    return ratio;
}
