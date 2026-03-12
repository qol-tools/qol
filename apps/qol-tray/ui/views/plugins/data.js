import { apiJson } from '../../api/client.js';
import { parseInstalledPayload } from '../../utils/plugins.js';
import { sortByName } from '../../utils/collections.js';

export async function loadInstalledPlugins() {
    const payload = parseInstalledPayload(await apiJson('/api/installed'));
    return {
        revision: payload.revision,
        plugins: sortByName(payload.plugins)
    };
}

export async function updateInstalledPlugin(pluginId) {
    const result = await apiJson(`/api/update/${pluginId}`, { method: 'POST' });
    if (result.success) {
        return result;
    }

    throw new Error(result.message);
}

export async function uninstallInstalledPlugin(pluginId) {
    const result = await apiJson(`/api/uninstall/${pluginId}`, { method: 'POST' });
    if (result.success) {
        return result;
    }

    throw new Error(result.message);
}

export function buildGhostPlugins(plugins, installingItems) {
    const installedIds = new Set(plugins.map(plugin => plugin.id));
    return installingItems.filter(plugin => !installedIds.has(plugin.id));
}

export function findPluginById(plugins, pluginId) {
    return plugins.find(plugin => plugin.id === pluginId) || null;
}
