import { apiJson, apiText, jsonRequest } from '../../api/client.js';

export async function loadShortcuts() {
    return apiJson('/api/shortcuts');
}

export async function createShortcut(shortcut) {
    return apiJson('/api/shortcuts', jsonRequest('POST', shortcut));
}

export async function updateShortcut(shortcut) {
    return apiJson(`/api/shortcuts/${shortcut.id}`, jsonRequest('PUT', shortcut));
}

export async function deleteShortcut(id) {
    return apiJson(`/api/shortcuts/${id}`, { method: 'DELETE' });
}

export async function runShortcut(id) {
    return apiText(`/api/shortcuts/${id}/run`, { method: 'POST' });
}

export function emptyShortcut() {
    return {
        id: '',
        name: '',
        enabled: true,
        export_to_launcher: true,
        action: { type: 'open_url', url: '' }
    };
}
