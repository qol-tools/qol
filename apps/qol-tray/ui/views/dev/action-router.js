export function routeDevClick({
    event,
    state,
    actionsController,
    discoveryController,
    mockController,
    cpuController,
    closePluginMenu,
    togglePluginMenu,
    syncPluginMenuDom,
    updateView
}) {
    const target = readEventTarget(event);
    if (!target) {
        return;
    }

    const actionTarget = target.closest('[data-action]');
    const action = actionTarget?.dataset.action;
    const actionId = actionTarget?.dataset.id;
    const context = {
        event,
        state,
        target,
        action,
        actionId,
        actionsController,
        discoveryController,
        mockController,
        cpuController,
        closePluginMenu,
        togglePluginMenu,
        syncPluginMenuDom,
        updateView
    };

    if (!action) {
        handleOutsideClick(context);
        return;
    }

    if (handleMockAction(context)) {
        return;
    }

    if (handleMenuToggle(context)) {
        return;
    }

    if (handlePluginMenuAction(context)) {
        return;
    }

    if (handleLinkAction(context)) {
        return;
    }

    handleGlobalAction(context);
}

function readEventTarget(event) {
    return event.target instanceof Element ? event.target : event.target?.parentElement;
}

function handleOutsideClick({ state, closePluginMenu, syncPluginMenuDom }) {
    if (!state.openPluginMenuId) {
        return;
    }

    closePluginMenu();
    syncPluginMenuDom();
}

function handleMockAction({ action, mockController }) {
    if (action !== 'mock-update') {
        return false;
    }

    void mockController.triggerMockFlows();
    return true;
}

function handleMenuToggle({ action, actionId, event, togglePluginMenu, syncPluginMenuDom }) {
    if (action !== 'toggle-plugin-menu' || !actionId) {
        return false;
    }

    event.preventDefault();
    event.stopPropagation();
    togglePluginMenu(actionId);
    syncPluginMenuDom();
    return true;
}

function handlePluginMenuAction(context) {
    if (context.action === 'toggle-plugin-logs' && context.actionId) {
        runMenuAction(context, () => {
            void context.actionsController.togglePluginLogs(context.actionId);
        });
        return true;
    }

    if (context.action === 'edit-plugin-log-filters' && context.actionId) {
        runMenuAction(context, () => {
            void context.actionsController.editPluginLogFilters(context.actionId);
        });
        return true;
    }

    if (context.action === 'toggle-plugin-cpu' && context.actionId) {
        runMenuAction(context, () => {
            context.cpuController.toggle(context.actionId);
        });
        return true;
    }

    return false;
}

function handleLinkAction({ action, actionId, state, target, actionsController, updateView }) {
    if (action !== 'toggle-link' || !actionId) {
        return false;
    }

    if (state.linkingId) {
        return true;
    }

    const row = target.closest('.plugin-row');
    if (row) {
        state.selectedIndex = parseInt(row.dataset.index, 10);
    }
    actionsController.handleItemActivation();
    updateView();
    return true;
}

function handleGlobalAction({ action, actionsController, discoveryController }) {
    if (action === 'reload') {
        void actionsController.reloadPlugins();
        return;
    }

    if (action === 'refresh-discovery') {
        void discoveryController.triggerDiscovery();
        return;
    }

    if (action === 'add-link') {
        actionsController.showLinkInput();
        return;
    }

    if (action === 'confirm-link') {
        void actionsController.confirmLink();
        return;
    }

    if (action === 'cancel-link') {
        actionsController.cancelLink();
    }
}

function runMenuAction({ event, closePluginMenu, syncPluginMenuDom }, callback) {
    event.preventDefault();
    event.stopPropagation();
    closePluginMenu();
    syncPluginMenuDom();
    callback();
}
