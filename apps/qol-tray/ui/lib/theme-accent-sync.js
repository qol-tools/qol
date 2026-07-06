import { apiJson, jsonRequest } from '../api/client.js';
import { applyAccent, DEFAULT_ACCENT, resolveAccent, SELECTED_ACCENT } from './accent-presets.js';

let selectedAccentKey = SELECTED_ACCENT;
let effectiveAccentKey = resolveAccent(selectedAccentKey ?? DEFAULT_ACCENT);
const listeners = new Set();

export function getThemeAccent() {
    return selectedAccentKey;
}

export function applyThemeAccent() {
    applyAccent(effectiveAccentKey);
    return selectedAccentKey;
}

export function subscribeThemeAccent(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

export async function setThemeAccent(key) {
    const response = await apiJson('/api/theme/accent', jsonRequest('PUT', { key }, { qolSuppressErrorToast: true }));
    return commitThemeAccent(response.selectedKey ?? null, response.key);
}

function commitThemeAccent(nextSelectedKey, nextEffectiveKey) {
    const resolvedSelectedKey = nextSelectedKey && resolveAccent(nextSelectedKey) === nextSelectedKey
        ? nextSelectedKey
        : null;
    const resolvedEffectiveKey = resolveAccent(nextEffectiveKey);
    const changed = selectedAccentKey !== resolvedSelectedKey || effectiveAccentKey !== resolvedEffectiveKey;
    selectedAccentKey = resolvedSelectedKey;
    effectiveAccentKey = resolvedEffectiveKey;
    applyAccent(effectiveAccentKey);
    if (changed) {
        for (const listener of listeners) listener(selectedAccentKey);
    }
    return selectedAccentKey;
}
