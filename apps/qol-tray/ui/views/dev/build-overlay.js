import { createOverlayCtx } from './build-overlay/overlay-ctx.js';
import { cancelFrame, cacheRows, queueRow } from './build-overlay/overlay-ops.js';

export function createPluginBuildOverlayController(deps) {
    const ctx = createOverlayCtx(deps);
    return {
        clearQueued: () => { ctx.pendingBuildRows.clear(); cancelFrame(ctx); ctx.completion.clearAll(); },
        completeRows: (pluginIds, onComplete) => ctx.completion.completeRows(pluginIds, onComplete),
        queue: (pluginId, cb) => queueRow(ctx, pluginId, cb),
        cacheRows: () => cacheRows(ctx),
        syncAll: pluginIds => { for (const id of pluginIds) ctx.rowSync.syncRow(id); }
    };
}
