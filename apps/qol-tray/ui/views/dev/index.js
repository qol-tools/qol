import { subscribe, onReconnect } from '../../events.js';
import { mergePlugins, renderBuildResults, renderPluginBuildMeta } from './plugin-model.js';
import { renderDevView } from './template.js';
import { createBuildController } from './build-controller.js';
import { createCpuController, readSavedCpuMonitoring } from './cpu-controller.js';
import { createDiscoveryController } from './discovery-controller.js';
import { createMockController } from './mock-controller.js';
import { createPluginActionsController } from './plugin-actions-controller.js';
import {
    nextDiscoveryCompletedState,
    nextDiscoveryStartedState
} from './discovery/reducer.js';

export const id = 'dev';

function saveSpinnerTimes(root) {
    const times = [];
    for (const btn of root.querySelectorAll('.refresh-btn.spinning')) {
        const anim = btn.getAnimations?.()[0];
        times.push(anim ? anim.currentTime : null);
    }
    return times;
}

function restoreSpinnerTimes(root, times) {
    if (!times.length) return;
    const buttons = root.querySelectorAll('.refresh-btn.spinning');
    for (let i = 0; i < buttons.length && i < times.length; i++) {
        if (times[i] === null) continue;
        const anim = buttons[i].getAnimations?.()[0];
        if (anim) anim.currentTime = times[i];
    }
}

function readSavedIndex() {
    return -1;
}

const state = {
    building: false,
    buildResults: null,
    lastReload: null,
    error: null,
    plugins: [],
    discovered: [],
    discovering: false,
    selectedIndex: readSavedIndex(),
    showLinkInput: false,
    linkPath: '',
    linkError: null,
    mergedList: [],
    mergedCount: 0,
    logControls: {},
    linkingId: null,
    buildProgress: {},
    mockTesting: false,
    cpuMonitoring: readSavedCpuMonitoring(),
    cpuByPlugin: {},
    openPluginMenuId: null
};

let container = null;
let unsubscribe = null;
let unsubscribeReconnect = null;
let focusRefreshPending = true;
let actionInteractionLocks = 0;
let deferredUpdatePending = false;

const discoveryController = createDiscoveryController({
    state,
    onNeedsRender: updateView
});

const cpuController = createCpuController({
    state,
    getVisiblePluginIds: visiblePluginIdSet,
    onNeedsRender: updateView,
    onMissingMenuPlugin: closePluginMenu
});

const actionsController = createPluginActionsController({
    state,
    discoveryController,
    getMergedPluginById,
    getActivePluginBuildState,
    closePluginMenu,
    onNeedsRender: updateView
});

let mockController = null;

const buildController = createBuildController({
    state,
    getContainer: () => container,
    getPluginById: getMergedPluginById,
    onNeedsRender: updateView,
    onBuildComplete: () => {
        actionsController.markReloadComplete();
        mockController?.completeMockTarget('plugin_build');
        updateView();
        void discoveryController.loadLinkedPlugins();
    }
});

mockController = createMockController({
    state,
    buildController,
    getMergedPlugins: () => state.mergedList,
    onNeedsRender: updateView
});

export function render(containerEl) {
    if (container) {
        container.removeEventListener('click', handleClick);
    }
    if (unsubscribe) {
        unsubscribe();
        unsubscribe = null;
    }

    container = containerEl;
    container.addEventListener('click', handleClick);
    unsubscribe = subscribe(handleEvent);

    if (!unsubscribeReconnect) {
        unsubscribeReconnect = onReconnect(() => {
            if (!state.building) return;
            if (mockController?.isMockTesting()) return;
            void buildController.hydrateBuildState();
        });
    }

    const hydrateBuildPromise = mockController.isMockTesting()
        ? Promise.resolve()
        : buildController.hydrateBuildState(true);
    const hydrateCpuPromise = cpuController.queueSync().then(() => cpuController.hydrate(true));

    void Promise.all([
        discoveryController.loadPlugins(true),
        discoveryController.fetchDiscoveryState(true),
        discoveryController.loadLogControls(true),
        hydrateBuildPromise,
        mockController.hydrateMockTargets(true),
        hydrateCpuPromise
    ]).finally(() => {
        if (state.linkingId) return;
        updateView();
    });
}

function handleEvent(event) {
    if (
        state.linkingId
        && (event.type === 'discovery_started'
            || event.type === 'discovery_complete'
            || event.type === 'plugins_changed')
    ) {
        return;
    }

    if (event.type === 'discovery_started') {
        Object.assign(state, nextDiscoveryStartedState());
        updateView();
        return;
    }

    if (event.type === 'discovery_complete') {
        Object.assign(state, nextDiscoveryCompletedState(event.plugins));
        updateView();
        return;
    }

    if (event.type === 'plugins_changed') {
        void discoveryController.loadLinkedPlugins();
        return;
    }

    if (cpuController.handleEvent(event)) {
        return;
    }

    buildController.handleEvent(event);
    mockController.handleEvent(event);
}

function totalItems() {
    return state.mergedCount || 0;
}

function closePluginMenu() {
    if (!state.openPluginMenuId) return;
    state.openPluginMenuId = null;
}

function togglePluginMenu(pluginId) {
    if (state.openPluginMenuId === pluginId) {
        state.openPluginMenuId = null;
        return;
    }
    state.openPluginMenuId = pluginId;
}

function syncPluginMenuDom() {
    if (!container) return;
    const rows = container.querySelectorAll('.plugin-row[data-plugin-id]');
    for (const row of rows) {
        const pluginId = row.dataset.pluginId;
        const isOpen = pluginId === state.openPluginMenuId;
        const menu = row.querySelector('.plugin-context-menu');
        if (menu) {
            menu.classList.toggle('open', isOpen);
        }
        const trigger = row.querySelector('.plugin-menu-trigger');
        if (trigger) {
            trigger.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
        }
    }
}

function visiblePluginIdSet() {
    return new Set(state.mergedList.map(plugin => plugin.id));
}

function getMergedPluginById(pluginId) {
    return state.mergedList.find(plugin => plugin.id === pluginId) || null;
}

function getActivePluginBuildState(plugin) {
    return buildController.getActivePluginBuildState(plugin, state.mockTesting);
}

function lockActionInteraction() {
    actionInteractionLocks += 1;
}

function unlockActionInteraction() {
    actionInteractionLocks = Math.max(0, actionInteractionLocks - 1);
    if (actionInteractionLocks !== 0) return;
    if (!deferredUpdatePending) return;
    deferredUpdatePending = false;
    updateView();
}

function bindActionInteractionLocks() {
    if (!container) return;
    const columns = container.querySelectorAll('.plugin-action-column');
    for (const column of columns) {
        column.addEventListener('pointerenter', lockActionInteraction);
        column.addEventListener('pointerleave', unlockActionInteraction);
    }
}

function updateView(force = false) {
    if (!force && actionInteractionLocks > 0) {
        deferredUpdatePending = true;
        return;
    }

    const mergedList = mergePlugins(state.discovered, state.plugins, state.logControls);
    state.mergedCount = mergedList.length;
    state.mergedList = mergedList;
    cpuController.prune(mergedList);

    if (mergedList.length === 0) {
        state.selectedIndex = -1;
    }
    if (mergedList.length > 0) {
        state.selectedIndex = Math.max(-1, Math.min(state.selectedIndex, mergedList.length - 1));
    }

    buildController.pruneInvisibleProgress(new Set(mergedList.map(plugin => plugin.id)));

    const prevScrollTop = container.querySelector('.view-body')?.scrollTop ?? 0;
    const spinnerTimes = saveSpinnerTimes(container);
    const hoveredActionZone = container.querySelector('.plugin-action-zone:hover');
    const hoveredActionId = hoveredActionZone?.dataset.id || null;

    container.innerHTML = renderDevView({
        state,
        mergedList,
        getActivePluginBuildState,
        renderPluginBuildMeta,
        renderBuildResults
    });

    const viewBody = container.querySelector('.view-body');
    if (viewBody) viewBody.scrollTop = prevScrollTop;
    restoreSpinnerTimes(container, spinnerTimes);

    if (hoveredActionId) {
        const actionZones = container.querySelectorAll('.plugin-action-zone[data-id]');
        for (const zone of actionZones) {
            if (zone.dataset.id !== hoveredActionId) continue;
            if (zone.classList.contains('is-disabled')) continue;
            zone.classList.add('is-hovered');
            break;
        }
    }

    const input = container.querySelector('#link-path');
    if (input) {
        input.addEventListener('input', event => {
            state.linkPath = event.target.value;
        });
        input.addEventListener('keydown', event => {
            if (event.key === 'Enter') actionsController.confirmLink();
            if (event.key === 'Escape') actionsController.cancelLink();
        });
    }

    buildController.cacheRows();
    buildController.syncAll();
    bindActionInteractionLocks();
}

function handleClick(event) {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    if (!target) return;

    const actionTarget = target.closest('[data-action]');
    const action = actionTarget?.dataset.action;
    const actionId = actionTarget?.dataset.id;

    if (!action) {
        if (!state.openPluginMenuId) return;
        closePluginMenu();
        syncPluginMenuDom();
        return;
    }

    if (action === 'mock-update') {
        void mockController.triggerMockFlows();
        return;
    }

    if (action === 'toggle-plugin-menu' && actionId) {
        event.preventDefault();
        event.stopPropagation();
        togglePluginMenu(actionId);
        syncPluginMenuDom();
        return;
    }

    if (action === 'toggle-plugin-logs' && actionId) {
        event.preventDefault();
        event.stopPropagation();
        closePluginMenu();
        syncPluginMenuDom();
        void actionsController.togglePluginLogs(actionId);
        return;
    }

    if (action === 'edit-plugin-log-filters' && actionId) {
        event.preventDefault();
        event.stopPropagation();
        closePluginMenu();
        syncPluginMenuDom();
        void actionsController.editPluginLogFilters(actionId);
        return;
    }

    if (action === 'toggle-plugin-cpu' && actionId) {
        event.preventDefault();
        event.stopPropagation();
        closePluginMenu();
        syncPluginMenuDom();
        cpuController.toggle(actionId);
        return;
    }

    if (action === 'toggle-link' && actionId) {
        if (state.linkingId) return;
        const row = target.closest('.plugin-row');
        if (row) {
            state.selectedIndex = parseInt(row.dataset.index, 10);
        }
        actionsController.handleItemActivation();
        updateView();
        return;
    }

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

export function handleKey(event) {
    if (state.showLinkInput) return;

    if ((event.ctrlKey || event.metaKey) && (event.key === 'r' || event.key === 'R')) {
        event.preventDefault();
        void actionsController.reloadPlugins();
        return;
    }

    if (event.ctrlKey || event.altKey || event.metaKey) return;

    if (event.key === 'Escape') {
        if (!state.openPluginMenuId) return;
        event.preventDefault();
        closePluginMenu();
        syncPluginMenuDom();
        return;
    }

    const total = totalItems();

    if (event.key === 'ArrowDown' && total > 0) {
        event.preventDefault();
        if (state.selectedIndex < 0) {
            state.selectedIndex = 0;
            updateView();
            return;
        }
        state.selectedIndex = Math.min(state.selectedIndex + 1, total - 1);
        updateView();
        return;
    }

    if (event.key === 'ArrowUp' && total > 0) {
        event.preventDefault();
        if (state.selectedIndex < 0) {
            state.selectedIndex = total - 1;
            updateView();
            return;
        }
        state.selectedIndex = Math.max(state.selectedIndex - 1, 0);
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
        if (!item) return;
        togglePluginMenu(item.id);
        syncPluginMenuDom();
    }
}

export function onFocus() {
    if (state.linkingId) return;
    if (!focusRefreshPending) return;
    focusRefreshPending = false;
    void Promise.all([
        discoveryController.loadPlugins(true),
        discoveryController.fetchDiscoveryState(true),
        discoveryController.loadLogControls(true),
        mockController.hydrateMockTargets(true)
    ]).finally(() => {
        updateView();
    });
}

export function onBlur() {
    closePluginMenu();
    focusRefreshPending = true;
}

export function destroy() {
    if (unsubscribe) {
        unsubscribe();
        unsubscribe = null;
    }
    if (unsubscribeReconnect) {
        unsubscribeReconnect();
        unsubscribeReconnect = null;
    }
    if (container) {
        container.removeEventListener('click', handleClick);
        container = null;
    }
    cpuController.destroy();
    actionInteractionLocks = 0;
    deferredUpdatePending = false;
}
