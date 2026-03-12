import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { useGridNav } from '../../hooks/useGridNav.js';
import { updateInstalledPlugin, uninstallInstalledPlugin } from './data.js';

async function doUpdate(pluginId, updatingRef, clearFeedback, setUpdating, setFeedback, refreshPlugins) {
    if (updatingRef.current.has(pluginId)) return;
    clearFeedback();
    setUpdating(prev => new Set(prev).add(pluginId));
    try {
        await updateInstalledPlugin(pluginId);
        setFeedback('success', `Updated ${pluginId}`);
    } catch (error) {
        setFeedback('error', `Failed to update ${pluginId}: ${error.message}`);
    } finally {
        setUpdating(prev => { const s = new Set(prev); s.delete(pluginId); return s; });
        refreshPlugins();
    }
}

async function doUninstall(confirmPluginIdRef, clearConfirm, clearFeedback, setFeedback, refreshPlugins) {
    const pluginId = confirmPluginIdRef.current;
    clearConfirm();
    if (!pluginId) return;
    clearFeedback();
    try {
        await uninstallInstalledPlugin(pluginId);
        setFeedback('success', `Uninstalled ${pluginId}`);
        await refreshPlugins();
    } catch (error) {
        setFeedback('error', `Failed to uninstall ${pluginId}: ${error.message}`);
    }
}

function doOpenSelected(pluginsRef, selectedIndexRef, setFeedback, onOpenPluginConfig) {
    const plugin = pluginsRef.current[selectedIndexRef.current];
    if (!plugin) return;
    if (plugin.loaded === false) {
        setFeedback('error', `Plugin ${plugin.name} is not loaded${plugin.load_error ? `: ${plugin.load_error}` : ''}`);
        return;
    }
    if (plugin.has_ui) {
        if (onOpenPluginConfig) onOpenPluginConfig(plugin.id);
        return;
    }
    setFeedback('info', `No settings UI available for ${plugin.name}`);
}

export function usePluginActions(list, modal, setFeedback, clearFeedback, onOpenPluginConfig) {
    const [updating, setUpdating, updatingRef] = useStateRef(new Set());
    const updatePlugin = useCallback(
        pluginId => doUpdate(pluginId, updatingRef, clearFeedback, setUpdating, setFeedback, list.refreshPlugins),
        [clearFeedback, setFeedback, list.refreshPlugins]
    );
    const confirmUninstall = useCallback(
        () => doUninstall(modal.confirmPluginIdRef, modal.clearConfirm, clearFeedback, setFeedback, list.refreshPlugins),
        [clearFeedback, setFeedback, list.refreshPlugins, modal.clearConfirm]
    );
    const openSelected = useCallback(
        () => doOpenSelected(list.pluginsRef, list.selectedIndexRef, setFeedback, onOpenPluginConfig),
        [setFeedback, onOpenPluginConfig]
    );
    const navigateInGrid = useGridNav('#plugins-grid .plugin-card:not(.ghost)', list.selectedIndexRef, list.setSelectedIndex);
    const isBlocking = useCallback(
        () => modal.confirmPluginIdRef.current !== null || modal.contextMenuOpenRef.current,
        []
    );
    return { updating, updatePlugin, confirmUninstall, openSelected, navigateInGrid, isBlocking };
}
