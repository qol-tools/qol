import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../hooks/useStateRef.js';
import { useGridNav } from '../../hooks/useGridNav.js';
import { updateInstalledPlugin, uninstallInstalledPlugin } from './data.js';
import { toast } from '../../lib/toast.js';

async function doUpdate(pluginId, updatingRef, setUpdating, refreshPlugins) {
    if (updatingRef.current.has(pluginId)) return;
    setUpdating(prev => new Set(prev).add(pluginId));
    try {
        await updateInstalledPlugin(pluginId);
        toast('success', `Updated ${pluginId}`);
    } catch (error) {
        toast('error', `Failed to update ${pluginId}: ${error.message}`);
    } finally {
        setUpdating(prev => { const s = new Set(prev); s.delete(pluginId); return s; });
        refreshPlugins();
    }
}

async function doUninstall(confirmPluginIdRef, clearConfirm, refreshPlugins) {
    const pluginId = confirmPluginIdRef.current;
    clearConfirm();
    if (!pluginId) return;
    try {
        await uninstallInstalledPlugin(pluginId);
        toast('success', `Uninstalled ${pluginId}`);
        await refreshPlugins();
    } catch (error) {
        toast('error', `Failed to uninstall ${pluginId}: ${error.message}`);
    }
}

async function doOpenSelected(pluginsRef, selectedIndexRef, onOpenPluginConfig) {
    const plugin = pluginsRef.current[selectedIndexRef.current];
    if (!plugin) return;
    if (plugin.loaded === false) {
        toast('error', `Plugin ${plugin.name} is not loaded${plugin.load_error ? `: ${plugin.load_error}` : ''}`);
        return;
    }
    if (!plugin.has_ui) {
        toast('info', `No settings UI available for ${plugin.name}`);
        return;
    }
    if (!onOpenPluginConfig) return;
    const opened = await onOpenPluginConfig(plugin.id);
    if (!opened) toast('info', `No configuration available for ${plugin.name}`);
}

export function usePluginActions(list, modal, onOpenPluginConfig) {
    const [updating, setUpdating, updatingRef] = useStateRef(new Set());
    const updatePlugin = useCallback(
        pluginId => doUpdate(pluginId, updatingRef, setUpdating, list.refreshPlugins),
        [list.refreshPlugins]
    );
    const confirmUninstall = useCallback(
        () => doUninstall(modal.confirmPluginIdRef, modal.clearConfirm, list.refreshPlugins),
        [list.refreshPlugins, modal.clearConfirm]
    );
    const openSelected = useCallback(
        () => doOpenSelected(list.pluginsRef, list.selectedIndexRef, onOpenPluginConfig),
        [onOpenPluginConfig]
    );
    const navigateInGrid = useGridNav('#plugins-grid .plugin-card:not(.ghost)', list.selectedIndexRef, list.setSelectedIndex);
    const isBlocking = useCallback(
        () => modal.confirmPluginIdRef.current !== null || modal.contextMenuOpenRef.current,
        []
    );
    return { updating, updatePlugin, confirmUninstall, openSelected, navigateInGrid, isBlocking };
}
