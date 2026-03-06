import { subscribe, onReconnect } from '../../events.js';
import { mergePlugins, renderBuildResults, renderPluginBuildMeta } from './plugin-model.js';
import { renderDevView } from './template.js';
import { createBuildController } from './build-controller.js';
import { createCpuController, readSavedCpuMonitoring } from './cpu-controller.js';
import { createDiscoveryController } from './discovery-controller.js';
import { createMockController } from './mock-controller.js';
import { createPluginActionsController } from './plugin-actions-controller.js';
import { routeDevClick } from './action-router.js';
import { routeDevKey } from './key-router.js';
import {
    nextDiscoveryCompletedState,
    nextDiscoveryStartedState
} from './discovery/reducer.js';
import {
    bindActionInteractionLocks as bindActionLocks,
    bindLinkInput,
    readHoveredActionId,
    restoreHoveredAction,
    restoreSpinnerTimes,
    restoreViewBodyScroll,
    saveSpinnerTimes,
    syncPluginMenuDom as syncPluginMenuState
} from './view-dom.js';

export const id = 'dev';

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
    syncPluginMenuState(container, state.openPluginMenuId);
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

function bindActionInteractionZones() {
    if (!container) return;
    bindActionLocks(container, {
        onEnter: lockActionInteraction,
        onLeave: unlockActionInteraction
    });
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
    const hoveredActionId = readHoveredActionId(container);

    container.innerHTML = renderDevView({
        state,
        mergedList,
        getActivePluginBuildState,
        renderPluginBuildMeta,
        renderBuildResults
    });

    restoreViewBodyScroll(container, prevScrollTop);
    restoreSpinnerTimes(container, spinnerTimes);
    restoreHoveredAction(container, hoveredActionId);
    bindLinkInput(container, {
        onInput: value => {
            state.linkPath = value;
        },
        onConfirm: () => actionsController.confirmLink(),
        onCancel: () => actionsController.cancelLink()
    });

    buildController.cacheRows();
    buildController.syncAll();
    bindActionInteractionZones();
}

function handleClick(event) {
    routeDevClick({
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
    });
}

export function handleKey(event) {
    routeDevKey({
        event,
        state,
        actionsController,
        discoveryController,
        closePluginMenu,
        togglePluginMenu,
        syncPluginMenuDom,
        updateView
    });
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
