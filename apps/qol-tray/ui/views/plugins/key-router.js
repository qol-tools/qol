import { useCallback } from 'preact/hooks';
import { withShiftVariants, dispatchKey } from '../../utils/keys.js';

function routePluginsKey(e, list, modal, actions) {
    if (modal.confirmPluginIdRef.current !== null) {
        routeConfirmKey(e, modal.clearConfirm, actions.confirmUninstall);
        return;
    }
    if (modal.contextMenuOpenRef.current) {
        routeContextMenuKey(e, list, modal);
        return;
    }
    routeNormalKey(e, list, modal, actions);
}

function routeConfirmKey(e, clearConfirm, confirmUninstall) {
    if (e.key === 'Escape') { e.preventDefault(); clearConfirm(); }
    else if (e.key === 'Enter') { e.preventDefault(); confirmUninstall(); }
}

function routeContextMenuKey(e, list, modal) {
    e.preventDefault();
    if (e.key === 'Escape') { modal.closeAll(); return; }
    if (e.key !== 'Enter') return;
    const plugin = list.pluginsRef.current[list.selectedIndexRef.current];
    if (plugin) { modal.closeAll(); modal.setConfirmPluginId(plugin.id); }
}

function routeNormalKey(e, list, modal, actions) {
    dispatchKey(e, withShiftVariants({
        ArrowUp: () => actions.navigateInGrid('up'),
        ArrowDown: () => actions.navigateInGrid('down'),
        ArrowLeft: () => actions.navigateInGrid('left'),
        ArrowRight: () => actions.navigateInGrid('right'),
        Enter: actions.openSelected,
        d: () => { const p = list.pluginsRef.current[list.selectedIndexRef.current]; if (p) modal.setConfirmPluginId(p.id); },
        u: () => { const p = list.pluginsRef.current[list.selectedIndexRef.current]; if (p?.update_available) actions.updatePlugin(p.id); },
        m: () => modal.setContextMenuOpen(prev => !prev),
    }));
}

export function usePluginsKeyHandler(list, modal, actions) {
    return useCallback(
        e => routePluginsKey(e, list, modal, actions),
        [actions.confirmUninstall, actions.openSelected, actions.updatePlugin, actions.navigateInGrid]
    );
}
