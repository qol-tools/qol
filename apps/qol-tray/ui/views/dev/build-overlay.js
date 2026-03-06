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

    const completion = createCompletionController({
        buildAnimation: BUILD_ANIMATION,
        rowRefs,
        normalizePercent,
        ensureOverlayNodes: ensureRowOverlayNodes,
        clearOverlayNodes,
        stopFillAnimation,
        applyFillScale,
        setOverlayCopy,
        finiteOr
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
                if (!syncRow(queuedId)) {
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
            stopFillAnimation(rowRef);
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
            syncRow(pluginId);
        }
    }

    function syncRow(pluginId) {
        if (!getContainer()) return false;
        const rowRef = rowRefs.get(pluginId);
        if (!rowRef) return false;
        const plugin = getPluginById(pluginId);
        if (!plugin) return false;

        const buildState = getBuildState(plugin);
        const isBuilding = !!buildState;
        rowRef.row.classList.toggle('is-building', isBuilding);

        if (!isBuilding) {
            if (completion.syncPlayback(rowRef)) return true;
            if (completion.startIfReady(rowRef)) return true;
            clearOverlayNodes(rowRef, stopFillAnimation);
            return true;
        }

        if (buildState.status === 'completed') {
            if (completion.syncPlayback(rowRef)) return true;
            if (!ensureRowOverlayNodes(rowRef)) return false;
            if (!completion.start(rowRef, true)) return false;
            return completion.syncPlayback(rowRef);
        }

        if (!ensureRowOverlayNodes(rowRef)) return false;

        const label = buildState.status === 'queued' ? 'Queued' : 'Compiling';
        const detail = formatDetail(buildState.phase, buildState.percent);
        const normalizedPercent = normalizePercent(buildState.percent);
        const cappedPercent = buildState.status === 'building'
            ? Math.min(normalizedPercent, BUILD_ANIMATION.completionTriggerPercent - 0.2)
            : normalizedPercent;

        completion.clear(rowRef.pluginId);
        rowRef.completing = false;
        rowRef.overlay.classList.remove('is-completing');
        resetStaleProgressState(rowRef, buildState.status, normalizedPercent);
        const displayPercent = toDisplayPercent(rowRef, cappedPercent, buildState.status);
        setFillTarget(rowRef, displayPercent, buildState.status !== 'building');
        setOverlayCopy(rowRef, label, detail);
        return true;
    }

    function ensureRowOverlayNodes(rowRef) {
        return ensureOverlayNodes(rowRef, percent => {
            applyFillScale(rowRef, percent, normalizePercent);
        });
    }

    function toDisplayPercent(rowRef, normalizedPercent, status) {
        if (status !== 'building') return normalizedPercent;
        if (!Number.isFinite(rowRef.lastBuildPercent)) return normalizedPercent;
        if (normalizedPercent >= rowRef.lastBuildPercent) return normalizedPercent;
        return rowRef.lastBuildPercent;
    }

    function resetStaleProgressState(rowRef, status, normalizedPercent) {
        if (status !== 'queued' && status !== 'building') return;
        if (normalizedPercent > BUILD_ANIMATION.staleResetPercent) return;
        if (!Number.isFinite(rowRef.lastBuildPercent) && !Number.isFinite(rowRef.displayPercent)) {
            return;
        }
        rowRef.displayPercent = Number.NaN;
        rowRef.targetPercent = Number.NaN;
        rowRef.lastBuildPercent = Number.NaN;
        rowRef.lastFrameTime = 0;
        stopFillAnimation(rowRef);
    }

    function setFillTarget(rowRef, targetPercent, immediate) {
        const nextPercent = normalizePercent(targetPercent);
        rowRef.lastBuildPercent = nextPercent;
        if (!rowRef.fill || rowRef.completing) return;

        if (immediate) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent, normalizePercent);
            stopFillAnimation(rowRef);
            return;
        }

        if (!Number.isFinite(rowRef.displayPercent)) {
            rowRef.displayPercent = 0;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, 0, normalizePercent);
            rowRef.lastFrameTime = performance.now();
            queueFillAnimation(rowRef);
            return;
        }

        const delta = Math.abs(nextPercent - rowRef.displayPercent);
        if (delta <= 0.01) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent, normalizePercent);
            stopFillAnimation(rowRef);
            return;
        }

        rowRef.targetPercent = nextPercent;
        rowRef.lastFrameTime = performance.now();
        queueFillAnimation(rowRef);
    }

    function queueFillAnimation(rowRef) {
        if (rowRef.animationFrame !== null) return;
        rowRef.animationFrame = requestAnimationFrame(timestamp => animateFill(rowRef, timestamp));
    }

    function animateFill(rowRef, timestamp) {
        rowRef.animationFrame = null;
        if (!rowRef.fill) return;

        const current = finiteOr(rowRef.displayPercent, 0);
        const target = finiteOr(rowRef.targetPercent, current);
        const delta = target - current;
        const allowHardSync = target < (BUILD_ANIMATION.completionTriggerPercent - 8);
        if (allowHardSync && delta > BUILD_ANIMATION.hardSyncDelta) {
            syncFill(rowRef, target, timestamp);
            return;
        }
        if (Math.abs(delta) <= BUILD_ANIMATION.snapDelta) {
            syncFill(rowRef, target, timestamp);
            return;
        }

        const elapsed = rowRef.lastFrameTime > 0 ? timestamp - rowRef.lastFrameTime : 16;
        const dt = Math.min(BUILD_ANIMATION.frameMaxMs, Math.max(BUILD_ANIMATION.frameMinMs, elapsed));
        rowRef.lastFrameTime = timestamp;
        const alpha = 1 - Math.exp(-dt / BUILD_ANIMATION.easeMs);
        const eased = current + delta * alpha;
        const next = Math.max(current, eased);

        rowRef.displayPercent = next;
        applyFillScale(rowRef, next, normalizePercent);
        rowRef.animationFrame = requestAnimationFrame(nextTimestamp => animateFill(rowRef, nextTimestamp));
    }

    function syncFill(rowRef, percent, timestamp) {
        rowRef.displayPercent = percent;
        applyFillScale(rowRef, percent, normalizePercent);
        rowRef.lastFrameTime = timestamp;
    }

    function stopFillAnimation(rowRef) {
        if (rowRef.animationFrame !== null) {
            cancelAnimationFrame(rowRef.animationFrame);
            rowRef.animationFrame = null;
        }
        rowRef.lastFrameTime = 0;
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
