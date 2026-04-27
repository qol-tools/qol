// Minimap geometry — piecewise linear projection.
//
// World coordinates are SPARSE (pages are 1280 wide but stride 10000 apart),
// so a single global linear projection collapses every slot to ~6px wide —
// labels skip, viewport rect degenerates. Instead we treat the minimap as a
// strip of slots proportional to entry.width, with inter-entry gaps collapsed
// to zero. The viewport rect is computed by walking the entries the camera
// window intersects and mapping each overlap into its slot.
//
// Slot-scale inflation: when the camera zooms below ghostThreshold and the
// `uiScaleOnZoomOut` setting is on (default), each page's `.world-view-slot`
// is CSS-scaled around its centre so it stays readable. The scaled slot's
// VISUAL world bounds extend beyond `entry.x..entry.x+entry.width`. The rect
// is therefore computed against an optional `inflatedRanges` array — when
// supplied, overlap is checked against the inflated bounds, so the rect
// reflects what the user *sees* rather than the raw camera window. When
// omitted the rect falls back to raw entry bounds (legacy behaviour, used
// in tests and zoom levels where slot-scale === 1 for every entry).
//
// Invariants the projection preserves:
//   - 1:1 alignment: when the camera window exactly covers one entry, the
//     viewport rect's x-range equals that entry's slot's x-range.
//   - zoom shrinks the rect: within a single entry, zoom * 2 halves rect
//     width (true linear piece).
//   - pan within an entry translates the rect linearly.
//   - slots are visible: min slot width >= minimapWidth / count, so labels
//     fit at reasonable minimap sizes.
//   - slot aspect matches page aspect: slot.w/slot.h equals the world entry's
//     width/height, so a landscape page renders as a landscape slot.
// Invariants it intentionally does NOT preserve:
//   - globally linear pan across world-gaps (gaps collapse, so panning over
//     a gap is instantaneous on the minimap). Users do not see the gaps, so
//     this matches expectations.

function projectEntries(sortedEntries, minimapWidth) {
    if (!Array.isArray(sortedEntries) || sortedEntries.length === 0) return null;
    if (!(minimapWidth > 0)) return null;
    let totalW = 0;
    for (const e of sortedEntries) totalW += e.width;
    if (!(totalW > 0)) return null;
    const scale = minimapWidth / totalW;
    const widths = new Array(sortedEntries.length);
    for (let i = 0; i < sortedEntries.length; i++) {
        widths[i] = sortedEntries[i].width * scale;
    }
    return { widths, totalW, scale };
}

// Compute the row layout: per-entry slot height from aspect ratio, the tallest
// slot height, and an optional uniform shrink factor when the tallest would
// otherwise exceed canvasHeight.
//
// Returns { scale, rowHeight } where scale <= 1 is the uniform shrink applied
// to slot width AND height to keep aspect intact. If canvasHeight is missing
// or non-positive, no shrink is applied and rowHeight is the unscaled max.
function rowLayout(sortedEntries, widths, canvasHeight) {
    let maxH = 0;
    for (let i = 0; i < sortedEntries.length; i++) {
        const e = sortedEntries[i];
        if (!(e.width > 0) || !(e.height > 0)) continue;
        const h = widths[i] * (e.height / e.width);
        if (h > maxH) maxH = h;
    }
    if (!(canvasHeight > 0) || !(maxH > 0) || maxH <= canvasHeight) {
        return { scale: 1, rowHeight: maxH };
    }
    const scale = canvasHeight / maxH;
    return { scale, rowHeight: canvasHeight };
}

function buildSlots(sortedEntries, widths, rowScale, minimapWidth, rowY) {
    const slots = new Array(sortedEntries.length);
    // When rowScale < 1 the total strip width drops below minimapWidth; centre
    // the strip horizontally so it stays anchored to the minimap centre.
    // unscaled sum of widths equals minimapWidth by construction of projectEntries.
    let totalShrunk = 0;
    for (let i = 0; i < widths.length; i++) totalShrunk += widths[i] * rowScale;
    let cursor = (minimapWidth - totalShrunk) / 2;
    for (let i = 0; i < sortedEntries.length; i++) {
        const e = sortedEntries[i];
        const w = widths[i] * rowScale;
        const h = e.width > 0 && e.height > 0 ? w * (e.height / e.width) : 0;
        slots[i] = { x: cursor, y: rowY, w, h };
        cursor += w;
    }
    return slots;
}

export function computeMinimapViewportRect({
    sortedEntries,
    cameraX,
    cameraZoom,
    viewportWidthPx,
    minimapWidth,
    canvasHeight,
    inflatedRanges,
}) {
    const empty = { x: 0, y: 0, width: 0, height: 0 };
    if (!(viewportWidthPx > 0)) return empty;
    const z = cameraZoom || 1;
    if (!(z > 0)) return empty;
    const proj = projectEntries(sortedEntries, minimapWidth);
    if (!proj) return empty;

    const layout = rowLayout(sortedEntries, proj.widths, canvasHeight);
    const rowHeight = layout.rowHeight;
    const rowY = canvasHeight > 0
        ? Math.max(0, (canvasHeight - rowHeight) / 2)
        : 0;
    const slots = buildSlots(sortedEntries, proj.widths, layout.scale, minimapWidth, rowY);

    const camEnd = cameraX + viewportWidthPx / z;
    const useInflated = Array.isArray(inflatedRanges) && inflatedRanges.length === sortedEntries.length;

    let minStart = Infinity;
    let maxEnd = -Infinity;
    for (let i = 0; i < sortedEntries.length; i++) {
        const e = sortedEntries[i];
        const slot = slots[i];
        // Visual world-x bounds: when slots are CSS-scaled around their centre
        // (ghost mode), `inflatedRanges[i]` extends past `entry.x..entry.x+width`.
        // The CAMERA-vs-VISUAL overlap test runs against the inflated range, but
        // the SLOT mapping still uses the visual range we overlapped against —
        // the slot strip itself isn't visually stretched, but the overlap
        // fraction within the slot tracks the inflated content.
        const visual = useInflated && inflatedRanges[i] ? inflatedRanges[i] : { x0: e.x, x1: e.x + e.width };
        const overlapStart = Math.max(cameraX, visual.x0);
        const overlapEnd = Math.min(camEnd, visual.x1);
        if (overlapEnd <= overlapStart) continue;
        const visW = visual.x1 - visual.x0;
        const fStart = visW > 0 ? Math.max(0, (overlapStart - visual.x0) / visW) : 0;
        const fEnd = visW > 0 ? Math.min(1, (overlapEnd - visual.x0) / visW) : 1;
        const xStart = slot.x + fStart * slot.w;
        const xEnd = slot.x + fEnd * slot.w;
        if (xStart < minStart) minStart = xStart;
        if (xEnd > maxEnd) maxEnd = xEnd;
    }
    if (minStart === Infinity) return { x: 0, y: rowY, width: 0, height: rowHeight };
    const x = Math.max(0, Math.min(minimapWidth, minStart));
    const end = Math.max(0, Math.min(minimapWidth, maxEnd));
    return { x, y: rowY, width: Math.max(0, end - x), height: rowHeight };
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

// Slot layout: one pixel-space rect per entry. Slots are sized proportional to
// entry.width, with all slots packed left-to-right. Slot height is derived
// from the per-entry aspect (entry.height / entry.width); when canvasHeight is
// supplied and the natural row height exceeds it, the row is uniformly shrunk
// (w AND h) so aspect is preserved. Slot y is the centred row offset.
// Returned array mirrors input order so callers can pair slots[i] with
// sortedEntries[i].
export function computeMinimapSlots({ sortedEntries, minimapWidth, canvasHeight }) {
    const proj = projectEntries(sortedEntries, minimapWidth);
    if (!proj) return [];
    const layout = rowLayout(sortedEntries, proj.widths, canvasHeight);
    const rowHeight = layout.rowHeight;
    const rowY = canvasHeight > 0
        ? Math.max(0, (canvasHeight - rowHeight) / 2)
        : 0;
    return buildSlots(sortedEntries, proj.widths, layout.scale, minimapWidth, rowY);
}
