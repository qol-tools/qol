import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { useGridNav } from '../../lib/hooks/useGridNav.js';
import { updateInstalledPlugin } from './data.js';
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

async function doOpenSelected(pluginsRef, selectedIndexRef, onOpenPluginConfig) {
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
}

export function usePluginActions(list, modal, onOpenPluginConfig) {
    const [updating, setUpdating, updatingRef] = useStateRef(new Set());
    const updatePlugin = useCallback(
        pluginId => doUpdate(pluginId, updatingRef, setUpdating, list.refreshPlugins),
        [list.refreshPlugins]
    );
    const openSelected = useCallback(
        () => doOpenSelected(list.pluginsRef, list.selectedIndexRef, onOpenPluginConfig),
        [onOpenPluginConfig]
    );
    const openConfig = useCallback(() => {
        const plugin = list.pluginsRef.current?.[list.selectedIndexRef.current];
        if (!plugin?.has_config) return;
        onOpenPluginConfig(plugin.id);
    }, [onOpenPluginConfig]);
    const navigateInGrid = useGridNav('#plugins-grid .plugin-card:not(.ghost)', list.selectedIndexRef, list.setSelectedIndex);
    const focusSelectedCard = useCallback(() => {
        const plugin = list.pluginsRef.current?.[list.selectedIndexRef.current];
        if (!plugin) return;
        const card = document.querySelector(`#plugins-grid [data-plugin-id="${CSS.escape(plugin.id)}"]`);
        if (card instanceof HTMLElement) card.focus({ preventScroll: true });
    }, []);
    const isBlocking = useCallback(
        () => modal.contextMenuOpenRef.current,
        []
    );
    return { updating, updatePlugin, openSelected, openConfig, navigateInGrid, focusSelectedCard, isBlocking };
}
