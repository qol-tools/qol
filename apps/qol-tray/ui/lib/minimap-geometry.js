// Minimap geometry — linear projection over a configurable world-x range.
//
// Each minimap pixel maps to a fixed slice of world-x within the projected
// range. Slot widths therefore depend only on entry.width and the
// projection scale (= minimapWidth / range) — they do NOT depend on how
// many entries fall inside the range. That means navigating between
// pages (which changes which entries are "in range") does not visually
// rescale the slot strip — page slots stay the same size whether the
// camera is at the world's edge (fewer neighbours) or in the middle
// (more neighbours).
//
// Inter-entry world gaps render as empty space at true scale, so the
// user sees the actual spatial layout. The viewport rect is derived
// from the same scale and tracks the camera's true world-x window.
//
// Trade-off vs the previous gap-collapsed packing: at very wide world
// spans (e.g. minimapZoom set to "all" over many strided pages) slot
// pixels shrink. That's the user's explicit choice — they picked "all"
// to see the entire world canvas.

// Linear layout of every entry into pixel space, given a world-x window
// to project. Slots are returned parallel-indexed with `entries`; the
// drawer culls slots whose pixel range falls outside [0, minimapWidth].
//
// `canvasHeight` is optional. When supplied and the largest slot's
// natural height (slotW * aspect) exceeds it, the row is uniformly
// shrunk in both dimensions so aspect is preserved. The result includes
// `rowY`/`rowHeight` so the caller can hand them to the rect helper.
export function computeMinimapLinearLayout({ entries, worldStart, worldEnd, minimapWidth, canvasHeight }) {
    if (!Array.isArray(entries) || entries.length === 0) return null;
    if (!(minimapWidth > 0)) return null;
    const range = worldEnd - worldStart;
    if (!(range > 0)) return null;
    const scale = minimapWidth / range;

    let maxNaturalH = 0;
    for (const e of entries) {
        if (!(e.width > 0) || !(e.height > 0)) continue;
        const slotW = e.width * scale;
        const slotH = slotW * (e.height / e.width);
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
        return {
            x: (e.x - worldStart) * scale,
            y: rowY,
            w,
            h,
        };
    });

    return { slots, scale, rowY, rowHeight, worldStart, worldEnd };
}

// Camera viewport rect in minimap pixel space. `viewportRange` is the
// camera's world-x window width (= viewportPixelWidth / cameraZoom). The
// rect can extend past [0, minimapWidth] when the camera is near the
// projected range's edge — the draw layer's clampRectForDraw clips it
// for rendering, but consumers (e.g. computeSlotCoverage) need the raw
// projection.
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

// Per-slot coverage: the fraction of the slot's x-range overlapped by the
// viewport rect in minimap pixel-space. Used to fade slots that sit outside
// the camera window, so the minimap strip itself conveys camera state rather
// than relying solely on the overlay rect.
//
// Return value is clamped to [0, 1]. Zero-width slots always return 0 (nothing
// to cover). Missing/invalid rect returns 0.
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
