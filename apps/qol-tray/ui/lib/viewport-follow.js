export const KEYBOARD_FOLLOW_DURATION_MS = 180;
export const EDGE_FOLLOW_DURATION_MS = 200;

const CAMERA_FOLLOW_PAD = 40;
const KEYBOARD_VERTICAL_COMFORT_RATIO = 0.24;
const KEYBOARD_VERTICAL_COMFORT_MIN = 160;
const KEYBOARD_VERTICAL_COMFORT_MAX = 260;

export function keyboardFollowDelta(viewportRect, surfaceRect, pageRect) {
    const page = pageRect || surfaceRect;
    const centerDx = page.left + page.width / 2 - (viewportRect.left + viewportRect.width / 2);
    const rawDy = surfaceRect.top + surfaceRect.height / 2 - (viewportRect.top + viewportRect.height / 2);
    return {
        dx: settleDelta(centerDx + overflowDx(viewportRect, surfaceRect, centerDx)),
        dy: settleDelta(deadzoneDelta(rawDy, keyboardVerticalComfort(viewportRect))),
        mode: 'keyboard-page-center',
        duration: KEYBOARD_FOLLOW_DURATION_MS,
    };
}

function overflowDx(viewportRect, surfaceRect, centerDx) {
    const left = surfaceRect.left - centerDx;
    const right = left + surfaceRect.width;
    const viewLeft = viewportRect.left + CAMERA_FOLLOW_PAD;
    const viewRight = viewportRect.left + viewportRect.width - CAMERA_FOLLOW_PAD;
    if (surfaceRect.width >= viewRight - viewLeft) {
        return left + surfaceRect.width / 2 - (viewportRect.left + viewportRect.width / 2);
    }
    if (right > viewRight) return right - viewRight;
    if (left < viewLeft) return left - viewLeft;
    return 0;
}

export function edgeFollowDelta(viewportRect, surfaceRect) {
    let dx = 0, dy = 0;
    if (surfaceRect.bottom > viewportRect.bottom - CAMERA_FOLLOW_PAD) dy = surfaceRect.bottom - (viewportRect.bottom - CAMERA_FOLLOW_PAD);
    else if (surfaceRect.top < viewportRect.top + CAMERA_FOLLOW_PAD) dy = surfaceRect.top - (viewportRect.top + CAMERA_FOLLOW_PAD);
    if (surfaceRect.right > viewportRect.right - CAMERA_FOLLOW_PAD) dx = surfaceRect.right - (viewportRect.right - CAMERA_FOLLOW_PAD);
    else if (surfaceRect.left < viewportRect.left + CAMERA_FOLLOW_PAD) dx = surfaceRect.left - (viewportRect.left + CAMERA_FOLLOW_PAD);
    return {
        dx: settleDelta(dx),
        dy: settleDelta(dy),
        mode: 'edge',
        duration: EDGE_FOLLOW_DURATION_MS,
    };
}

export function normalizedZoom(zoom) {
    return zoom > 0 ? zoom : 1;
}

export function keyboardVerticalComfort(viewportRect) {
    return Math.max(
        KEYBOARD_VERTICAL_COMFORT_MIN,
        Math.min(KEYBOARD_VERTICAL_COMFORT_MAX, viewportRect.height * KEYBOARD_VERTICAL_COMFORT_RATIO),
    );
}

function settleDelta(value) {
    return Math.abs(value) > 0.5 ? value : 0;
}

function deadzoneDelta(value, threshold) {
    if (value > threshold) return value - threshold;
    if (value < -threshold) return value + threshold;
    return 0;
}
