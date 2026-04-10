import { apiJson } from '../../api/client.js';
import { fetchGitHubAuthStatus } from '../../features/github-auth/actions.js';

export async function fetchTokenStatus() {
    try {
        const data = await fetchGitHubAuthStatus();
        return Boolean(data.connected);
    } catch (e) {
        return false;
    }
}

export async function fetchPluginsRequest(forceRefresh = false) {
    const url = forceRefresh ? '/api/plugins?refresh=true' : '/api/plugins';
    return apiJson(url);
}

export async function installPluginRequest(id) {
    return apiJson(`/api/install/${id}`, { method: 'POST' });
}
