import { useCallback } from 'preact/hooks';

function routeCardClick(e, index, pluginId, list, modal, actions) {
    const configStrip = e.target.closest('.plugin-config-strip');
    if (configStrip) { e.stopPropagation(); list.setSelectedIndex(index); actions.openConfig(); return; }
    const updateBtn = e.target.closest('.plugin-update:not([disabled])');
    if (updateBtn) { e.stopPropagation(); actions.updatePlugin(pluginId); return; }
    const cogBtn = e.target.closest('.plugin-cog');
    if (cogBtn) { e.stopPropagation(); list.setSelectedIndex(index); modal.setContextMenuOpen(prev => !prev); return; }
    const ctxUpdate = e.target.closest('.context-update');
    if (ctxUpdate) { e.stopPropagation(); modal.closeAll(); actions.updatePlugin(pluginId); return; }
    const ctxDelete = e.target.closest('.context-delete');
    if (ctxDelete) { e.stopPropagation(); modal.closeAll(); modal.setConfirmPluginId(pluginId); return; }
    if (index !== list.selectedIndexRef.current) list.setSelectedIndex(index);
    else actions.openSelected();
}

export function useCardClickHandler(list, modal, actions) {
    return useCallback(
        (e, i, id) => routeCardClick(e, i, id, list, modal, actions),
        [actions.updatePlugin, actions.openSelected, actions.openConfig]
    );
}
