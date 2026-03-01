import { apiJson, apiResponse, jsonRequest, readResponseText } from '../../api/client.js';

async function throwIfNotOk(response, fallback) {
    if (response.ok) return;
    const message = (await readResponseText(response)) || fallback;
    throw new Error(message);
}

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
    await throwIfNotOk(response, 'Failed to save token');
}

export async function deleteTokenRequest() {
    const response = await apiResponse('/api/github-token', { method: 'DELETE' });
    await throwIfNotOk(response, 'Failed to delete token');
}

export async function fetchPluginsRequest(forceRefresh = false) {
    const url = forceRefresh ? '/api/plugins?refresh=true' : '/api/plugins';
    return apiJson(url);
}

export async function installPluginRequest(id) {
    const response = await apiResponse(`/api/install/${id}`, { method: 'POST' });
    await throwIfNotOk(response, 'Installation failed');
}
