import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { findPluginById } from './data.js';
import { diveViaSelector } from '../../lib/world-navigation-singleton.js';
import { uninstallConfirmSlot } from './uninstall-confirm-subpage.js';
import { uninstallInstalledPlugin } from './data.js';
import { toast } from '../../lib/toast.js';

export function usePluginsModal(plugins, refreshPlugins) {
    const [contextMenuOpen, setContextMenuOpen, contextMenuOpenRef] = useStateRef(false);
    const closeAll = useCallback(() => setContextMenuOpen(false), []);
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
    return {
        contextMenuOpen, setContextMenuOpen, contextMenuOpenRef,
        closeAll, triggerUninstallConfirm,
    };
}
