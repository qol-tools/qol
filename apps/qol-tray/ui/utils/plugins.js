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

const PLUGIN_EQ_KEYS = [
    'id', 'name', 'description', 'version', 'installed', 'installed_version',
    'running_version', 'available_version', 'update_available', 'source',
    'loaded', 'unavailable', 'has_config', 'has_cover', 'resolved_from',
    'load_error', 'active_failure_reason'
];

export function samePluginList(a, b) {
    if (a === b) return true;
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) {
        const x = a[i];
        const y = b[i];
        if (x === y) continue;
        if (!x || !y) return false;
        for (const key of PLUGIN_EQ_KEYS) {
            if (x[key] !== y[key]) return false;
        }
    }
    return true;
}
