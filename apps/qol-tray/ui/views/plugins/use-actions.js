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

async function doOpenSelected(pluginsRef, selectedIndexRef, onOpenPluginConfig, onOpenPluginUi) {
    const plugin = pluginsRef.current[selectedIndexRef.current];
    if (!plugin) return;
    if (plugin.loaded === false) {
        toast('error', `${plugin.name} failed to load: ${plugin.load_error}`);
        return;
    }
    if (plugin.has_config) {
        const opened = await onOpenPluginConfig(plugin.id);
        if (!opened) toast('info', `No settings available for ${plugin.name}`);
        return;
    }
    if (plugin.has_custom_ui) {
        onOpenPluginUi(plugin.id);
        return;
    }
}

export function usePluginActions(list, modal, onOpenPluginConfig, onOpenPluginUi) {
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
        () => doOpenSelected(list.pluginsRef, list.selectedIndexRef, onOpenPluginConfig, onOpenPluginUi),
        [onOpenPluginConfig, onOpenPluginUi]
    );
    const openConfig = useCallback(() => {
        const plugin = list.pluginsRef.current?.[list.selectedIndexRef.current];
        if (!plugin?.has_config) return;
        onOpenPluginConfig(plugin.id);
    }, [onOpenPluginConfig]);
    const navigateInGrid = useGridNav('#plugins-grid .plugin-card:not(.ghost)', list.selectedIndexRef, list.setSelectedIndex);
    const isBlocking = useCallback(
        () => modal.confirmPluginIdRef.current !== null || modal.contextMenuOpenRef.current,
        []
    );
    return { updating, updatePlugin, confirmUninstall, openSelected, openConfig, navigateInGrid, isBlocking };
}
