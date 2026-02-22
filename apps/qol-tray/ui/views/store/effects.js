import { apiJson, apiResponse, jsonRequest, readResponseText } from '../../api/client.js';

export async function fetchTokenStatus() {
    try {
        const data = await apiJson('/api/github-token');
        return Boolean(data.has_token);
    } catch (e) {
        return false;
    }
}

export async function saveTokenRequest(token) {
    const response = await apiResponse('/api/github-token', jsonRequest('POST', { token }));
    if (response.ok) {
        return;
    }
    const message = (await readResponseText(response)) || 'Failed to save token';
    throw new Error(message);
}

export async function deleteTokenRequest() {
    const response = await apiResponse('/api/github-token', { method: 'DELETE' });
    if (response.ok) {
        return;
    }
    const message = (await readResponseText(response)) || 'Failed to delete token';
    throw new Error(message);
}

export async function fetchPluginsRequest(forceRefresh = false) {
    const url = forceRefresh ? '/api/plugins?refresh=true' : '/api/plugins';
    return apiJson(url);
}

export async function installPluginRequest(id) {
    const response = await apiResponse(`/api/install/${id}`, { method: 'POST' });
    if (response.ok) {
        return;
    }
    const message = (await readResponseText(response)) || 'Installation failed';
    throw new Error(message);
}
