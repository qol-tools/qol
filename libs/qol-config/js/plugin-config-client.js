export async function fetchPluginConfig(pluginId) {
    const res = await fetch(`/api/plugins/${pluginId}/config`);
    if (!res.ok) return null;
    const text = await res.text();
    return text ? JSON.parse(text) : null;
}

export function savePluginConfig(pluginId, config) {
    return fetch(`/api/plugins/${pluginId}/config`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(config, null, 2),
    });
}
