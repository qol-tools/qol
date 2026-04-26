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
