import { BUILD_ANIMATION } from './build-animation.js';
import {
    applyFillScale,
    cacheRowRefs,
    clearOverlayNodes,
    ensureOverlayNodes,
    finiteOr,
    setOverlayCopy
} from './build-overlay/dom.js';
import { createCompletionController } from './build-overlay/completion.js';
import { createFillController } from './build-overlay/fill.js';
import { createRowSync } from './build-overlay/sync.js';

export function createPluginBuildOverlayController({
    getContainer,
    getPluginById,
    getBuildState,
    formatDetail,
    normalizePercent
}) {
    const LOG_PREFIX = '[qol-dev-loading]';
    const DEBUG_LOADING = false;
    const rowRefs = new Map();
    const pendingBuildRows = new Set();
    let buildSyncFrame = null;
    const fill = createFillController({
        buildAnimation: BUILD_ANIMATION,
        normalizePercent,
        applyFillScale,
        finiteOr
    });

    const completion = createCompletionController({
        buildAnimation: BUILD_ANIMATION,
        rowRefs,
        normalizePercent,
        ensureOverlayNodes: ensureRowOverlayNodes,
        clearOverlayNodes,
        stopFillAnimation: fill.stopFillAnimation,
        applyFillScale,
        setOverlayCopy,
        finiteOr
    });
    const rowSync = createRowSync({
        buildAnimation: BUILD_ANIMATION,
        rowRefs,
        getContainer,
        getPluginById,
        getBuildState,
        formatDetail,
        normalizePercent,
        ensureRowOverlayNodes,
        clearOverlayNodes,
        setOverlayCopy,
        completion,
        fill
    });

    function log(event, payload) {
        if (!DEBUG_LOADING) return;
        console.info(`${LOG_PREFIX} ${event}`, payload);
    }

    function clearQueued() {
        pendingBuildRows.clear();
        cancelBuildSync();
        completion.clearAll();
    }

    function completeRows(pluginIds, onComplete) {
        return completion.completeRows(pluginIds, onComplete);
    }

    function queue(pluginId, onNeedsFullRender) {
        if (!pluginId) return;
        log('overlay:queue', { pluginId });
        pendingBuildRows.add(pluginId);
        if (buildSyncFrame !== null) return;

        buildSyncFrame = requestAnimationFrame(() => {
            buildSyncFrame = null;
            let needsFullRender = false;
            for (const queuedId of pendingBuildRows) {
                if (!rowSync.syncRow(queuedId)) {
                    needsFullRender = true;
                    break;
                }
            }
            pendingBuildRows.clear();
            if (!needsFullRender) return;
            if (typeof onNeedsFullRender === 'function') {
                onNeedsFullRender();
            }
        });
    }

    function cacheRows() {
        const previousRows = new Map(rowRefs);
        cancelBuildSync();
        for (const rowRef of previousRows.values()) {
            fill.stopFillAnimation(rowRef);
        }

        rowRefs.clear();
        const nextRows = cacheRowRefs(getContainer, previousRows, completion.snapshot);
        for (const [pluginId, rowRef] of nextRows.entries()) {
            rowRefs.set(pluginId, rowRef);
        }

        log('overlay:cache-rows', {
            mappedCount: rowRefs.size,
            pluginIds: Array.from(rowRefs.keys())
        });
    }

    function syncAll(pluginIds) {
        for (const pluginId of pluginIds) {
            rowSync.syncRow(pluginId);
        }
    }

    function ensureRowOverlayNodes(rowRef) {
        return ensureOverlayNodes(rowRef, percent => {
            applyFillScale(rowRef, percent, normalizePercent);
        });
    }

    function cancelBuildSync() {
        if (buildSyncFrame === null) return;
        cancelAnimationFrame(buildSyncFrame);
        buildSyncFrame = null;
    }

    return {
        clearQueued,
        completeRows,
        queue,
        cacheRows,
        syncAll
    };
}
