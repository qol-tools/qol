function handleEscapeKey(event, state, bump) {
    if (!state.openPluginMenuId && !state.openCoreMenuId) return;
    event.preventDefault();
    state.openPluginMenuId = null;
    state.openCoreMenuId = null;
    bump();
}

export function handleDevKey(event, state, ctrl, bump) {
    if (state.showLinkInput) return;
    if ((event.ctrlKey || event.metaKey) && (event.key === 'r' || event.key === 'R')) {
        event.preventDefault();
        void ctrl.actionsController.reloadPlugins();
        return;
    }
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.key === 'Escape') { handleEscapeKey(event, state, bump); return; }
    if ((event.key === ' ' || event.key === 'Enter') && state.selectedIndex >= 0) { event.preventDefault(); ctrl.actionsController.handleItemActivation(); return; }
}
