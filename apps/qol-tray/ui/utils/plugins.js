export function parseInstalledPayload(payload) {
    if (Array.isArray(payload)) {
        return { revision: 0, plugins: payload };
    }
    return {
        revision: Number.isInteger(payload?.revision) ? payload.revision : 0,
        plugins: Array.isArray(payload?.plugins) ? payload.plugins : []
    };
}

export function parseInstalledPlugins(payload) {
    return parseInstalledPayload(payload).plugins;
}
