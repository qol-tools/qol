export function routeDevClick({
    event, state, actionsController, discoveryController, mockController,
    cpuController, closePluginMenu, togglePluginMenu, syncPluginMenuDom, updateView
}) {
    const target = readEventTarget(event);
    if (!target) return;
    const actionTarget = target.closest('[data-action]');
    const action = actionTarget?.dataset.action;
    const actionId = actionTarget?.dataset.id;
    if (!action) {
        if (state.openPluginMenuId) { closePluginMenu(); syncPluginMenuDom(); }
        return;
    }
    if (action === 'mock-update') { void mockController.triggerMockFlows(); return; }
    if (dispatchMenuToggle(action, actionId, event, togglePluginMenu, syncPluginMenuDom)) return;
    if (dispatchMenuItemAction(action, actionId, event, actionsController, cpuController, closePluginMenu, syncPluginMenuDom)) return;
    if (dispatchLinkAction(action, actionId, target, state, actionsController, updateView)) return;
    dispatchGlobalAction(action, actionsController, discoveryController);
}

function readEventTarget(event) {
    return event.target instanceof Element ? event.target : event.target?.parentElement;
}

function dispatchMenuToggle(action, actionId, event, togglePluginMenu, syncPluginMenuDom) {
    if (action !== 'toggle-plugin-menu' || !actionId) return false;
    event.preventDefault();
    event.stopPropagation();
    togglePluginMenu(actionId);
    syncPluginMenuDom();
    return true;
}

function dispatchMenuItemAction(action, actionId, event, actionsController, cpuController, closePluginMenu, syncPluginMenuDom) {
    if (!actionId) return false;
    if (action === 'toggle-plugin-logs') { runMenuAction(event, closePluginMenu, syncPluginMenuDom, () => void actionsController.togglePluginLogs(actionId)); return true; }
    if (action === 'edit-plugin-log-filters') { runMenuAction(event, closePluginMenu, syncPluginMenuDom, () => void actionsController.editPluginLogFilters(actionId)); return true; }
    if (action === 'toggle-plugin-cpu') { runMenuAction(event, closePluginMenu, syncPluginMenuDom, () => cpuController.toggle(actionId)); return true; }
    return false;
}

function dispatchLinkAction(action, actionId, target, state, actionsController, updateView) {
    if (action !== 'toggle-link' || !actionId) return false;
    if (state.linkingId) return true;
    const row = target.closest('.plugin-row');
    if (row) state.selectedIndex = parseInt(row.dataset.index, 10);
    actionsController.handleItemActivation();
    updateView();
    return true;
}

function dispatchGlobalAction(action, actionsController, discoveryController) {
    if (action === 'reload') { void actionsController.reloadPlugins(); return; }
    if (action === 'refresh-discovery') { void discoveryController.triggerDiscovery(); return; }
    if (action === 'add-link') { actionsController.showLinkInput(); return; }
    if (action === 'confirm-link') { void actionsController.confirmLink(); return; }
    if (action === 'cancel-link') { actionsController.cancelLink(); }
}

function runMenuAction(event, closePluginMenu, syncPluginMenuDom, callback) {
    event.preventDefault();
    event.stopPropagation();
    closePluginMenu();
    syncPluginMenuDom();
    callback();
}
