const GLOBAL_ID = '__global__';
const ALWAYS_ID = '__always__';
const registry = new Map();
const listeners = new Set();
let version = 0;

function bump() {
    version += 1;
    for (const fn of listeners) fn(version);
}

export function registerCommands(viewId, scope, commands) {
    let bucket = registry.get(viewId);
    if (!bucket) { bucket = new Map(); registry.set(viewId, bucket); }
    bucket.set(scope, commands);
    bump();
}

export function unregisterCommands(viewId, scope) {
    const bucket = registry.get(viewId);
    if (!bucket) return;
    bucket.delete(scope);
    if (bucket.size === 0) registry.delete(viewId);
    bump();
}

function bucketCommands(viewId) {
    const bucket = registry.get(viewId);
    if (!bucket) return [];
    return [...bucket.values()].flat();
}

export function getContextualCommands(activeViewId) {
    const always = bucketCommands(ALWAYS_ID);
    const view = bucketCommands(activeViewId);
    if (view.length > 0) return [...view, ...always];
    return [...bucketCommands(GLOBAL_ID), ...always];
}

export function subscribeRegistry(fn) {
    listeners.add(fn);
    return () => listeners.delete(fn);
}

export function getRegistryVersion() {
    return version;
}

export { ALWAYS_ID, GLOBAL_ID };
