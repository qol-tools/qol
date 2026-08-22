import { CAMERA_FOLLOW_PAD_PX, verticalComfortPx } from './world-geometry.js';

export const KEYBOARD_FOLLOW_DURATION_MS = 180;
const EDGE_FOLLOW_DURATION_MS = 200;

export function surfaceCenterDelta(viewportRect, surfaceRect) {
    const targetCenterX = surfaceRect.left + surfaceRect.width / 2;
    const targetCenterY = surfaceRect.top + surfaceRect.height / 2;
    const rawDy = targetCenterY - (viewportRect.top + viewportRect.height / 2);
    return {
        dx: settleDelta(targetCenterX - (viewportRect.left + viewportRect.width / 2)),
        dy: settleDelta(deadzoneDelta(rawDy, verticalComfortPx(viewportRect.height))),
        mode: 'surface-center',
        duration: KEYBOARD_FOLLOW_DURATION_MS,
    };
}

export function edgeFollowDelta(viewportRect, surfaceRect) {
    let dx = 0, dy = 0;
    if (surfaceRect.bottom > viewportRect.bottom - CAMERA_FOLLOW_PAD_PX) dy = surfaceRect.bottom - (viewportRect.bottom - CAMERA_FOLLOW_PAD_PX);
    else if (surfaceRect.top < viewportRect.top + CAMERA_FOLLOW_PAD_PX) dy = surfaceRect.top - (viewportRect.top + CAMERA_FOLLOW_PAD_PX);
    if (surfaceRect.right > viewportRect.right - CAMERA_FOLLOW_PAD_PX) dx = surfaceRect.right - (viewportRect.right - CAMERA_FOLLOW_PAD_PX);
    else if (surfaceRect.left < viewportRect.left + CAMERA_FOLLOW_PAD_PX) dx = surfaceRect.left - (viewportRect.left + CAMERA_FOLLOW_PAD_PX);
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

function settleDelta(value) {
    return Math.abs(value) > 0.5 ? value : 0;
}

function deadzoneDelta(value, threshold) {
    if (value > threshold) return value - threshold;
    if (value < -threshold) return value + threshold;
    return 0;
}
