export const FOCAL_GAP_PX = 5;
export const FOCAL_SLOT_ASPECT = 0.62;

export function computeMinimapFocalLayout({ entries, activePosF, focusRadius = 1, entryBoosts, minimapWidth, canvasHeight, minSlotPx = 0 }) {
    if (!Array.isArray(entries) || entries.length === 0) return null;
    if (!(minimapWidth > 0)) return null;
    const N = entries.length;
    const posF = Math.max(0, Math.min(N - 1, Number(activePosF) || 0));
    const floor = Math.max(0, Number(minSlotPx) || 0);
    const R = Math.max(0.5, Number(focusRadius) || 1);
    const decay = focusDecayFor(R);
    const boost = (i) => {
        if (!Array.isArray(entryBoosts)) return 1;
        const b = Number(entryBoosts[i]);
        return Number.isFinite(b) && b > 0 ? b : 1;
    };

    const weights = entries.map((_, i) => slotWeight(Math.abs(i - posF), decay, R) * boost(i));
    const weightSum = weights.reduce((a, b) => a + b, 0);
    if (!(weightSum > 0)) return null;

    const referenceDenom = referenceWeightSum(N, decay, R, boost);
    const referenceGapCount = referenceVisibleSlotCount(N, decay, R, boost) - 1;
    const gapTotal = Math.max(0, referenceGapCount) * FOCAL_GAP_PX;
    const availableW = Math.max(0, minimapWidth - gapTotal);
    const denom = Math.max(weightSum, referenceDenom);

    let widths = weights.map(w => availableW * (w / denom));
    if (floor > 0) widths = redistributeWithFloor(widths, weights, availableW, floor);

    let maxH = 0;
    const heights = widths.map(w => w * FOCAL_SLOT_ASPECT);
    for (const h of heights) if (h > maxH) maxH = h;
    const rowScale = canvasHeight > 0 && maxH > canvasHeight ? canvasHeight / maxH : 1;
    const rowHeight = maxH * rowScale;
    const rowY = canvasHeight > 0 ? Math.max(0, (canvasHeight - rowHeight) / 2) : 0;

    const lo = Math.min(N - 1, Math.floor(posF));
    const frac = Math.max(0, Math.min(1, posF - lo));
    let activeCenterOffset = 0;
    for (let i = 0; i < lo; i++) {
        activeCenterOffset += widths[i] * rowScale;
        if (widths[i] > 1e-9) activeCenterOffset += FOCAL_GAP_PX;
    }
    activeCenterOffset += (widths[lo] * rowScale) / 2;
    if (frac > 0 && lo < N - 1) {
        const stepToNext = (widths[lo] * rowScale) / 2 + (widths[lo] > 1e-9 || widths[lo + 1] > 1e-9 ? FOCAL_GAP_PX : 0) + (widths[lo + 1] * rowScale) / 2;
        activeCenterOffset += frac * stepToNext;
    }

    const startX = minimapWidth / 2 - activeCenterOffset;

    let cursor = startX;
    const slots = entries.map((_e, i) => {
        const w = widths[i] * rowScale;
        const h = heights[i] * rowScale;
        const slot = {
            x: cursor,
            y: rowY + (rowHeight - h) / 2,
            w,
            h,
        };
        cursor += w;
        if (w > 1e-9) cursor += FOCAL_GAP_PX;
        return slot;
    });

    return { slots, rowY, rowHeight };
}

function focusDecayFor(R) {
    return Math.pow(0.3, 1 / R);
}

function slotWeight(distance, decay, R) {
    const fade = R + 1 - distance;
    if (fade <= 0) return 0;
    if (fade >= 1) return Math.pow(decay, distance);
    return Math.pow(decay, distance) * fade;
}

function referenceWeightSum(N, decay, R, boost) {
    let max = 0;
    for (let k = 0; k < N; k++) {
        let s = 0;
        for (let i = 0; i < N; i++) s += slotWeight(Math.abs(i - k), decay, R) * boost(i);
        if (s > max) max = s;
    }
    return max;
}

function referenceVisibleSlotCount(N, decay, R, boost) {
    let max = 0;
    for (let k = 0; k < N; k++) {
        let n = 0;
        for (let i = 0; i < N; i++) {
            if (slotWeight(Math.abs(i - k), decay, R) * boost(i) > 1e-9) n++;
        }
        if (n > max) max = n;
    }
    return max;
}

function redistributeWithFloor(widths, weights, availableW, floor) {
    const N = widths.length;
    const eligible = weights.map(w => w > 1e-9);
    const flooredAt = new Array(N).fill(false);
    while (true) {
        let changed = false;
        let unflooredWeightSum = 0;
        let flooredW = 0;
        for (let i = 0; i < N; i++) {
            if (!eligible[i]) continue;
            if (flooredAt[i]) flooredW += floor;
            else unflooredWeightSum += weights[i];
        }
        const remaining = Math.max(0, availableW - flooredW);
        for (let i = 0; i < N; i++) {
            if (!eligible[i]) { widths[i] = 0; continue; }
            if (flooredAt[i]) { widths[i] = floor; continue; }
            const w = unflooredWeightSum > 0 ? remaining * (weights[i] / unflooredWeightSum) : 0;
            if (w < floor) { flooredAt[i] = true; changed = true; }
            else widths[i] = w;
        }
        if (!changed) break;
    }
    const overrun = widths.reduce((a, b) => a + b, 0) - availableW;
    if (overrun > 1e-6) {
        const scale = availableW / (availableW + overrun);
        for (let i = 0; i < N; i++) widths[i] = Math.max(0, widths[i] * scale);
    }
    return widths;
}

export function computeMinimapFocalRect({ entries, slots, cameraX, viewportRange }) {
    if (!Array.isArray(entries) || entries.length === 0) return null;
    if (!Array.isArray(slots) || slots.length !== entries.length) return null;
    if (!(viewportRange > 0)) return null;
    const camEnd = cameraX + viewportRange;
    let minPx = Infinity;
    let maxPx = -Infinity;
    for (let i = 0; i < entries.length; i++) {
        const e = entries[i];
        const slot = slots[i];
        if (!(e.width > 0)) continue;
        if (!(slot.w > 1e-9)) continue;
        const overlapStart = Math.max(cameraX, e.x);
        const overlapEnd = Math.min(camEnd, e.x + e.width);
        if (overlapEnd <= overlapStart) continue;
        const fStart = (overlapStart - e.x) / e.width;
        const fEnd = (overlapEnd - e.x) / e.width;
        const pxStart = slot.x + fStart * slot.w;
        const pxEnd = slot.x + fEnd * slot.w;
        if (pxStart < minPx) minPx = pxStart;
        if (pxEnd > maxPx) maxPx = pxEnd;
    }
    if (minPx === Infinity) return { x: 0, width: 0 };
    return { x: minPx, width: Math.max(0, maxPx - minPx) };
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
