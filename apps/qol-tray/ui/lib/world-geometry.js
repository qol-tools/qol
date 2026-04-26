// Pixels from the top of the viewport where every navigated page anchors.
// Holding this constant across pages of different heights is what stops the
// vertical bounce when cycling through dive pages. Also reserves enough
// headroom for the world-region-label (positioned at entry.y - 56) to sit
// fully above each page without being clipped by the viewport top.
export const PAGE_TOP_PAD_PX = 96;

export function cameraTargetFor(entry, viewportW, viewportH, zoom) {
    const z = zoom || 1;
    return {
        x: entry.x + entry.width / 2 - viewportW / (2 * z),
        y: entry.y - PAGE_TOP_PAD_PX / z,
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

// Screen-space projection of a world-region-label for an entry, given the
// current camera. The label bottom is anchored LABEL_GAP_PX above the page
// top in screen pixels (CSS `transform: translateY(-100%)` applied via class).
// When the page's projected width falls below HIDE_BELOW_SCREEN_W, the label
// is hidden — otherwise tiny labels pile up when zoomed far out.
export const LABEL_GAP_PX = 10;
export const HIDE_BELOW_SCREEN_W = 120;

export function regionLabelPosition(entry, cam) {
    const z = cam.zoom;
    const screenX = (entry.x - cam.x) * z;
    const screenY = (entry.y - cam.y) * z;
    const screenW = entry.width * z;
    if (screenW < HIDE_BELOW_SCREEN_W) return { hidden: true };
    return {
        hidden: false,
        left: screenX,
        top: screenY - LABEL_GAP_PX,
        maxWidth: screenW,
    };
}

// Single source of truth for camera bounds on any layer (root or dive).
// Pads the raw confinement rect by half the viewport in world units so the
// camera can always anchor a page top at PAGE_TOP_PAD_PX regardless of page
// or confinement size, and so bounds-driven minZoom doesn't force auto-
// zoom-in when the confinement is tighter than the viewport.
// Falls back to the largest entry extent when viewport/zoom are unknown,
// which is the best we can do before first measure.
export function paddedWorldBounds(rect, vp, zoom, entries) {
    if (!rect) return null;
    const { padX, padY } = viewportPadding(vp, zoom, entries || [rect]);
    return { ...withPadding(rect, padX, padY), layer: rect.layer };
}

// Pages on the world canvas are CSS-scaled up via `transform: scale(--slot-scale)`
// when the camera zooms below `ghostThreshold` and `uiScaleOnZoomOut` is on. The
// scale is computed per-entry: full at `baseScale` near the camera centre, falling
// off with distance. This is the same formula App.js's `applySlotScales` writes
// to the DOM. We extract it here as a pure function so the minimap viewport rect
// can use the *visually inflated* bounds when checking what the camera frames —
// otherwise the rect under-represents what the user actually sees.
//
// IMPORTANT: keep WORLD_PAGE_STRIDE / WORLD_SLOT_GAP_FACTOR / WORLD_GHOST_THRESHOLD
// in sync with App.js's PAGE_STRIDE/SLOT_GAP_FACTOR and world-settings DEFAULTS.
// Drift here means the minimap rect and the rendered slots disagree.
export const WORLD_PAGE_STRIDE = 10000;
export const WORLD_SLOT_GAP_FACTOR = 0.85;
export const WORLD_GHOST_THRESHOLD_DEFAULT = 0.55;
const SLOT_SCALE_CENTER_FLOOR = 0.75;
const SLOT_SCALE_FALLOFF_FACTOR = 2.5;
const SLOT_SCALE_ZOOM_FLOOR = 0.05;

export function computeBaseScale(zoom, threshold) {
    if (!(zoom > 0)) return 1;
    if (!(threshold > 0)) return 1;
    return Math.max(1, Math.pow(threshold / zoom, 0.85));
}

// Per-entry slot scale. Returns 1 when `baseScale === 1` so callers can short-
// circuit the inflation pass cheaply. Pure: no DOM, no camera mutation.
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

// Visual world-x bounds of an entry after slot-scale inflation. Slots are
// scaled around their centre, so the inflated range is `centre ± (width/2)*scale`.
export function inflatedEntryRange(entry, slotScale) {
    if (!entry || !(entry.width > 0)) return { x0: entry?.x ?? 0, x1: (entry?.x ?? 0) + (entry?.width ?? 0) };
    if (!(slotScale > 1)) return { x0: entry.x, x1: entry.x + entry.width };
    const cx = entry.x + entry.width / 2;
    const half = (entry.width * slotScale) / 2;
    return { x0: cx - half, x1: cx + half };
}
