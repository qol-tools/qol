import {
    QOL_ACCENT_PRESETS,
    QOL_DEFAULT_ACCENT,
    QOL_DEFAULT_ACCENT_RGB,
} from './generated-theme-tokens.js';

const boot = (typeof window !== 'undefined' && window.__QOL_BOOT__) || null;

function buildPresets(palette) {
    const out = {};
    for (const entry of palette) {
        out[entry.key] = { label: entry.label, rgb: entry.rgb, hover: entry.hover };
    }
    return out;
}

export const ACCENT_PRESETS = buildPresets(boot?.accent?.palette || QOL_ACCENT_PRESETS);

export const DEFAULT_ACCENT = boot?.accent?.defaultKey ?? QOL_DEFAULT_ACCENT;
export const SELECTED_ACCENT = boot?.accent?.selectedKey ?? null;
export const DEFAULT_ACCENT_RGB = ACCENT_PRESETS[DEFAULT_ACCENT]?.rgb || QOL_DEFAULT_ACCENT_RGB;

export function resolveAccent(setting) {
    if (setting && ACCENT_PRESETS[setting]) return setting;
    return DEFAULT_ACCENT;
}

export function applyAccent(presetKey) {
    const preset = ACCENT_PRESETS[presetKey] || ACCENT_PRESETS[DEFAULT_ACCENT];
    if (!preset) return;
    const root = document.documentElement.style;
    root.setProperty('--accent-rgb', preset.rgb);
    root.setProperty('--accent-hover', preset.hover);
}
