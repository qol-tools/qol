import { apiJson, jsonRequest } from '../api/client.js';

export async function setNativeTheme(key) {
    return apiJson('/api/native-theme', jsonRequest('PUT', { key }, { qolSuppressErrorToast: true }));
}

export async function getNativeTheme() {
    const response = await apiJson('/api/native-theme');
    return response.key;
}
