import { useCallback } from 'preact/hooks';
import { useStateRef } from '../../lib/hooks/useStateRef.js';
import { findPluginById } from './data.js';

export function usePluginsModal(plugins) {
    const [contextMenuOpen, setContextMenuOpen, contextMenuOpenRef] = useStateRef(false);
    const [confirmPluginId, setConfirmPluginId, confirmPluginIdRef] = useStateRef(null);
    const closeAll = useCallback(() => setContextMenuOpen(false), []);
    const clearConfirm = useCallback(() => setConfirmPluginId(null), []);
    const confirmPlugin = findPluginById(plugins, confirmPluginId);
    return {
        contextMenuOpen, setContextMenuOpen, contextMenuOpenRef,
        confirmPluginId, setConfirmPluginId, confirmPluginIdRef,
        closeAll, clearConfirm, confirmPlugin
    };
}
