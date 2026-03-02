import { tryFetchJson } from '../../api/client.js';
import { formatBuildOverlayDetail, normalizePercent } from '../../utils/progress.js';
import {
    nextBuildCompletedState,
    nextBuildProgressState,
    nextBuildStartedState,
    parseHydratedBuildState
} from './build/reducer.js';
import { createPluginBuildOverlayController } from './build-overlay.js';

export function createBuildController({
    state,
    getContainer,
    getPluginById,
    onNeedsRender,
    onBuildComplete
}) {
    const buildOverlayController = createPluginBuildOverlayController({
        getContainer,
        getPluginById,
        getBuildState: plugin => getActivePluginBuildState(plugin, state.mockTesting),
        formatDetail: formatBuildOverlayDetail,
        normalizePercent
    });

    function handleEvent(event) {
        if (event.type === 'build_started') {
            Object.assign(state, nextBuildStartedState());
            clearQueuedBuildRowSync();
            onNeedsRender();
            return;
        }

        if (event.type === 'build_plugin_progress') {
            state.building = true;
            state.buildProgress = nextBuildProgressState(state.buildProgress, event);
            queueBuildRowSync(event.plugin_id);
            return;
        }

        if (event.type !== 'build_complete') {
            return;
        }

        clearQueuedBuildRowSync();
        Object.assign(state, nextBuildCompletedState(event.results));
        onBuildComplete();
    }

    function maybeRender(skipUpdate) {
        if (!skipUpdate && !state.linkingId) onNeedsRender();
    }

    async function hydrateBuildState(skipUpdate = false) {
        const payload = await tryFetchJson('/api/dev/build-state');
        if (!payload) {
            state.building = false;
            maybeRender(skipUpdate);
            return;
        }

        const nextState = parseHydratedBuildState(payload);
        state.building = nextState.building;

        if (!state.building) {
            state.buildProgress = nextState.buildProgress;
            if (nextState.buildResults) {
                state.buildResults = nextState.buildResults;
            }
            clearQueuedBuildRowSync();
            maybeRender(skipUpdate);
            return;
        }

        for (const [id, hydrated] of Object.entries(nextState.buildProgress)) {
            const live = state.buildProgress[id];
            if (live && (live.percent ?? 0) > (hydrated.percent ?? 0)) continue;
            state.buildProgress[id] = hydrated;
        }

        maybeRender(skipUpdate);
    }

    function getActivePluginBuildState(plugin, mockTesting) {
        if (!state.building) return null;
        if (!mockTesting && plugin.status !== 'linked') return null;

        const progress = state.buildProgress[plugin.id];
        if (!progress) return null;

        const status = progress.status || 'building';
        if (status !== 'queued' && status !== 'building') return null;
        if (!mockTesting && (!plugin.has_cargo || !plugin.needs_rebuild)) {
            return null;
        }

        const percent = normalizePercent(progress.percent, { round: true });
        const phase = (progress.phase || '').trim() || (status === 'queued' ? 'Queued' : 'Compiling');
        return { status, percent, phase };
    }

    function pruneInvisibleProgress(visibleIds) {
        for (const pluginId of Object.keys(state.buildProgress)) {
            if (!visibleIds.has(pluginId)) {
                delete state.buildProgress[pluginId];
            }
        }
    }

    function clearQueuedBuildRowSync() {
        buildOverlayController.clearQueued();
    }

    function queueBuildRowSync(pluginId) {
        buildOverlayController.queue(pluginId, onNeedsRender);
    }

    function cacheRows() {
        buildOverlayController.cacheRows();
    }

    function syncAll() {
        if (!state.building) {
            return;
        }
        buildOverlayController.syncAll(Object.keys(state.buildProgress), onNeedsRender);
    }

    function stopLocalMockBuildUi() {
        clearQueuedBuildRowSync();
        state.building = false;
        state.buildProgress = {};
        state.buildResults = null;
    }

    return {
        handleEvent,
        hydrateBuildState,
        getActivePluginBuildState,
        pruneInvisibleProgress,
        clearQueuedBuildRowSync,
        queueBuildRowSync,
        cacheRows,
        syncAll,
        stopLocalMockBuildUi
    };
}
