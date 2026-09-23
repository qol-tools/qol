export const GHOST_OPACITY_MIN = 0;
export const GHOST_OPACITY_MAX = 1;
export const GHOST_OPACITY_STEP = 0.05;
export const GHOST_OPACITY_DEFAULT = 0;
export const GHOST_DEBUG_COLOR_DEFAULT = '';

const HEX6 = /^[0-9a-f]{6}$/;

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

export function normalizeGhostColor(value) {
    if (value === null || value === undefined) return GHOST_DEBUG_COLOR_DEFAULT;
    const raw = String(value).trim();
    if (raw === '') return GHOST_DEBUG_COLOR_DEFAULT;
    const body = raw.startsWith('#') ? raw.slice(1) : raw;
    const lower = body.toLowerCase();
    if (!HEX6.test(lower)) return GHOST_DEBUG_COLOR_DEFAULT;
    return '#' + lower;
}

export function isValidGhostColor(value) {
    if (typeof value !== 'string') return false;
    const raw = value.trim();
    if (raw === '') return false;
    const body = raw.startsWith('#') ? raw.slice(1) : raw;
    return HEX6.test(body.toLowerCase());
}

export function parseGpuiResponse(body) {
    if (!body || typeof body !== 'object') {
        return { ghostOpacity: GHOST_OPACITY_DEFAULT, ghostColor: GHOST_DEBUG_COLOR_DEFAULT };
    }
    const opacity = body.ghost_opacity;
    const color = body.ghost_debug_color;
    return {
        ghostOpacity: (opacity === null || opacity === undefined) ? GHOST_OPACITY_DEFAULT : clampOpacity(opacity),
        ghostColor: normalizeGhostColor(color),
    };
}
