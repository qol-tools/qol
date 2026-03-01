import { createMockTargetsPoller } from './mock/effects.js';
import {
    buildLegacyStartErrorMessage,
    fetchMockTargetsState,
    readMockStartError,
    startLegacyMockTargets,
    startMockTargetsApi,
    stopMockTargetsApi
} from './mock/api.js';
import { runLocalMockPluginBuild } from './mock/local-build.js';
import {
    beginMockFlowRun,
    completeBackendTarget,
    createMockFlowModel,
    finishMockTesting,
    getMockFlowSource,
    hasActiveBackendTargets,
    isCurrentMockRun,
    pruneInactiveTargets,
    setBackendTargets,
    setMockFlowSource,
    stopMockFlowRun
} from './mock/reducer.js';

export function createMockController({
    state,
    buildController,
    getMergedPlugins,
    onNeedsRender
}) {
    function maybeRender(skipUpdate) {
        if (!skipUpdate && !state.linkingId) onNeedsRender();
    }

    let mockFlowModel = createMockFlowModel();
    const poller = createMockTargetsPoller({
        onTick: async () => {
            await reconcileMockTargets();
        }
    });

    function handleEvent(event) {
        if (event.type === 'update_complete' || event.type === 'update_failed') {
            completeMockTarget('self_update');
            return;
        }
        if (event.type === 'self_recompile_complete' || event.type === 'self_recompile_failed') {
            completeMockTarget('self_recompile');
            return;
        }
        if (event.type === 'build_complete') {
            completeMockTarget('plugin_build');
        }
    }

    function isMockTesting() {
        return state.mockTesting;
    }

    function onFocus() {
        if (state.mockTesting && getMockFlowSource(mockFlowModel) === 'backend') {
            startPolling();
        }
    }

    function onBlur() {
        stopPolling();
    }

    async function hydrateMockTargets(skipUpdate = false) {
        if (getMockFlowSource(mockFlowModel) === 'local') {
            return;
        }

        const targetState = await fetchMockTargetsState();
        if (!targetState) {
            return;
        }

        const next = setBackendTargets(mockFlowModel, targetState.runningIds);
        mockFlowModel = next.model;
        state.mockTesting = next.mockTesting;

        (state.mockTesting ? startPolling : stopPolling)();
        maybeRender(skipUpdate);
    }

    async function triggerMockFlows() {
        if (state.mockTesting) {
            await stopMockFlows();
            return;
        }

        const started = beginMockFlowRun(mockFlowModel);
        mockFlowModel = started.model;
        state.mockTesting = started.mockTesting;
        const runId = started.runId;
        state.error = null;
        onNeedsRender();

        const startRes = await startMockTargetsApi();
        if (!isCurrent(runId)) return;

        if (startRes instanceof Error) {
            state.error = startRes.message || 'Failed to trigger mock targets';
            finishTestingReducer();
            stopPolling();
            onNeedsRender();
            return;
        }

        if (startRes.ok) {
            let startedTargets = [];
            try {
                const payload = await startRes.json();
                if (Array.isArray(payload?.started)) {
                    startedTargets = payload.started.filter(id => typeof id === 'string');
                }
            } catch (error) {}

            if (startedTargets.length === 0) {
                startedTargets = ['self_update', 'self_recompile', 'plugin_build'];
            }

            const targetState = setBackendTargets(mockFlowModel, startedTargets);
            mockFlowModel = targetState.model;
            state.mockTesting = targetState.mockTesting;
            (targetState.mockTesting ? startPolling : stopPolling)();
            onNeedsRender();
            return;
        }

        if (startRes.status !== 404) {
            state.error = await readMockStartError(startRes);
            finishTestingReducer();
            onNeedsRender();
            return;
        }

        const { updateRes, recompileRes, buildRes } = await startLegacyMockTargets();
        if (!isCurrent(runId)) return;

        const needsLocalFallback = !buildRes || buildRes.status === 404;
        if (needsLocalFallback) {
            mockFlowModel = setMockFlowSource(mockFlowModel, 'local');
            stopPolling();
            const completed = await runLocalMockPluginBuild({
                getPluginIds: () => getMergedPlugins().map(plugin => plugin.id),
                isCurrentRun: () => isCurrent(runId),
                setBuildStarted: () => {
                    state.building = true;
                    state.buildResults = null;
                    state.buildProgress = {};
                },
                setPluginQueued: pluginId => {
                    state.buildProgress[pluginId] = {
                        status: 'queued',
                        percent: 0,
                        phase: 'Queued'
                    };
                },
                setPluginBuilding: (pluginId, percent, phase) => {
                    state.buildProgress[pluginId] = {
                        status: 'building',
                        percent,
                        phase
                    };
                },
                setBuildCompleted: results => {
                    state.building = false;
                    state.buildResults = results;
                },
                onRender: onNeedsRender,
                onQueueBuildSync: pluginId => buildController.queueBuildRowSync(pluginId),
                onClearQueuedBuildSync: () => buildController.clearQueuedBuildRowSync()
            });
            if (!completed || !isCurrent(runId)) return;
            finishTestingReducer();
            stopPolling();
        }

        if (!needsLocalFallback && buildRes.ok) {
            const backendTargets = setBackendTargets(mockFlowModel, ['plugin_build']);
            mockFlowModel = backendTargets.model;
            state.mockTesting = backendTargets.mockTesting;
            startPolling();
        }

        const failure = await buildLegacyStartErrorMessage({
            updateRes,
            recompileRes,
            buildRes,
            needsLocalFallback
        });
        if (failure.message) {
            state.error = failure.message;
        }

        if (failure.buildFailed) {
            finishTestingReducer();
            stopPolling();
        }

        onNeedsRender();
    }

    async function stopMockFlows() {
        if (!state.mockTesting) return;

        const source = getMockFlowSource(mockFlowModel);
        const stopped = stopMockFlowRun(mockFlowModel);
        mockFlowModel = stopped.model;
        state.mockTesting = stopped.mockTesting;
        stopPolling();

        if (source === 'local') {
            buildController.stopLocalMockBuildUi();
        }
        onNeedsRender();

        if (source !== 'backend') {
            return;
        }

        await stopMockTargetsApi();
    }

    async function reconcileMockTargets() {
        if (!state.mockTesting || !hasActiveBackendTargets(mockFlowModel)) return;
        if (mockFlowModel.activeTargets.size === 0) {
            finishTesting();
            return;
        }

        const targetState = await fetchMockTargetsState();
        if (!targetState) {
            return;
        }

        const pruned = pruneInactiveTargets(mockFlowModel, targetState.runningById);
        mockFlowModel = pruned.model;
        if (pruned.done) {
            finishTesting();
            return;
        }
        if (pruned.changed) {
            onNeedsRender();
        }
    }

    function completeMockTarget(targetId) {
        const completed = completeBackendTarget(mockFlowModel, targetId, state.mockTesting);
        mockFlowModel = completed.model;
        state.mockTesting = completed.mockTesting;
        if (completed.completed) {
            stopPolling();
            onNeedsRender();
        }
    }

    function finishTesting() {
        finishTestingReducer();
        stopPolling();
        onNeedsRender();
    }

    function finishTestingReducer() {
        const next = finishMockTesting(mockFlowModel);
        mockFlowModel = next.model;
        state.mockTesting = next.mockTesting;
    }

    function startPolling() {
        poller.start();
    }

    function stopPolling() {
        poller.stop();
    }

    function isCurrent(runId) {
        return isCurrentMockRun(mockFlowModel, runId, state.mockTesting);
    }

    return {
        handleEvent,
        hydrateMockTargets,
        triggerMockFlows,
        onFocus,
        onBlur,
        isMockTesting,
        completeMockTarget
    };
}
