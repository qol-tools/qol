import { BUILD_ANIMATION } from './build-animation.js';

export function createPluginBuildOverlayController({
    getContainer,
    getPluginById,
    getBuildState,
    formatDetail,
    normalizePercent
}) {
    const LOG_PREFIX = '[qol-dev-loading]';
    const DEBUG_LOADING = false;
    let rowRefs = new Map();
    const completionByPlugin = new Map();
    const pendingBuildRows = new Set();
    let buildSyncFrame = null;
    let completionFrame = null;

    function log(event, payload) {
        if (!DEBUG_LOADING) return;
        console.info(`${LOG_PREFIX} ${event}`, payload);
    }

    function clearQueued() {
        pendingBuildRows.clear();
        if (buildSyncFrame !== null) {
            cancelAnimationFrame(buildSyncFrame);
            buildSyncFrame = null;
        }
        cancelCompletionFrame();
        for (const rowRef of rowRefs.values()) {
            stopFillAnimation(rowRef);
            rowRef.completing = false;
            if (rowRef.overlay) {
                rowRef.overlay.classList.remove('is-completing');
            }
        }
        completionByPlugin.clear();
    }

    function completeRows(pluginIds, onComplete) {
        const done = typeof onComplete === 'function' ? onComplete : () => {};
        let started = 0;
        let longestRemainingMs = 0;
        const now = performance.now();

        for (const pluginId of pluginIds) {
            const rowRef = rowRefs.get(pluginId);
            const completionState = getCompletionState(pluginId);
            if (completionState === 'done') continue;
            if (completionState === 'playing') {
                started += 1;
                longestRemainingMs = Math.max(longestRemainingMs, completionRemainingMs(pluginId, now));
                continue;
            }
            if (!rowRef) continue;
            if (!ensureOverlayNodes(rowRef)) continue;
            if (!startCompletion(rowRef, true, now)) continue;
            started += 1;
            longestRemainingMs = Math.max(longestRemainingMs, completionRemainingMs(pluginId, now));
        }

        if (started === 0) {
            done();
            return false;
        }

        setTimeout(done, longestRemainingMs + 20);
        return true;
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
            if (needsFullRender && typeof onNeedsFullRender === 'function') {
                onNeedsFullRender();
            }
        });
    }

    function cacheRows() {
        const previousRows = rowRefs;
        if (buildSyncFrame !== null) {
            cancelAnimationFrame(buildSyncFrame);
            buildSyncFrame = null;
        }
        for (const rowRef of previousRows.values()) {
            stopFillAnimation(rowRef);
        }
        rowRefs = new Map();
        const container = getContainer();
        if (!container) return;

        const rows = container.querySelectorAll('.plugin-row[data-plugin-id]');
        for (const row of rows) {
            const pluginId = row.dataset.pluginId;
            if (!pluginId) continue;
            const previous = previousRows.get(pluginId);
            rowRefs.set(pluginId, makeRowRef(row, previous));
        }
        log('overlay:cache-rows', {
            rowCount: rows.length,
            mappedCount: rowRefs.size,
            pluginIds: Array.from(rowRefs.keys())
        });
    }

    function makeRowRef(row, previous) {
        const pluginId = row.dataset.pluginId || '';
        const snapshot = getCompletionSnapshot(pluginId, performance.now());
        return {
            row,
            pluginId,
            overlayHost: row.querySelector('.plugin-build-overlay-host'),
            overlay: null,
            fill: null,
            main: null,
            sub: null,
            displayPercent: finiteOr(previous?.displayPercent, finiteOr(snapshot?.percent, Number.NaN)),
            targetPercent: finiteOr(previous?.targetPercent, finiteOr(snapshot?.percent, Number.NaN)),
            lastBuildPercent: finiteOr(previous?.lastBuildPercent, finiteOr(snapshot?.percent, Number.NaN)),
            animationFrame: null,
            completing: previous?.completing === true || snapshot?.completing === true,
            lastFrameTime: 0,
            lastMain: '',
            lastSub: ''
        };
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
            if (syncCompletionPlayback(rowRef)) return true;
            if (startCompletionIfReady(rowRef)) return true;
            clearOverlayNodes(rowRef);
            return true;
        }

        if (buildState.status === 'completed') {
            if (syncCompletionPlayback(rowRef)) return true;
            if (!ensureOverlayNodes(rowRef)) return false;
            if (!startCompletion(rowRef, true)) return false;
            return syncCompletionPlayback(rowRef);
        }

        if (!ensureOverlayNodes(rowRef)) return false;

        const label = buildState.status === 'queued' ? 'Queued' : 'Compiling';
        const detail = formatDetail(buildState.phase, buildState.percent);
        const normalizedPercent = normalizePercent(buildState.percent);
        const cappedPercent = buildState.status === 'building'
            ? Math.min(normalizedPercent, BUILD_ANIMATION.completionTriggerPercent - 0.2)
            : normalizedPercent;
        setCompletionState(rowRef.pluginId, 'idle');
        rowRef.completing = false;
        rowRef.overlay.classList.remove('is-completing');
        resetStaleProgressState(rowRef, buildState.status, normalizedPercent);
        const displayPercent = toDisplayPercent(rowRef, cappedPercent, buildState.status);
        setFillTarget(rowRef, displayPercent, buildState.status !== 'building');
        if (rowRef.lastMain !== label && rowRef.main) {
            rowRef.main.textContent = label;
            rowRef.lastMain = label;
        }
        if (rowRef.lastSub !== detail && rowRef.sub) {
            rowRef.sub.textContent = detail;
            rowRef.lastSub = detail;
        }

        return true;
    }

    function ensureOverlayNodes(rowRef) {
        if (!rowRef.overlayHost) return false;
        if (rowRef.overlay && rowRef.overlay.isConnected) return true;

        const hadAnimationState = Number.isFinite(rowRef.displayPercent);

        const overlay = document.createElement('div');
        overlay.className = 'plugin-build-overlay progress-track is-downloading compiling';
        overlay.setAttribute('aria-hidden', 'true');

        const fill = document.createElement('div');
        fill.className = 'progress-fill';
        overlay.appendChild(fill);

        const copy = document.createElement('div');
        copy.className = 'plugin-build-overlay-copy';

        const main = document.createElement('span');
        main.className = 'plugin-build-overlay-main';
        copy.appendChild(main);

        const sub = document.createElement('span');
        sub.className = 'plugin-build-overlay-sub';
        copy.appendChild(sub);

        overlay.appendChild(copy);
        rowRef.overlayHost.replaceChildren(overlay);

        rowRef.overlay = overlay;
        rowRef.fill = fill;
        rowRef.main = main;
        rowRef.sub = sub;

        if (hadAnimationState) {
            applyFillScale(rowRef, rowRef.displayPercent);
            return true;
        }

        rowRef.lastFrameTime = 0;
        rowRef.lastMain = '';
        rowRef.lastSub = '';
        return true;
    }

    function clearOverlayNodes(rowRef) {
        stopFillAnimation(rowRef);
        if (rowRef.fill) {
            rowRef.fill.style.removeProperty('--progress-transition-override');
        }
        if (rowRef.overlayHost && rowRef.overlayHost.childElementCount > 0) {
            rowRef.overlayHost.replaceChildren();
        }
        rowRef.overlay = null;
        rowRef.fill = null;
        rowRef.main = null;
        rowRef.sub = null;
        rowRef.completing = false;
        rowRef.displayPercent = Number.NaN;
        rowRef.targetPercent = Number.NaN;
        rowRef.lastBuildPercent = Number.NaN;
        rowRef.lastFrameTime = 0;
        rowRef.lastMain = '';
        rowRef.lastSub = '';
    }

    function startCompletionIfReady(rowRef) {
        const completionState = getCompletionState(rowRef.pluginId);
        if (completionState === 'done') {
            clearOverlayNodes(rowRef);
            return true;
        }
        if (completionState === 'playing') {
            return syncCompletionPlayback(rowRef);
        }
        if (!ensureOverlayNodes(rowRef)) return false;
        if (!startCompletion(rowRef)) return false;
        return syncCompletionPlayback(rowRef);
    }

    function startCompletion(rowRef, force = false, now = performance.now()) {
        if (!rowRef.overlay || !rowRef.fill) return false;
        if (getCompletionState(rowRef.pluginId) === 'playing') return true;
        const visiblePercent = finiteOr(rowRef.displayPercent, Number.NaN);
        const fallbackPercent = finiteOr(rowRef.lastBuildPercent, 0);
        const completedPercent = Number.isFinite(visiblePercent) ? visiblePercent : fallbackPercent;
        if (!force && completedPercent < BUILD_ANIMATION.completionTriggerPercent) return false;
        const startPercent = normalizePercent(completedPercent);
        setCompletionState(rowRef.pluginId, 'playing', {
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
        applyFillScale(rowRef, startPercent);
        rowRef.overlay.classList.remove('is-completing');
        setCompletedCopy(rowRef);
        ensureCompletionFrame();
        return true;
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
        if (!Number.isFinite(rowRef.lastBuildPercent) && !Number.isFinite(rowRef.displayPercent)) return;
        rowRef.displayPercent = Number.NaN;
        rowRef.targetPercent = Number.NaN;
        rowRef.lastBuildPercent = Number.NaN;
        rowRef.lastFrameTime = 0;
        stopFillAnimation(rowRef);
    }

    function setFillTarget(rowRef, targetPercent, immediate) {
        const nextPercent = normalizePercent(targetPercent);
        rowRef.lastBuildPercent = nextPercent;
        if (!rowRef.fill) return;
        if (rowRef.completing) return;

        if (immediate) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent);
            stopFillAnimation(rowRef);
            return;
        }

        if (!Number.isFinite(rowRef.displayPercent)) {
            rowRef.displayPercent = 0;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, 0);
            rowRef.lastFrameTime = performance.now();
            if (rowRef.animationFrame !== null) return;
            rowRef.animationFrame = requestAnimationFrame(ts => animateFill(rowRef, ts));
            return;
        }

        const delta = Math.abs(nextPercent - rowRef.displayPercent);
        if (delta <= 0.01) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent);
            stopFillAnimation(rowRef);
            return;
        }

        rowRef.targetPercent = nextPercent;
        rowRef.lastFrameTime = performance.now();
        if (rowRef.animationFrame !== null) return;
        rowRef.animationFrame = requestAnimationFrame(ts => animateFill(rowRef, ts));
    }

    function animateFill(rowRef, timestamp) {
        rowRef.animationFrame = null;
        if (!rowRef.fill) return;

        const current = finiteOr(rowRef.displayPercent, 0);
        const target = finiteOr(rowRef.targetPercent, current);
        const delta = target - current;
        const allowHardSync = target < (BUILD_ANIMATION.completionTriggerPercent - 8);
        if (allowHardSync && delta > BUILD_ANIMATION.hardSyncDelta) {
            rowRef.displayPercent = target;
            applyFillScale(rowRef, target);
            rowRef.lastFrameTime = timestamp;
            return;
        }

        if (Math.abs(delta) <= BUILD_ANIMATION.snapDelta) {
            rowRef.displayPercent = target;
            applyFillScale(rowRef, target);
            rowRef.lastFrameTime = timestamp;
            return;
        }

        const elapsed = rowRef.lastFrameTime > 0 ? timestamp - rowRef.lastFrameTime : 16;
        const dt = Math.min(BUILD_ANIMATION.frameMaxMs, Math.max(BUILD_ANIMATION.frameMinMs, elapsed));
        rowRef.lastFrameTime = timestamp;
        const alpha = 1 - Math.exp(-dt / BUILD_ANIMATION.easeMs);
        const eased = current + delta * alpha;
        const next = Math.max(current, eased);

        rowRef.displayPercent = next;
        applyFillScale(rowRef, next);
        rowRef.animationFrame = requestAnimationFrame(ts => animateFill(rowRef, ts));
    }

    function ensureCompletionFrame() {
        if (completionFrame !== null) return;
        completionFrame = requestAnimationFrame(timestamp => tickCompletionPlayback(timestamp));
    }

    function cancelCompletionFrame() {
        if (completionFrame === null) return;
        cancelAnimationFrame(completionFrame);
        completionFrame = null;
    }

    function tickCompletionPlayback(timestamp) {
        completionFrame = null;
        let hasActiveCompletion = false;

        for (const [pluginId, completion] of completionByPlugin.entries()) {
            if (completion.state !== 'playing') continue;
            const remainingMs = completionRemainingMs(pluginId, timestamp);
            if (remainingMs <= 0) {
                finalizeCompletion(pluginId);
                const rowRef = rowRefs.get(pluginId);
                if (rowRef) {
                    clearOverlayNodes(rowRef);
                }
                continue;
            }
            hasActiveCompletion = true;
            const rowRef = rowRefs.get(pluginId);
            if (!rowRef) continue;
            if (!ensureOverlayNodes(rowRef)) continue;
            setCompletedCopy(rowRef);
            renderCompletionFrame(rowRef, completion, timestamp);
        }

        if (!hasActiveCompletion) return;
        completionFrame = requestAnimationFrame(ts => tickCompletionPlayback(ts));
    }

    function syncCompletionPlayback(rowRef) {
        const completion = completionByPlugin.get(rowRef.pluginId);
        if (!completion) return false;
        if (completion.state === 'done') {
            clearOverlayNodes(rowRef);
            return true;
        }
        if (!ensureOverlayNodes(rowRef)) return false;
        setCompletedCopy(rowRef);
        if (!renderCompletionFrame(rowRef, completion, performance.now())) {
            ensureCompletionFrame();
            return true;
        }
        finalizeCompletion(rowRef.pluginId);
        clearOverlayNodes(rowRef);
        return true;
    }

    function renderCompletionFrame(rowRef, completion, timestamp) {
        const phase = completion.phase || 'ramp';
        if (phase === 'ramp') {
            return renderCompletionRamp(rowRef, completion, timestamp);
        }
        if (phase === 'hold') {
            return renderCompletionHold(rowRef, completion, timestamp);
        }
        return renderCompletionFade(rowRef, completion, timestamp);
    }

    function renderCompletionRamp(rowRef, completion, timestamp) {
        const rampMs = BUILD_ANIMATION.completionRampMs;
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        const startPercent = normalizePercent(completion.startPercent);
        if (phaseElapsed < rampMs) {
            const progressT = rampMs <= 0 ? 1 : phaseElapsed / rampMs;
            const eased = easeOutCubic(progressT);
            const nextPercent = startPercent + (100 - startPercent) * eased;
            applyCompletionProgress(rowRef, nextPercent, false);
            return false;
        }
        completion.phase = 'hold';
        completion.phaseStartedAt = timestamp;
        applyCompletionProgress(rowRef, 100, false);
        return false;
    }

    function renderCompletionHold(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        applyCompletionProgress(rowRef, 100, false);
        if (phaseElapsed < BUILD_ANIMATION.completionHoldMs) return false;
        completion.phase = 'fade';
        completion.phaseStartedAt = timestamp;
        applyCompletionProgress(rowRef, 100, true);
        return false;
    }

    function renderCompletionFade(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        applyCompletionProgress(rowRef, 100, true);
        if (phaseElapsed < BUILD_ANIMATION.completionVisibleMs) return false;
        return true;
    }

    function applyCompletionProgress(rowRef, percent, completing) {
        rowRef.completing = completing;
        if (rowRef.overlay) {
            rowRef.overlay.classList.toggle('is-completing', completing);
        }
        rowRef.displayPercent = percent;
        rowRef.targetPercent = 100;
        rowRef.lastBuildPercent = 100;
        applyFillScale(rowRef, percent);
    }

    function setCompletedCopy(rowRef) {
        if (rowRef.main) {
            rowRef.main.textContent = 'Completed';
            rowRef.lastMain = 'Completed';
        }
        if (rowRef.sub) {
            rowRef.sub.textContent = 'Reloading plugin';
            rowRef.lastSub = 'Reloading plugin';
        }
    }

    function getCompletionSnapshot(pluginId, now) {
        const completion = completionByPlugin.get(pluginId);
        if (!completion) return null;
        if (completion.state !== 'playing') return null;
        const phase = completion.phase || 'ramp';
        const phaseStartedAt = finiteOr(completion.phaseStartedAt, now);
        const phaseElapsed = Math.max(0, now - phaseStartedAt);
        if (phase === 'fade') {
            return { percent: 100, completing: true };
        }
        if (phase === 'hold') {
            return { percent: 100, completing: false };
        }
        const rampMs = BUILD_ANIMATION.completionRampMs;
        const startPercent = normalizePercent(completion.startPercent);
        const t = rampMs <= 0 ? 1 : Math.max(0, Math.min(1, phaseElapsed / rampMs));
        const eased = easeOutCubic(t);
        return {
            percent: startPercent + (100 - startPercent) * eased,
            completing: false
        };
    }

    function completionRemainingMs(pluginId, now = performance.now()) {
        const completion = completionByPlugin.get(pluginId);
        if (!completion) return 0;
        if (completion.state !== 'playing') return 0;
        const phase = completion.phase || 'ramp';
        const phaseStartedAt = finiteOr(completion.phaseStartedAt, now);
        const phaseElapsed = Math.max(0, now - phaseStartedAt);
        if (phase === 'ramp') {
            const rampRemaining = Math.max(0, BUILD_ANIMATION.completionRampMs - phaseElapsed);
            return rampRemaining + BUILD_ANIMATION.completionHoldMs + BUILD_ANIMATION.completionVisibleMs;
        }
        if (phase === 'hold') {
            const holdRemaining = Math.max(0, BUILD_ANIMATION.completionHoldMs - phaseElapsed);
            return holdRemaining + BUILD_ANIMATION.completionVisibleMs;
        }
        return Math.max(0, BUILD_ANIMATION.completionVisibleMs - phaseElapsed);
    }

    function finalizeCompletion(pluginId) {
        setCompletionState(pluginId, 'done');
    }

    function easeOutCubic(value) {
        const clamped = Math.max(0, Math.min(1, value));
        const inv = 1 - clamped;
        return 1 - inv * inv * inv;
    }

    function applyFillScale(rowRef, percent) {
        if (!rowRef.fill) return;
        rowRef.fill.style.setProperty('--progress-width', `${normalizePercent(percent).toFixed(2)}%`);
    }

    function stopFillAnimation(rowRef) {
        if (rowRef.animationFrame !== null) {
            cancelAnimationFrame(rowRef.animationFrame);
            rowRef.animationFrame = null;
        }
        rowRef.lastFrameTime = 0;
    }

    function finiteOr(value, fallback) {
        return Number.isFinite(value) ? value : fallback;
    }

    function getCompletionState(pluginId) {
        if (!pluginId) return 'idle';
        const completion = completionByPlugin.get(pluginId);
        if (!completion) return 'idle';
        return completion.state;
    }

    function setCompletionState(pluginId, state, patch = {}) {
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
            phaseStartedAt: finiteOr(patch.phaseStartedAt, finiteOr(previous.phaseStartedAt, finiteOr(patch.startedAt, finiteOr(previous.startedAt, 0))))
        });
    }

    return {
        clearQueued,
        completeRows,
        queue,
        cacheRows,
        syncAll
    };
}
