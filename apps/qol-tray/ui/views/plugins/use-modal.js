import { useCallback } from 'preact/hooks';
import { findPluginById } from './data.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';
import { uninstallConfirmSlot } from './uninstall-confirm-subpage.js';
import { pluginActionsSlot } from './plugin-actions-subpage.js';
import { uninstallInstalledPlugin } from './data.js';
import { bindPluginContextMenuItems } from '../../lib/plugin-context-menu-items.js';
import { toast } from '../../lib/toast.js';

const PLUGIN_ACTIONS_DIVE_SELECTOR = '[data-dive-source="plugins-actions"]';

export function usePluginsModal(plugins, refreshPlugins) {
    const triggerUninstallConfirm = useCallback((pluginId) => {
        if (!pluginId) return;
        const plugin = findPluginById(plugins, pluginId);
        uninstallConfirmSlot.set({
            pluginId,
            pluginName: plugin?.name || pluginId,
            confirm: async () => {
                try {
                    await uninstallInstalledPlugin(pluginId);
                    toast('success', `Uninstalled ${pluginId}`);
                    if (refreshPlugins) await refreshPlugins();
                } catch (error) {
                    toast('error', `Failed to uninstall ${pluginId}: ${error.message}`);
                }
            },
        });
        diveViaSelector('[data-view-id="plugins"]');
    }, [plugins, refreshPlugins]);
    const triggerActionsMenu = useCallback((pluginId, ctx) => {
        if (!pluginId) return;
        const plugin = findPluginById(plugins, pluginId);
        if (!plugin) return;
        const items = bindPluginContextMenuItems(plugin, ctx);
        if (items.length === 0) return;
        pluginActionsSlot.set({
            rowId: pluginId,
            rowName: plugin.name || pluginId,
            items,
        });
        diveViaSelector(PLUGIN_ACTIONS_DIVE_SELECTOR);
    }, [plugins]);
    return { triggerUninstallConfirm, triggerActionsMenu };
}
