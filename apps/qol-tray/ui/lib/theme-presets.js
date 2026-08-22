import { QOL_THEMES, QOL_DEFAULT_THEME } from './generated-theme-tokens.js';

const boot = (typeof window !== 'undefined' && window.__QOL_BOOT__) || null;

export const THEMES = (boot?.theme?.themes?.length ? boot.theme.themes : QOL_THEMES)
    .map((entry) => ({
        key: entry.key,
        label: entry.label,
        accentKey: entry.accentKey ?? null,
        identityKey: entry.identityKey ?? null,
    }));

export const DEFAULT_THEME = boot?.theme?.defaultKey ?? QOL_DEFAULT_THEME;
export const SELECTED_THEME = boot?.theme?.selectedKey ?? null;

export function resolveTheme(key) {
    if (key && THEMES.some((theme) => theme.key === key)) return key;
    return DEFAULT_THEME;
}

export function applyTheme(key) {
    const resolved = resolveTheme(key);
    document.documentElement.setAttribute('data-qol-theme', resolved);
    const identity = themeIdentityKey(resolved);
    if (identity) document.documentElement.setAttribute('data-qol-identity', identity);
}

export function themeAccentKey(key) {
    return THEMES.find((theme) => theme.key === resolveTheme(key))?.accentKey ?? null;
}

function themeIdentityKey(key) {
    return THEMES.find((theme) => theme.key === resolveTheme(key))?.identityKey ?? null;
}
