import { nextBuildStartedState, nextBuildProgressState, nextBuildCompletedState } from './reducer.js';

export function dispatchBuildEvent(event, state, overlay, clearSync, queueSync, onNeedsRender, onBuildComplete) {
    if (event.type === 'build_started') return handleBuildStarted(state, clearSync, onNeedsRender);
    if (event.type === 'build_plugin_progress') return handleBuildProgress(state, event, queueSync);
    if (event.type === 'build_complete') return handleBuildComplete(state, event, overlay, clearSync, onBuildComplete);
}

function handleBuildStarted(state, clearSync, onNeedsRender) {
    Object.assign(state, nextBuildStartedState());
    clearSync();
    onNeedsRender();
}

function handleBuildProgress(state, event, queueSync) {
    state.building = true;
    state.buildProgress = nextBuildProgressState(state.buildProgress, event);
    queueSync(event.plugin_id);
}

function handleBuildComplete(state, event, overlay, clearSync, onBuildComplete) {
    const completedPluginIds = Object.keys(state.buildProgress)
        .filter(id => state.buildProgress[id]?.status !== 'skipped');
    overlay.cancelPendingSync();
    overlay.completeRows(completedPluginIds, () => {
        clearSync();
        Object.assign(state, nextBuildCompletedState(event.results));
        onBuildComplete();
    });
}

export function completeLocalBuild(state, overlay, clearSync, maybeRender, results) {
    const ids = Object.keys(state.buildProgress)
        .filter(id => state.buildProgress[id]?.status !== 'skipped');
    overlay.cancelPendingSync();
    overlay.completeRows(ids, () => {
        clearSync();
        state.building = false;
        state.buildResults = Array.isArray(results) ? results : [];
        maybeRender();
    });
}
