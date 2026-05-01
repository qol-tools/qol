import { useState, useRef, useEffect, useCallback } from 'preact/hooks';
import { useSSE, useSSEReconnect } from '../../hooks/useSSE.js';
import { mergePlugins } from './plugin-model.js';
import { createDiscoveryController } from './discovery-controller.js';
import { createCpuController, readSavedCpuMonitoring } from './cpu-controller.js';
import { createBuildController } from './build-controller.js';
import { createMockController } from './mock-controller.js';
import { createCoreLogActions } from './core-log-actions.js';
import { createPluginActionsController } from './plugin-actions-controller.js';
import { nextDiscoveryCompletedState, nextDiscoveryStartedState } from './discovery/reducer.js';
import { handleDevKey } from './keys.js';

export function createInitialState() {
    return {
        building: false, buildResults: null, lastReload: null, error: null,
        plugins: [], discovered: [], discovering: false,
        selectedIndex: -1, showLinkInput: false, linkPath: '', linkError: null,
        mergedList: [], mergedCount: 0, logControls: {},
        linkingId: null, buildProgress: {}, mockTesting: false,
        cpuMonitoring: readSavedCpuMonitoring(),
        cpuByPlugin: {},
        coreLogControls: {}
    };
}

function initDataControllers(state, bump) {
    const discoveryController = createDiscoveryController({ state, onNeedsRender: bump });
    const cpuController = createCpuController({
        state,
        getVisiblePluginIds: () => new Set(state.mergedList.map(p => p.id)),
        onNeedsRender: bump
    });
    const coreLogActions = createCoreLogActions({ state, discoveryController, onNeedsRender: bump });
    return { discoveryController, cpuController, coreLogActions };
}

function makeBuildCompleteHandler(actionsCtrl, mockRef, discoveryController, bump) {
    return () => {
        actionsCtrl.markReloadComplete();
        mockRef.current?.completeMockTarget('plugin_build');
        bump();
        void discoveryController.loadLinkedPlugins();
    };
}

function initBuildControllers(state, containerRef, dataCtrl, bump) {
    const { discoveryController } = dataCtrl;
    const buildRef = { current: null }; const mockRef = { current: null };
    const actionsController = createPluginActionsController({
        state, discoveryController,
        getMergedPluginById: id => state.mergedList.find(p => p.id === id) || null,
        getActivePluginBuildState: p => buildRef.current.getActivePluginBuildState(p, state.mockTesting),
        closePluginMenu: () => {},
        onNeedsRender: bump
    });
    buildRef.current = createBuildController({
        state, getContainer: () => containerRef.current,
        getPluginById: id => state.mergedList.find(p => p.id === id) || null,
        onNeedsRender: bump,
        onBuildComplete: makeBuildCompleteHandler(actionsController, mockRef, discoveryController, bump)
    });
    mockRef.current = createMockController({ state, buildController: buildRef.current, getMergedPlugins: () => state.mergedList, onNeedsRender: bump });
    return { actionsController, buildController: buildRef.current, mockController: mockRef.current };
}

function syncMergedList(state, ctrl) {
    const mergedList = mergePlugins(state.discovered, state.plugins, state.logControls);
    state.mergedCount = mergedList.length;
    state.mergedList = mergedList;
    ctrl.cpuController.prune(mergedList);
    if (mergedList.length === 0) state.selectedIndex = -1;
    if (mergedList.length > 0) {
        state.selectedIndex = Math.max(-1, Math.min(state.selectedIndex, mergedList.length - 1));
    }
    ctrl.buildController.pruneInvisibleProgress(new Set(mergedList.map(p => p.id)));
}

function handleSSEEvent(event, state, ctrl, bump) {
    if (state.linkingId && (event.type === 'discovery_started' || event.type === 'plugins_changed')) return;
    if (event.type === 'discovery_started') { Object.assign(state, nextDiscoveryStartedState()); bump(); return; }
    if (event.type === 'discovery_complete') { Object.assign(state, nextDiscoveryCompletedState(event.plugins)); bump(); return; }
    if (event.type === 'plugins_changed') { void ctrl.discoveryController.loadLinkedPlugins(); return; }
    if (ctrl.cpuController.handleEvent(event)) return;
    ctrl.buildController.handleEvent(event);
    ctrl.mockController.handleEvent(event);
}

function useSSESubscription(state, ctrl, bump) {
    useSSE(event => handleSSEEvent(event, state, ctrl, bump));
}

function useReconnectSubscription(state, ctrl) {
    useSSEReconnect(() => {
        if (!state.building) return;
        if (ctrl.mockController.isMockTesting()) return;
        void ctrl.buildController.hydrateBuildState();
    });
}

function useHydration(state, ctrl, bump) {
    useEffect(() => {
        const { discoveryController: disc, buildController: build, cpuController: cpu, mockController: mock } = ctrl;
        const buildPromise = mock.isMockTesting() ? Promise.resolve() : build.hydrateBuildState(true);
        const cpuPromise = cpu.queueSync().then(() => cpu.hydrate(true));
        void Promise.all([
            disc.loadPlugins(true), disc.fetchDiscoveryState(true),
            disc.loadLogControls(true), disc.loadCoreLogControls(true),
            buildPromise, mock.hydrateMockTargets(true), cpuPromise
        ]).finally(() => { if (!state.linkingId) bump(); });
    }, []);
}

function useFocusLifecycle(state, ctrl, bump) {
    const focusRefreshPending = useRef(true);
    const onFocus = useCallback(() => {
        if (state.linkingId || !focusRefreshPending.current) return;
        focusRefreshPending.current = false;
        const { discoveryController: disc, mockController: mock } = ctrl;
        void Promise.all([
            disc.loadPlugins(true), disc.fetchDiscoveryState(true),
            disc.loadLogControls(true), disc.loadCoreLogControls(true),
            mock.hydrateMockTargets(true)
        ]).finally(() => bump());
    }, []);
    const onBlur = useCallback(() => {
        focusRefreshPending.current = true;
    }, []);
    return { onFocus, onBlur };
}

function buildLinkCallbacks(state, ctrl, bump) {
    return {
        reloadPlugins: () => void ctrl.actionsController.reloadPlugins(),
        triggerMockFlows: () => void ctrl.mockController.triggerMockFlows(),
        triggerDiscovery: () => void ctrl.discoveryController.triggerDiscovery(),
        openLinkInput: () => { ctrl.actionsController.showLinkInput(); bump(); },
        confirmLink: () => void ctrl.actionsController.confirmLink(),
        cancelLink: () => { ctrl.actionsController.cancelLink(); bump(); },
        handleItemActivation: () => ctrl.actionsController.handleItemActivation(),
        setSelectedIndex: index => { state.selectedIndex = index; bump(); },
        onLinkInput: value => { state.linkPath = value; }
    };
}

function buildMenuCallbacks(state, ctrl, bump) {
    return {
        togglePluginLogs: id => void ctrl.actionsController.togglePluginLogs(id),
        editPluginLogFilters: id => void ctrl.actionsController.editPluginLogFilters(id),
        toggleCpu: id => ctrl.cpuController.toggle(id),
        toggleCoreLogs: id => void ctrl.coreLogActions.toggleCoreLogs(id),
        editCoreLogFilters: id => void ctrl.coreLogActions.editCoreLogFilters(id)
    };
}

function buildActionCallbacks(state, ctrl, bump) {
    return { ...buildLinkCallbacks(state, ctrl, bump), ...buildMenuCallbacks(state, ctrl, bump) };
}

function buildControllerInterface(state, ctrl, bump, lifecycle) {
    return {
        ...state,
        ...buildActionCallbacks(state, ctrl, bump),
        ...lifecycle,
        buildController: ctrl.buildController,
        cpuController: ctrl.cpuController,
        getActivePluginBuildState: p => ctrl.buildController.getActivePluginBuildState(p, state.mockTesting)
    };
}

export function useDevController(containerRef) {
    const stateRef = useRef(null);
    if (!stateRef.current) stateRef.current = createInitialState();
    const state = stateRef.current;
    const [, setTick] = useState(0);
    const bump = useCallback(() => setTick(t => t + 1), []);
    const dataCtrlRef = useRef(null);
    if (!dataCtrlRef.current) dataCtrlRef.current = initDataControllers(state, bump);
    const buildCtrlRef = useRef(null);
    if (!buildCtrlRef.current) buildCtrlRef.current = initBuildControllers(state, containerRef, dataCtrlRef.current, bump);
    const ctrl = { ...dataCtrlRef.current, ...buildCtrlRef.current };
    syncMergedList(state, ctrl);
    useSSESubscription(state, ctrl, bump);
    useReconnectSubscription(state, ctrl);
    useHydration(state, ctrl, bump);
    const { onFocus, onBlur } = useFocusLifecycle(state, ctrl, bump);
    const handleKey = useCallback(e => handleDevKey(e, state, ctrl), []);
    return buildControllerInterface(state, ctrl, bump, { onFocus, onBlur, handleKey });
}
