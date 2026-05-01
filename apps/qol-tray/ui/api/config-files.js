import { apiResponse, readResponseText } from './client.js';

async function postOpen(endpoint, fallbackMessage) {
    const response = await apiResponse(endpoint, { method: 'POST' });
    if (response.ok) return;
    const message = (await readResponseText(response)) || fallbackMessage;
    throw new Error(message);
}

export function openHotkeysFile() {
    return postOpen('/api/hotkeys/open-file', 'Failed to open hotkeys file');
}

export function openShortcutsFile() {
    return postOpen('/api/shortcuts/open-file', 'Failed to open shortcuts file');
}
