import { createCompletionPhaseRenderer } from './completion/phases.js';
import { createCompletionStore } from './completion/store.js';

export function createCompletionController({
    buildAnimation,
    rowRefs,
    normalizePercent,
    ensureOverlayNodes,
    clearOverlayNodes,
    stopFillAnimation,
    applyFillScale,
    setOverlayCopy,
    finiteOr
}) {
    const phases = createCompletionPhaseRenderer({
        buildAnimation,
        normalizePercent,
        applyFillScale,
        finiteOr
    });
    const completionState = createCompletionStore({
        createSnapshot: phases.snapshot,
        computeRemainingMs: phases.remainingMs,
        finiteOr
    });
    let completionFrame = null;

    function clearAll() {
        cancelFrame();
        for (const rowRef of rowRefs.values()) {
            stopFillAnimation(rowRef);
            rowRef.completing = false;
            if (rowRef.overlay) {
                rowRef.overlay.classList.remove('is-completing');
            }
        }
        completionState.clearAll();
    }

    function clear(pluginId) {
        completionState.clear(pluginId);
    }

    function completeRows(pluginIds, onComplete) {
        const done = typeof onComplete === 'function' ? onComplete : () => {};
        let started = 0;
        let longestRemainingMs = 0;
        const now = performance.now();

        for (const pluginId of pluginIds) {
            const rowRef = rowRefs.get(pluginId);
            const completionStatus = getState(pluginId);
            if (completionStatus === 'done') continue;
            if (completionStatus === 'playing') {
                started += 1;
                longestRemainingMs = Math.max(longestRemainingMs, remainingMs(pluginId, now));
                continue;
            }
            if (!rowRef) continue;
            if (!ensureOverlayNodes(rowRef)) continue;
            if (!start(rowRef, true, now)) continue;
            started += 1;
            longestRemainingMs = Math.max(longestRemainingMs, remainingMs(pluginId, now));
        }

        if (started === 0) {
            done();
            return false;
        }

        setTimeout(done, longestRemainingMs + 20);
        return true;
    }

    function startIfReady(rowRef) {
        const completionStatus = getState(rowRef.pluginId);
        if (completionStatus === 'done') {
            clearOverlayNodes(rowRef, stopFillAnimation);
            return true;
        }
        if (completionStatus === 'playing') {
            return syncPlayback(rowRef);
        }
        if (!ensureOverlayNodes(rowRef)) return false;
        if (!start(rowRef)) return false;
        return syncPlayback(rowRef);
    }

    function start(rowRef, force = false, now = performance.now()) {
        if (!rowRef.overlay || !rowRef.fill) return false;
        if (getState(rowRef.pluginId) === 'playing') return true;
        const visiblePercent = finiteOr(rowRef.displayPercent, Number.NaN);
        const fallbackPercent = finiteOr(rowRef.lastBuildPercent, 0);
        const completedPercent = Number.isFinite(visiblePercent) ? visiblePercent : fallbackPercent;
        if (!force && completedPercent < buildAnimation.completionTriggerPercent) return false;

        const startPercent = normalizePercent(completedPercent);
        completionState.setState(rowRef.pluginId, 'playing', {
            startedAt: now,
            startPercent,
            phase: 'ramp',
            phaseStartedAt: now
        });
        rowRef.completing = false;
        stopFillAnimation(rowRef);
        rowRef.displayPercent = startPercent;
        rowRef.targetPercent = 100;
        rowRef.lastBuildPercent = 100;
        rowRef.fill.style.removeProperty('--progress-transition-override');
        applyFillScale(rowRef, startPercent, normalizePercent);
        rowRef.overlay.classList.remove('is-completing');
        setCompletedCopy(rowRef);
        ensureFrame();
        return true;
    }

    function syncPlayback(rowRef) {
        const completion = completionState.get(rowRef.pluginId);
        if (!completion) return false;
        if (completion.state === 'done') {
            clearOverlayNodes(rowRef, stopFillAnimation);
            return true;
        }
        if (!ensureOverlayNodes(rowRef)) return false;
        setCompletedCopy(rowRef);
        if (!renderFrame(rowRef, completion, performance.now())) {
            ensureFrame();
            return true;
        }
        finalize(rowRef.pluginId);
        clearOverlayNodes(rowRef, stopFillAnimation);
        return true;
    }

    function snapshot(pluginId, now) {
        return completionState.snapshot(pluginId, now);
    }

    function getState(pluginId) {
        return completionState.getState(pluginId);
    }

    function ensureFrame() {
        if (completionFrame !== null) return;
        completionFrame = requestAnimationFrame(timestamp => tick(timestamp));
    }

    function cancelFrame() {
        if (completionFrame === null) return;
        cancelAnimationFrame(completionFrame);
        completionFrame = null;
    }

    function tick(timestamp) {
        completionFrame = null;
        let hasActiveCompletion = false;

        for (const [pluginId, completion] of completionState.entries()) {
            if (completion.state !== 'playing') continue;
            if (remainingMs(pluginId, timestamp) <= 0) {
                finalize(pluginId);
                const rowRef = rowRefs.get(pluginId);
                if (rowRef) {
                    clearOverlayNodes(rowRef, stopFillAnimation);
                }
                continue;
            }
            hasActiveCompletion = true;
            const rowRef = rowRefs.get(pluginId);
            if (!rowRef) continue;
            if (!ensureOverlayNodes(rowRef)) continue;
            setCompletedCopy(rowRef);
            renderFrame(rowRef, completion, timestamp);
        }

        if (!hasActiveCompletion) return;
        completionFrame = requestAnimationFrame(next => tick(next));
    }

    function renderFrame(rowRef, completion, timestamp) {
        return phases.renderFrame(rowRef, completion, timestamp);
    }

    function remainingMs(pluginId, now = performance.now()) {
        return completionState.remainingMs(pluginId, now);
    }

    function finalize(pluginId) {
        completionState.finalize(pluginId);
    }

    function setCompletedCopy(rowRef) {
        setOverlayCopy(rowRef, 'Completed', 'Reloading plugin');
    }

    return {
        clear,
        clearAll,
        completeRows,
        getState,
        snapshot,
        start,
        startIfReady,
        syncPlayback
    };
}
