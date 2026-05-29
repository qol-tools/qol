import {
    fetchTokenStatus,
    fetchPluginsRequest,
    fetchInstalledRequest,
    installPluginRequest,
    updatePluginRequest
} from './effects.js';
import { isRateLimitedWithoutToken, sortPluginsByName } from './reducer.js';

export async function loadStoreTokenState() {
    return fetchTokenStatus();
}

export async function loadStorePlugins({ forceRefresh = false, hasToken = false } = {}) {
    const [data, installed] = await Promise.all([fetchPluginsRequest(forceRefresh), loadInstalledIndex()]);
    const merged = (data.plugins || []).map(plugin => overlayInstalledState(plugin, installed[plugin.id]));
    const plugins = sortPluginsByName(merged);
    return {
        plugins,
        cacheAgeSecs: data.cache_age_secs ?? null,
        stale: Boolean(data.stale),
        revalidating: Boolean(data.revalidating),
        rateLimited: isRateLimitedWithoutToken(plugins, hasToken)
    };
}

async function loadInstalledIndex() {
    try {
        const payload = await fetchInstalledRequest();
        const index = {};
        for (const plugin of payload.plugins || []) index[plugin.id] = plugin;
        return index;
    } catch {
        return {};
    }
}

function overlayInstalledState(plugin, installed) {
    if (!installed) return plugin;
    return {
        ...plugin,
        installed: plugin.installed || !installed.unavailable,
        source: installed.source ?? null,
        update_available: Boolean(installed.update_available),
        running_version: installed.version ?? null,
        available_version: installed.available_version ?? null
    };
}

export async function installStorePlugin(pluginId) {
    await installPluginRequest(pluginId);
}

export async function updateStorePlugin(pluginId) {
    const result = await updatePluginRequest(pluginId);
    if (result && result.success === false) {
        throw new Error(result.message || 'Update failed');
    }
    return result;
}
