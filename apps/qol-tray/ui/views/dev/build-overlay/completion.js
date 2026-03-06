import { createCompletionPhaseRenderer } from './completion/phases.js';
import {
    clearAll,
    completeRows,
    startCompletion,
    startIfReady,
    syncPlayback
} from './completion/playback.js';
import { createCompletionStore } from './completion/store.js';

export function createCompletionController(deps) {
    const phases = createCompletionPhaseRenderer(deps);
    const store = createCompletionStore({
        createSnapshot: phases.snapshot,
        computeRemainingMs: phases.remainingMs,
        finiteOr: deps.finiteOr
    });
    const ctx = { ...deps, phases, store, frame: { id: null } };
    return {
        clear: pluginId => store.clear(pluginId),
        clearAll: () => clearAll(ctx),
        completeRows: (ids, cb) => completeRows(ctx, ids, cb),
        getState: pluginId => store.getState(pluginId),
        snapshot: (pluginId, now) => store.snapshot(pluginId, now),
        start: (rowRef, force, now) => startCompletion(ctx, rowRef, force, now),
        startIfReady: rowRef => startIfReady(ctx, rowRef),
        syncPlayback: rowRef => syncPlayback(ctx, rowRef)
    };
}
