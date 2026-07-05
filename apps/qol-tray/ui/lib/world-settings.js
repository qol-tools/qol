const KEY = 'qol-world-settings';

const DEFAULTS = {
    panSpeed: 12,
    transitionSpeed: 120,
    transitionStyle: 'zoom-fade',
    minimapSize: 380,
    minimapZoomFactor: 1,
    anchorToPages: true,
    defaultZoom: 0.8,
    resetZoomOnNav: true,
    ghostThreshold: 0.55,
    uiScaleOnZoomOut: true,
};

let current = load();

function load() {
    try {
        const raw = localStorage.getItem(KEY);
        return raw ? { ...DEFAULTS, ...withoutThemeOwnedSettings(JSON.parse(raw)) } : { ...DEFAULTS };
    } catch {
        return { ...DEFAULTS };
    }
}

function save() {
    localStorage.setItem(KEY, JSON.stringify(current));
    const snapshot = { ...current };
    for (const fn of listeners) fn(snapshot);
}

const listeners = new Set();

export function getWorldSettings() { return current; }

export function setWorldSetting(key, value) {
    if (key === 'accent') return;
    current[key] = value;
    save();
}

export function subscribeWorldSettings(fn) {
    listeners.add(fn);
    return () => listeners.delete(fn);
}

function withoutThemeOwnedSettings(settings) {
    if (!settings || typeof settings !== 'object') return {};
    const { accent: _accent, ...worldSettings } = settings;
    return worldSettings;
}
