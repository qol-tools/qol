import {
    fetchTokenStatus,
    saveTokenRequest,
    deleteTokenRequest,
    fetchPluginsRequest,
    installPluginRequest
} from './effects.js';
import { isRateLimitedWithoutToken, sortPluginsByName } from './reducer.js';

export async function loadStoreTokenState() {
    return fetchTokenStatus();
}

export async function loadStorePlugins({ forceRefresh = false, hasToken = false } = {}) {
    const data = await fetchPluginsRequest(forceRefresh);
    const plugins = sortPluginsByName(data.plugins || []);
    return {
        plugins,
        cacheAgeSecs: data.cache_age_secs ?? null,
        rateLimited: isRateLimitedWithoutToken(plugins, hasToken)
    };
}

export async function saveStoreToken(token) {
    await saveTokenRequest(token);
}

export async function deleteStoreToken() {
    await deleteTokenRequest();
}

export async function installStorePlugin(pluginId) {
    await installPluginRequest(pluginId);
}
