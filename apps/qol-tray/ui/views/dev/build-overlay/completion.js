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
    const completionByPlugin = new Map();
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
        completionByPlugin.clear();
    }

    function clear(pluginId) {
        setState(pluginId, 'idle');
    }

    function completeRows(pluginIds, onComplete) {
        const done = typeof onComplete === 'function' ? onComplete : () => {};
        let started = 0;
        let longestRemainingMs = 0;
        const now = performance.now();

        for (const pluginId of pluginIds) {
            const rowRef = rowRefs.get(pluginId);
            const completionState = getState(pluginId);
            if (completionState === 'done') continue;
            if (completionState === 'playing') {
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
        const completionState = getState(rowRef.pluginId);
        if (completionState === 'done') {
            clearOverlayNodes(rowRef, stopFillAnimation);
            return true;
        }
        if (completionState === 'playing') {
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
        setState(rowRef.pluginId, 'playing', {
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
        const completion = completionByPlugin.get(rowRef.pluginId);
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
        const completion = completionByPlugin.get(pluginId);
        if (!completion || completion.state !== 'playing') return null;

        const phase = completion.phase || 'ramp';
        const phaseStartedAt = finiteOr(completion.phaseStartedAt, now);
        const phaseElapsed = Math.max(0, now - phaseStartedAt);
        if (phase === 'fade') {
            return { percent: 100, completing: true };
        }
        if (phase === 'hold') {
            return { percent: 100, completing: false };
        }

        const rampMs = buildAnimation.completionRampMs;
        const startPercent = normalizePercent(completion.startPercent);
        const t = rampMs <= 0 ? 1 : Math.max(0, Math.min(1, phaseElapsed / rampMs));
        const eased = easeOutCubic(t);
        return {
            percent: startPercent + (100 - startPercent) * eased,
            completing: false
        };
    }

    function getState(pluginId) {
        if (!pluginId) return 'idle';
        const completion = completionByPlugin.get(pluginId);
        if (!completion) return 'idle';
        return completion.state;
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

        for (const [pluginId, completion] of completionByPlugin.entries()) {
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
        const phase = completion.phase || 'ramp';
        if (phase === 'ramp') {
            return renderRamp(rowRef, completion, timestamp);
        }
        if (phase === 'hold') {
            return renderHold(rowRef, completion, timestamp);
        }
        return renderFade(rowRef, completion, timestamp);
    }

    function renderRamp(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        const rampMs = buildAnimation.completionRampMs;
        if (phaseElapsed < rampMs) {
            const progressT = rampMs <= 0 ? 1 : phaseElapsed / rampMs;
            const eased = easeOutCubic(progressT);
            const startPercent = normalizePercent(completion.startPercent);
            const nextPercent = startPercent + (100 - startPercent) * eased;
            applyProgress(rowRef, nextPercent, false);
            return false;
        }
        completion.phase = 'hold';
        completion.phaseStartedAt = timestamp;
        applyProgress(rowRef, 100, false);
        return false;
    }

    function renderHold(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        applyProgress(rowRef, 100, false);
        if (phaseElapsed < buildAnimation.completionHoldMs) return false;
        completion.phase = 'fade';
        completion.phaseStartedAt = timestamp;
        applyProgress(rowRef, 100, true);
        return false;
    }

    function renderFade(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        applyProgress(rowRef, 100, true);
        return phaseElapsed >= buildAnimation.completionVisibleMs;
    }

    function applyProgress(rowRef, percent, completing) {
        rowRef.completing = completing;
        if (rowRef.overlay) {
            rowRef.overlay.classList.toggle('is-completing', completing);
        }
        rowRef.displayPercent = percent;
        rowRef.targetPercent = 100;
        rowRef.lastBuildPercent = 100;
        applyFillScale(rowRef, percent, normalizePercent);
    }

    function remainingMs(pluginId, now = performance.now()) {
        const completion = completionByPlugin.get(pluginId);
        if (!completion || completion.state !== 'playing') return 0;

        const phase = completion.phase || 'ramp';
        const phaseStartedAt = finiteOr(completion.phaseStartedAt, now);
        const phaseElapsed = Math.max(0, now - phaseStartedAt);
        if (phase === 'ramp') {
            const rampRemaining = Math.max(0, buildAnimation.completionRampMs - phaseElapsed);
            return rampRemaining + buildAnimation.completionHoldMs + buildAnimation.completionVisibleMs;
        }
        if (phase === 'hold') {
            const holdRemaining = Math.max(0, buildAnimation.completionHoldMs - phaseElapsed);
            return holdRemaining + buildAnimation.completionVisibleMs;
        }
        return Math.max(0, buildAnimation.completionVisibleMs - phaseElapsed);
    }

    function finalize(pluginId) {
        setState(pluginId, 'done');
    }

    function setCompletedCopy(rowRef) {
        setOverlayCopy(rowRef, 'Completed', 'Reloading plugin');
    }

    function setState(pluginId, state, patch = {}) {
        if (!pluginId) return;
        if (state === 'idle') {
            completionByPlugin.delete(pluginId);
            return;
        }
        const previous = completionByPlugin.get(pluginId) || {};
        completionByPlugin.set(pluginId, {
            state,
            startedAt: finiteOr(patch.startedAt, finiteOr(previous.startedAt, 0)),
            startPercent: finiteOr(patch.startPercent, finiteOr(previous.startPercent, 100)),
            phase: typeof patch.phase === 'string' ? patch.phase : (previous.phase || 'ramp'),
            phaseStartedAt: finiteOr(
                patch.phaseStartedAt,
                finiteOr(previous.phaseStartedAt, finiteOr(patch.startedAt, finiteOr(previous.startedAt, 0)))
            )
        });
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

function easeOutCubic(value) {
    const clamped = Math.max(0, Math.min(1, value));
    const inv = 1 - clamped;
    return 1 - inv * inv * inv;
}
