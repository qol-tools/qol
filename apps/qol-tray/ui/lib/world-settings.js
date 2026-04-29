const KEY = 'qol-world-settings';

const DEFAULTS = {
    panSpeed: 12,
    transitionSpeed: 120,
    transitionStyle: 'zoom-fade',
    minimapSize: 380,
    // How many times wider than the viewport's world-x range the minimap
    // should cover. The minimap therefore tracks the viewport's zoom — when
    // the viewport zooms out, its world-x range grows, and so does the
    // minimap's. At MINIMAP_ZOOM_MAX (20) the minimap is clamped to the full
    // world span. 1 means minimap == viewport.
    minimapZoomFactor: 4,
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
        return raw ? { ...DEFAULTS, ...JSON.parse(raw) } : { ...DEFAULTS };
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
    current[key] = value;
    save();
}

export function subscribeWorldSettings(fn) {
    listeners.add(fn);
    return () => listeners.delete(fn);
}
