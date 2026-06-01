import { cacheRowRefs } from './dom.js';

export function cancelFrame(ctx) {
    if (ctx.buildSyncFrame === null) return;
    cancelAnimationFrame(ctx.buildSyncFrame);
    ctx.buildSyncFrame = null;
}

export function queueRow(ctx, pluginId, onNeedsFullRender) {
    if (!pluginId) return;
    ctx.pendingBuildRows.add(pluginId);
    if (ctx.buildSyncFrame !== null) return;
    ctx.buildSyncFrame = requestAnimationFrame(() => {
        ctx.buildSyncFrame = null;
        let needsFullRender = false;
        for (const queuedId of ctx.pendingBuildRows) {
            if (!ctx.rowSync.syncRow(queuedId)) { needsFullRender = true; break; }
        }
        ctx.pendingBuildRows.clear();
        if (needsFullRender && typeof onNeedsFullRender === 'function') onNeedsFullRender();
    });
}

export function cacheRows(ctx) {
    const previousRows = new Map(ctx.rowRefs);
    cancelFrame(ctx);
    for (const rowRef of previousRows.values()) ctx.fill.stopFillAnimation(rowRef);
    ctx.rowRefs.clear();
    const nextRows = cacheRowRefs(ctx.getContainer, previousRows, ctx.completion.snapshot);
    for (const [pluginId, rowRef] of nextRows.entries()) ctx.rowRefs.set(pluginId, rowRef);
}
