import { apiJson, jsonRequest } from '../api/client.js';
import { applyTheme, DEFAULT_THEME, resolveTheme, SELECTED_THEME } from './theme-presets.js';

let selectedThemeKey = SELECTED_THEME;
let effectiveThemeKey = resolveTheme(selectedThemeKey ?? DEFAULT_THEME);
const listeners = new Set();

export function getTheme() {
    return selectedThemeKey;
}

export function applyThemeSelection() {
    applyTheme(effectiveThemeKey);
    return selectedThemeKey;
}

export function subscribeTheme(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

export async function setTheme(key) {
    const response = await apiJson('/api/theme', jsonRequest('PUT', { key }, { qolSuppressErrorToast: true }));
    return commitTheme(response.selectedKey ?? null, response.key);
}

function commitTheme(nextSelectedKey, nextEffectiveKey) {
    const resolvedSelectedKey = nextSelectedKey && resolveTheme(nextSelectedKey) === nextSelectedKey
        ? nextSelectedKey
        : null;
    const resolvedEffectiveKey = resolveTheme(nextEffectiveKey);
    const changed = selectedThemeKey !== resolvedSelectedKey || effectiveThemeKey !== resolvedEffectiveKey;
    selectedThemeKey = resolvedSelectedKey;
    effectiveThemeKey = resolvedEffectiveKey;
    applyTheme(effectiveThemeKey);
    if (changed) {
        for (const listener of listeners) listener(selectedThemeKey);
    }
    return selectedThemeKey;
}
