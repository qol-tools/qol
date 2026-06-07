export const GHOST_OPACITY_MIN = 0;
export const GHOST_OPACITY_MAX = 1;
export const GHOST_OPACITY_STEP = 0.05;
export const GHOST_OPACITY_DEFAULT = 0;

export function clampOpacity(value) {
    const num = Number(value);
    if (!Number.isFinite(num)) return GHOST_OPACITY_DEFAULT;
    if (num < GHOST_OPACITY_MIN) return GHOST_OPACITY_MIN;
    if (num > GHOST_OPACITY_MAX) return GHOST_OPACITY_MAX;
    return num;
}

export function normalizeOpacityForServer(value) {
    if (value === null || value === undefined) return null;
    const num = Number(value);
    if (!Number.isFinite(num)) return null;
    return clampOpacity(num);
}

export function formatOpacityPercent(value) {
    const clamped = clampOpacity(value);
    return Math.round(clamped * 100) + '%';
}

export function parseGpuiResponse(body) {
    if (!body || typeof body !== 'object') return { ghostOpacity: GHOST_OPACITY_DEFAULT };
    const raw = body.ghost_opacity;
    if (raw === null || raw === undefined) return { ghostOpacity: GHOST_OPACITY_DEFAULT };
    return { ghostOpacity: clampOpacity(raw) };
}
