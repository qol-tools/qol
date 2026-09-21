import { useCallback } from 'preact/hooks';

function routeCardClick(e, index, pluginId, list, actions) {
    const updateBtn = e.target.closest('.plugin-update:not([disabled])');
    if (updateBtn) { e.stopPropagation(); actions.updatePlugin(pluginId); return; }
    if (index !== list.selectedIndexRef.current) list.setSelectedIndex(index);
    else actions.openSelected();
}

export function useCardClickHandler(list, actions) {
    return useCallback(
        (e, i, id) => routeCardClick(e, i, id, list, actions),
        [actions.updatePlugin, actions.openSelected]
    );
}
