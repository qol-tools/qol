import { computeSlotCoverage } from './minimap-geometry.js';

const SLOT_INSET = 1;
const RADIUS = 3;
const ACTIVE_SCALE_X = 1.15;
const ACTIVE_SCALE_Y = 1.12;
const VIEWPORT_MIN_WIDTH = 8;
const INACTIVE_OPACITY_FLOOR = 0.22;
const ACTIVE_OPACITY_FLOOR = 0.55;
// Keep in sync with --accent-rgb in theme-tokens.css; canvas can't read CSS vars.
const ACCENT_R = 74;
const ACCENT_G = 158;
const ACCENT_B = 255;

function slotAlpha(coverage, floor) {
    if (!(coverage >= 0)) return floor;
    if (coverage >= 1) return 1;
    return floor + (1 - floor) * coverage;
}

function roundRect(ctx, x, y, w, h, r) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.lineTo(x + w - r, y);
    ctx.quadraticCurveTo(x + w, y, x + w, y + r);
    ctx.lineTo(x + w, y + h - r);
    ctx.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
    ctx.lineTo(x + r, y + h);
    ctx.quadraticCurveTo(x, y + h, x, y + h - r);
    ctx.lineTo(x, y + r);
    ctx.quadraticCurveTo(x, y, x + r, y);
    ctx.closePath();
}

function drawInactiveSlot(ctx, cw, label, slot, alpha) {
    if (slot.x + slot.w < 0 || slot.x > cw) return;
    const innerX = slot.x + SLOT_INSET;
    const innerW = Math.max(0, slot.w - SLOT_INSET * 2);
    const innerH = Math.max(0, slot.h - SLOT_INSET * 2);
    if (innerW <= 0 || innerH <= 0) return;
    const innerY = slot.y + SLOT_INSET;

    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.fillStyle = 'rgba(255,255,255,0.05)';
    roundRect(ctx, innerX, innerY, innerW, innerH, RADIUS);
    ctx.fill();
    ctx.strokeStyle = 'rgba(255,255,255,0.15)';
    ctx.lineWidth = 0.5;
    ctx.stroke();

    if (innerW >= 18 && label) {
        ctx.fillStyle = 'rgba(255,255,255,0.45)';
        ctx.font = '8px -apple-system, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(label, innerX + innerW / 2, innerY + innerH / 2, innerW - 6);
    }
    ctx.restore();
}

function drawActiveSlot(ctx, cw, label, slot, alpha) {
    const layoutInnerX = slot.x + SLOT_INSET;
    const layoutInnerW = Math.max(0, slot.w - SLOT_INSET * 2);
    const layoutInnerH = Math.max(0, slot.h - SLOT_INSET * 2);
    if (layoutInnerW <= 0 || layoutInnerH <= 0) return;

    const centerX = layoutInnerX + layoutInnerW / 2;
    const centerY = slot.y + slot.h / 2;
    const drawW = layoutInnerW * ACTIVE_SCALE_X;
    const drawH = layoutInnerH * ACTIVE_SCALE_Y;
    const drawX = centerX - drawW / 2;
    const drawY = centerY - drawH / 2;

    if (drawX + drawW < 0 || drawX > cw) return;

    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.shadowColor = 'rgba(140, 200, 255, 0.55)';
    ctx.shadowBlur = 8;
    ctx.fillStyle = 'rgba(255,255,255,0.22)';
    roundRect(ctx, drawX, drawY, drawW, drawH, RADIUS);
    ctx.fill();
    ctx.restore();

    ctx.save();
    ctx.globalAlpha = alpha;
    ctx.strokeStyle = 'rgba(255,255,255,0.85)';
    ctx.lineWidth = 1.5;
    roundRect(ctx, drawX, drawY, drawW, drawH, RADIUS);
    ctx.stroke();

    if (drawW >= 18 && label) {
        ctx.fillStyle = 'rgba(255,255,255,0.98)';
        ctx.font = 'bold 11px -apple-system, sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(label, centerX, centerY, drawW - 6);
    }
    ctx.restore();
}

export function drawMinimap(ctx, cw, ch, sortedEntries, slots, activeId, labelFor, rect) {
    let activeIdx = -1;
    for (let i = 0; i < sortedEntries.length; i++) {
        if (sortedEntries[i].id === activeId) { activeIdx = i; break; }
    }

    for (let i = 0; i < sortedEntries.length; i++) {
        if (i === activeIdx) continue;
        const label = labelFor ? labelFor(sortedEntries[i]) : null;
        const coverage = computeSlotCoverage(slots[i], rect);
        drawInactiveSlot(ctx, cw, label, slots[i], slotAlpha(coverage, INACTIVE_OPACITY_FLOOR));
    }

    if (activeIdx >= 0) {
        const label = labelFor ? labelFor(sortedEntries[activeIdx]) : null;
        const coverage = computeSlotCoverage(slots[activeIdx], rect);
        drawActiveSlot(ctx, cw, label, slots[activeIdx], slotAlpha(coverage, ACTIVE_OPACITY_FLOOR));
    }
}

export function clampRectForDraw(rect, cw, minWidth = VIEWPORT_MIN_WIDTH) {
    if (!(cw > 0)) return { x: 0, width: 0 };
    if (!(rect.width > 0)) return { x: 0, width: 0 };
    const targetWidth = Math.min(cw, Math.max(rect.width, minWidth));
    const centre = rect.x + rect.width / 2;
    let x = centre - targetWidth / 2;
    if (x < 0) x = 0;
    if (x + targetWidth > cw) x = cw - targetWidth;
    return { x, width: targetWidth };
}

export function drawViewportRect(ctx, cw, ch, rect) {
    const clamped = clampRectForDraw(rect, cw);
    if (clamped.width <= 0) return;
    const y = rect.y != null ? rect.y : 0;
    const h = rect.height != null ? rect.height : ch;

    ctx.fillStyle = `rgba(${ACCENT_R}, ${ACCENT_G}, ${ACCENT_B}, 0.18)`;
    roundRect(ctx, clamped.x, y, clamped.width, h, RADIUS);
    ctx.fill();

    ctx.save();
    ctx.shadowColor = `rgba(${ACCENT_R}, ${ACCENT_G}, ${ACCENT_B}, 0.45)`;
    ctx.shadowBlur = 6;
    ctx.strokeStyle = `rgba(${ACCENT_R}, ${ACCENT_G}, ${ACCENT_B}, 0.95)`;
    ctx.lineWidth = 2;
    roundRect(ctx, clamped.x, y, clamped.width, h, RADIUS);
    ctx.stroke();
    ctx.restore();
}

export { RADIUS, VIEWPORT_MIN_WIDTH };
