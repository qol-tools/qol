export function routeDevKey({
    event,
    state,
    actionsController,
    discoveryController,
    closePluginMenu,
    togglePluginMenu,
    syncPluginMenuDom,
    updateView
}) {
    if (state.showLinkInput) {
        return;
    }

    if ((event.ctrlKey || event.metaKey) && (event.key === 'r' || event.key === 'R')) {
        event.preventDefault();
        void actionsController.reloadPlugins();
        return;
    }

    if (event.ctrlKey || event.altKey || event.metaKey) {
        return;
    }

    if (event.key === 'Escape') {
        if (!state.openPluginMenuId) {
            return;
        }

        event.preventDefault();
        closePluginMenu();
        syncPluginMenuDom();
        return;
    }

    const total = state.mergedCount || 0;
    if (event.key === 'ArrowDown' && total > 0) {
        event.preventDefault();
        state.selectedIndex = nextSelectedIndex(state.selectedIndex, total, 1);
        updateView();
        return;
    }

    if (event.key === 'ArrowUp' && total > 0) {
        event.preventDefault();
        state.selectedIndex = nextSelectedIndex(state.selectedIndex, total, -1);
        updateView();
        return;
    }

    if (event.key === ' ' || event.key === 'Enter') {
        event.preventDefault();
        actionsController.handleItemActivation();
        return;
    }

    if (event.key === 'r' || event.key === 'R') {
        event.preventDefault();
        void discoveryController.triggerDiscovery();
        return;
    }

    if (event.key === 'm' || event.key === 'M') {
        event.preventDefault();
        const item = state.mergedList[state.selectedIndex];
        if (!item) {
            return;
        }

        togglePluginMenu(item.id);
        syncPluginMenuDom();
    }
}

function nextSelectedIndex(selectedIndex, total, delta) {
    if (selectedIndex < 0) {
        return delta > 0 ? 0 : total - 1;
    }

    if (delta > 0) {
        return Math.min(selectedIndex + 1, total - 1);
    }

    return Math.max(selectedIndex - 1, 0);
}
