import { apiJson, jsonRequest } from '../api/client.js';

const boot = (typeof window !== 'undefined' && window.__QOL_BOOT__) || null;

let useSystemNotifications = boot?.notifications?.useSystemNotifications ?? false;
const listeners = new Set();

export function getSystemNotifications() {
    return useSystemNotifications;
}

export function subscribeSystemNotifications(listener) {
    listeners.add(listener);
    return () => listeners.delete(listener);
}

export async function setSystemNotifications(enabled) {
    const response = await apiJson(
        '/api/notifications',
        jsonRequest('PUT', { useSystemNotifications: enabled }, { qolSuppressErrorToast: true }),
    );
    return commitNotifications(response.useSystemNotifications);
}

function commitNotifications(enabled) {
    const changed = useSystemNotifications !== enabled;
    useSystemNotifications = enabled;
    if (changed) {
        for (const listener of listeners) listener(useSystemNotifications);
    }
    return useSystemNotifications;
}
