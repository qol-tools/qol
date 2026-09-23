export const PAGE_TOP_PAD_PX = 96;

export function cameraTargetFor(entry, viewportW, viewportH, zoom) {
    const z = zoom || 1;
    return {
        x: entry.x + entry.width / 2 - viewportW / (2 * z),
        y: entry.y - PAGE_TOP_PAD_PX / z,
    };
}

export const CAMERA_FOLLOW_PAD_PX = 40;
const VERTICAL_COMFORT_RATIO = 0.24;
const VERTICAL_COMFORT_MIN_PX = 160;
const VERTICAL_COMFORT_MAX_PX = 260;

export function verticalComfortPx(viewportH) {
    return Math.max(
        VERTICAL_COMFORT_MIN_PX,
        Math.min(VERTICAL_COMFORT_MAX_PX, viewportH * VERTICAL_COMFORT_RATIO),
    );
}

export function cameraTargetForSurface(entry, surface, viewportW, viewportH, zoom) {
    const z = zoom || 1;
    const base = cameraTargetFor(entry, viewportW, viewportH, z);
    return {
        x: base.x + surfaceOverflowX(base.x, surface, viewportW / z, CAMERA_FOLLOW_PAD_PX / z),
        y: base.y + surfaceOverflowY(base.y, surface, entry, viewportH / z, verticalComfortPx(viewportH) / z),
    };
}

function surfaceOverflowX(baseX, surface, viewW, pad) {
    if (surface.width >= viewW - pad * 2) {
        return surface.x + surface.width / 2 - (baseX + viewW / 2);
    }
    const right = surface.x + surface.width;
    if (right > baseX + viewW - pad) return right - (baseX + viewW - pad);
    if (surface.x < baseX + pad) return surface.x - (baseX + pad);
    return 0;
}

function surfaceOverflowY(baseY, surface, entry, viewH, comfort) {
    const down = surface.y + surface.height - (baseY + viewH - comfort);
    if (down <= 0) return 0;
    return Math.min(down, Math.max(surface.y - entry.y, 0));
}

export function screenRectToWorld(rect, viewportRect, camera) {
    const z = camera.zoom > 0 ? camera.zoom : 1;
    return {
        x: camera.x + (rect.left - viewportRect.left) / z,
        y: camera.y + (rect.top - viewportRect.top) / z,
        width: rect.width / z,
        height: rect.height / z,
    };
}

export function withPadding(rect, padX, padY) {
    return {
        x: rect.x - padX,
        y: rect.y - padY,
        width: rect.width + padX * 2,
        height: rect.height + padY * 2,
        layer: rect.layer,
    };
}

export function boundsOfEntries(entries) {
    if (!entries.length) return null;
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const e of entries) {
        if (e.x < x0) x0 = e.x;
        if (e.y < y0) y0 = e.y;
        if (e.x + e.width > x1) x1 = e.x + e.width;
        if (e.y + e.height > y1) y1 = e.y + e.height;
    }
    return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
}

export function maxEntryExtent(entries) {
    let maxW = 0;
    let maxH = 0;
    for (const e of entries) {
        if (e.width > maxW) maxW = e.width;
        if (e.height > maxH) maxH = e.height;
    }
    return { padX: maxW, padY: maxH };
}

export function viewportPadding(vp, zoom, entries) {
    const w = vp && vp.w;
    const h = vp && vp.h;
    if (w > 0 && h > 0 && zoom > 0) {
        return { padX: w / (2 * zoom), padY: h / (2 * zoom) };
    }
    return maxEntryExtent(entries);
}

export const HIDE_BELOW_SCREEN_W = 120;

export function paddedWorldBounds(rect, vp, zoom, entries) {
    if (!rect) return null;
    const { padX, padY } = viewportPadding(vp, zoom, entries || [rect]);
    return { ...withPadding(rect, padX, padY), layer: rect.layer };
}

const WORLD_PAGE_STRIDE = 10000;
const WORLD_SLOT_GAP_FACTOR = 0.85;
const SLOT_SCALE_CENTER_FLOOR = 0.75;
const SLOT_SCALE_FALLOFF_FACTOR = 2.5;
const SLOT_SCALE_ZOOM_FLOOR = 0.05;

export function computeBaseScale(zoom, threshold) {
    if (!(zoom > 0)) return 1;
    if (!(threshold > 0)) return 1;
    return Math.max(1, Math.pow(threshold / zoom, 0.85));
}

export function computeSlotScale({ entry, cameraX, cameraY, viewportW, viewportH, zoom, baseScale }) {
    if (!(baseScale > 1)) return 1;
    if (!entry || !(entry.width > 0)) return 1;
    const z = Math.max(zoom, SLOT_SCALE_ZOOM_FLOOR);
    const vpCenterX = cameraX + viewportW / (2 * z);
    const vpCenterY = cameraY + viewportH / (2 * z);
    const cx = entry.x + entry.width / 2;
    const cy = entry.y + (entry.height || 0) / 2;
    const falloff = (WORLD_PAGE_STRIDE * z) * SLOT_SCALE_FALLOFF_FACTOR;
    if (!(falloff > 0)) return 1;
    const d = Math.hypot(cx - vpCenterX, cy - vpCenterY) * z;
    const t = 1 - Math.min(d / falloff, 1);
    const proximity = SLOT_SCALE_CENTER_FLOOR + (1 - SLOT_SCALE_CENTER_FLOOR) * (t * t * (3 - 2 * t));
    const maxSlotScale = (WORLD_PAGE_STRIDE * WORLD_SLOT_GAP_FACTOR) / Math.max(entry.width, 1);
    return Math.min(1 + (baseScale - 1) * proximity, maxSlotScale);
}
