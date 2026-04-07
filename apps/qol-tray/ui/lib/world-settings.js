const KEY = 'qol-world-settings';

const DEFAULTS = {
    panSpeed: 12,
    transitionSpeed: 120,
    transitionStyle: 'zoom-fade',
    minimapSize: 280,
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
