export function createCompletionStore({ createSnapshot, computeRemainingMs, finiteOr }) {
    const ctx = { map: new Map(), createSnapshot, computeRemainingMs, finiteOr };
    return {
        clear: id => { if (id) ctx.map.delete(id); },
        clearAll: () => ctx.map.clear(),
        entries: () => ctx.map.entries(),
        finalize: id => setState(ctx, id, 'done'),
        get: id => ctx.map.get(id) || null,
        getState: id => ctx.map.get(id)?.state || 'idle',
        remainingMs: (id, now) => remainingMs(ctx, id, now),
        setState: (id, state, patch) => setState(ctx, id, state, patch),
        snapshot: (id, now) => snapshot(ctx, id, now)
    };
}

function remainingMs(ctx, pluginId, now) {
    const c = ctx.map.get(pluginId);
    if (!c || c.state !== 'playing') return 0;
    return ctx.computeRemainingMs(c, now);
}

function snapshot(ctx, pluginId, now) {
    const c = ctx.map.get(pluginId);
    if (!c || c.state !== 'playing') return null;
    return ctx.createSnapshot(c, now);
}

function setState(ctx, pluginId, state, patch = {}) {
    if (!pluginId) return;
    if (state === 'idle') { ctx.map.delete(pluginId); return; }
    const prev = ctx.map.get(pluginId) || {};
    ctx.map.set(pluginId, {
        state,
        startedAt: ctx.finiteOr(patch.startedAt, ctx.finiteOr(prev.startedAt, 0)),
        startPercent: ctx.finiteOr(patch.startPercent, ctx.finiteOr(prev.startPercent, 100)),
        phase: typeof patch.phase === 'string' ? patch.phase : (prev.phase || 'ramp'),
        phaseStartedAt: ctx.finiteOr(
            patch.phaseStartedAt,
            ctx.finiteOr(prev.phaseStartedAt, ctx.finiteOr(patch.startedAt, ctx.finiteOr(prev.startedAt, 0)))
        )
    });
}
