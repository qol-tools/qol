import { tryFetchJson } from '../../../api/client.js';
import { parseHydratedBuildState } from './reducer.js';

export async function hydrateBuildState(state, clearSync, maybeRender) {
    const payload = await tryFetchJson('/api/dev/build-state');
    if (!payload) {
        state.building = false;
        maybeRender();
        return;
    }
    const nextState = parseHydratedBuildState(payload);
    state.building = nextState.building;
    if (!state.building) {
        applyCompletedHydration(state, nextState, clearSync);
        maybeRender();
        return;
    }
    mergeActiveProgress(state, nextState.buildProgress);
    maybeRender();
}

function applyCompletedHydration(state, nextState, clearSync) {
    state.buildProgress = nextState.buildProgress;
    if (nextState.buildResults) {
        state.buildResults = nextState.buildResults;
    }
    clearSync();
}

function mergeActiveProgress(state, hydratedProgress) {
    for (const [id, hydrated] of Object.entries(hydratedProgress)) {
        const live = state.buildProgress[id];
        if (live && (live.percent ?? 0) > (hydrated.percent ?? 0)) continue;
        state.buildProgress[id] = hydrated;
    }
}
