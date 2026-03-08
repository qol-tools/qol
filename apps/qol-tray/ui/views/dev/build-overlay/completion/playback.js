export function clearAll(ctx) {
    cancelFrame(ctx);
    for (const rowRef of ctx.rowRefs.values()) {
        ctx.clearOverlayNodes(rowRef, ctx.stopFillAnimation);
    }
    ctx.store.clearAll();
}

export function completeRows(ctx, pluginIds, onComplete) {
    const done = typeof onComplete === 'function' ? onComplete : () => {};
    const now = performance.now();
    const result = collectStarts(ctx, pluginIds, now);
    if (result.started === 0) { done(); return false; }
    setTimeout(done, result.longestMs + 20);
    return true;
}

function collectStarts(ctx, pluginIds, now) {
    let started = 0;
    let longestMs = 0;
    for (const pluginId of pluginIds) {
        const status = ctx.store.getState(pluginId);
        if (status === 'done') continue;
        if (status === 'playing') {
            started += 1;
            longestMs = Math.max(longestMs, ctx.store.remainingMs(pluginId, now));
            continue;
        }
        const rowRef = ctx.rowRefs.get(pluginId);
        if (!rowRef || !ctx.ensureOverlayNodes(rowRef)) continue;
        if (!startCompletion(ctx, rowRef, true, now)) continue;
        started += 1;
        longestMs = Math.max(longestMs, ctx.store.remainingMs(pluginId, now));
    }
    return { started, longestMs };
}

export function startCompletion(ctx, rowRef, force = false, now = performance.now()) {
    if (!rowRef.overlay || !rowRef.fill) return false;
    if (ctx.store.getState(rowRef.pluginId) === 'playing') return true;
    const rawPercent = resolveStartPercent(ctx, rowRef);
    if (!force && rawPercent < ctx.buildAnimation.completionTriggerPercent) return false;
    applyStartState(ctx, rowRef, ctx.normalizePercent(rawPercent), now);
    ensureFrame(ctx);
    return true;
}

function resolveStartPercent(ctx, rowRef) {
    const visible = ctx.finiteOr(rowRef.displayPercent, Number.NaN);
    const fallback = ctx.finiteOr(rowRef.lastBuildPercent, 0);
    return Number.isFinite(visible) ? visible : fallback;
}

function applyStartState(ctx, rowRef, startPercent, now) {
    ctx.store.setState(rowRef.pluginId, 'playing', {
        startedAt: now,
        startPercent,
        phase: 'ramp',
        phaseStartedAt: now
    });
    rowRef.completing = false;
    ctx.stopFillAnimation(rowRef);
    rowRef.displayPercent = startPercent;
    rowRef.targetPercent = 100;
    rowRef.lastBuildPercent = 100;
    rowRef.fill.style.removeProperty('--progress-transition-override');
    ctx.applyFillScale(rowRef, startPercent, ctx.normalizePercent);
    rowRef.overlay.classList.remove('is-completing');
    setCompletedCopy(ctx, rowRef);
}

export function startIfReady(ctx, rowRef) {
    const status = ctx.store.getState(rowRef.pluginId);
    if (status === 'done') {
        ctx.clearOverlayNodes(rowRef, ctx.stopFillAnimation);
        return true;
    }
    if (status === 'playing') return syncPlayback(ctx, rowRef);
    if (!ctx.ensureOverlayNodes(rowRef)) return false;
    if (!startCompletion(ctx, rowRef)) return false;
    return syncPlayback(ctx, rowRef);
}

export function syncPlayback(ctx, rowRef) {
    const completion = ctx.store.get(rowRef.pluginId);
    if (!completion) return false;
    if (completion.state === 'done') {
        ctx.clearOverlayNodes(rowRef, ctx.stopFillAnimation);
        return true;
    }
    if (!ctx.ensureOverlayNodes(rowRef)) return false;
    setCompletedCopy(ctx, rowRef);
    if (!ctx.phases.renderFrame(rowRef, completion, performance.now())) {
        ensureFrame(ctx);
        return true;
    }
    ctx.store.finalize(rowRef.pluginId);
    ctx.clearOverlayNodes(rowRef, ctx.stopFillAnimation);
    return true;
}

function tick(ctx, timestamp) {
    ctx.frame.id = null;
    let hasActive = false;
    for (const [pluginId, completion] of ctx.store.entries()) {
        if (completion.state !== 'playing') continue;
        if (ctx.store.remainingMs(pluginId, timestamp) <= 0) {
            finalizeAndClear(ctx, pluginId);
            continue;
        }
        hasActive = true;
        renderRow(ctx, pluginId, completion, timestamp);
    }
    if (hasActive) ensureFrame(ctx);
}

function renderRow(ctx, pluginId, completion, timestamp) {
    const rowRef = ctx.rowRefs.get(pluginId);
    if (!rowRef) return;
    if (!ctx.ensureOverlayNodes(rowRef)) return;
    setCompletedCopy(ctx, rowRef);
    ctx.phases.renderFrame(rowRef, completion, timestamp);
}

function finalizeAndClear(ctx, pluginId) {
    ctx.store.finalize(pluginId);
    const rowRef = ctx.rowRefs.get(pluginId);
    if (rowRef) ctx.clearOverlayNodes(rowRef, ctx.stopFillAnimation);
}

function ensureFrame(ctx) {
    if (ctx.frame.id !== null) return;
    ctx.frame.id = requestAnimationFrame(ts => tick(ctx, ts));
}

function cancelFrame(ctx) {
    if (ctx.frame.id === null) return;
    cancelAnimationFrame(ctx.frame.id);
    ctx.frame.id = null;
}

function setCompletedCopy(ctx, rowRef) {
    ctx.setOverlayCopy(rowRef, 'Completed', 'Reloading plugin');
}
