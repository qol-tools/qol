import { formatBuildOverlayDetail, normalizePercent } from '../../utils/progress.js';
import { dispatchBuildEvent, completeLocalBuild } from './build/event-handler.js';
import { hydrateBuildState } from './build/hydration.js';
import { getActivePluginBuildState, pruneInvisibleProgress } from './build/active-state.js';
import { resetBuildState } from './build/reducer.js';
import { createPluginBuildOverlayController } from './build-overlay.js';

function createOverlay(state, getContainer, getPluginById) {
    return createPluginBuildOverlayController({
        getContainer,
        getPluginById,
        getBuildState: plugin => getActivePluginBuildState(state, plugin, state.mockTesting),
        formatDetail: formatBuildOverlayDetail,
        normalizePercent
    });
}

export function createBuildController({ state, getContainer, getPluginById, onNeedsRender, onBuildComplete }) {
    const overlay = createOverlay(state, getContainer, getPluginById);
    const clearSync = () => overlay.clearQueued();
    const maybeRender = () => { if (!state.linkingId) onNeedsRender(); };
    const queueSync = (pluginId) => overlay.queue(pluginId, onNeedsRender);
    return {
        handleEvent: (e) => dispatchBuildEvent(e, state, overlay, clearSync, queueSync, onNeedsRender, onBuildComplete),
        completeLocalBuild: (results) => completeLocalBuild(state, overlay, clearSync, maybeRender, results),
        hydrateBuildState: (skip) => hydrateBuildState(state, clearSync, skip ? () => {} : maybeRender),
        getActivePluginBuildState: (plugin, mock) => getActivePluginBuildState(state, plugin, mock),
        pruneInvisibleProgress: (ids) => pruneInvisibleProgress(state, ids),
        clearQueuedBuildRowSync: clearSync,
        queueBuildRowSync: queueSync,
        cacheRows: () => overlay.cacheRows(),
        syncAll: () => { if (state.building) overlay.syncAll(Object.keys(state.buildProgress)); },
        stopLocalMockBuildUi: () => { clearSync(); resetBuildState(state); }
    };
}
