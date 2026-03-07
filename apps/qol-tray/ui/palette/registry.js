const GLOBAL_ID = '__global__';
const registry = new Map();

export function registerCommands(viewId, commands) {
    registry.set(viewId, commands);
}

export function unregisterCommands(viewId) {
    registry.delete(viewId);
}

export function getCommands(activeViewId) {
    const contextual = registry.get(activeViewId) || [];
    const global = registry.get(GLOBAL_ID) || [];
    return [...contextual, ...global];
}

export { GLOBAL_ID };
