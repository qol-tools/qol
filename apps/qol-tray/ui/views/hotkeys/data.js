import { apiJson, apiResponse, jsonRequest } from '../../api/client.js';
import { parseInstalledPlugins } from '../../utils/plugins.js';
import { clampIndex } from '../../utils/collections.js';

const DEFAULT_ACTION = { id: 'run', label: 'Run' };

export async function loadHotkeysViewData() {
    const [hotkeysConfig, installedPayload] = await Promise.all([
        apiJson('/api/hotkeys'),
        apiJson('/api/installed')
    ]);

    return {
        hotkeys: hotkeysConfig.hotkeys || [],
        plugins: parseInstalledPlugins(installedPayload)
    };
}

export async function loadPlugins() {
    return parseInstalledPlugins(await apiJson('/api/installed'));
}

export async function loadRegistrationErrors() {
    return apiJson('/api/hotkeys/errors');
}

export async function persistHotkeys(hotkeys) {
    await apiResponse('/api/hotkeys', jsonRequest('PUT', { hotkeys }));
}

export function getAvailableActions(plugins, hotkeys, pluginId, editingId) {
    const plugin = plugins.find(p => p.id === pluginId);
    if (!plugin?.actions?.length) {
        return [DEFAULT_ACTION];
    }

    const assigned = hotkeys
        .filter(h => h.plugin_id === pluginId && h.id !== editingId)
        .map(h => h.action);

    return plugin.actions.filter(a => !assigned.includes(a.id));
}

export function buildSavedHotkeys(hotkeys, modal) {
    const entry = hotkeyEntry(modal);
    if (!modal.hotkey) {
        return [...hotkeys, entry];
    }

    return hotkeys.map(h => h.id === modal.hotkey.id ? entry : h);
}

export function removeHotkeyAtIndex(hotkeys, index) {
    return hotkeys.filter((_, i) => i !== index);
}

export function nextSelectedIndex(hotkeys, currentIndex) {
    return clampIndex(currentIndex, hotkeys.length);
}

function hotkeyEntry(modal) {
    return {
        id: modal.hotkey?.id || `hk-${Date.now()}`,
        key: modal.key,
        plugin_id: modal.pluginId,
        action: modal.action,
        enabled: modal.enabled !== false
    };
}
