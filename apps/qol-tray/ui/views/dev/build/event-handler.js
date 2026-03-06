import { normalizePercent } from '../../../utils/progress.js';
import { nextBuildStartedState, nextBuildProgressState, nextBuildCompletedState } from './reducer.js';

const LOADING_LOG_PREFIX = '[qol-dev-loading]';
const DEBUG_LOADING = false;

function logLoading(event, payload) {
    if (!DEBUG_LOADING) return;
    console.info(`${LOADING_LOG_PREFIX} ${event}`, payload);
}

export function dispatchBuildEvent(event, state, overlay, clearSync, queueSync, onNeedsRender, onBuildComplete) {
    if (event.type === 'build_started') return handleBuildStarted(state, clearSync, onNeedsRender);
    if (event.type === 'build_plugin_progress') return handleBuildProgress(state, event, queueSync);
    if (event.type === 'build_complete') return handleBuildComplete(state, event, overlay, clearSync, onBuildComplete);
}

function handleBuildStarted(state, clearSync, onNeedsRender) {
    logLoading('event:build_started', {});
    Object.assign(state, nextBuildStartedState());
    clearSync();
    onNeedsRender();
}

function handleBuildProgress(state, event, queueSync) {
    logLoading('event:build_plugin_progress', {
        pluginId: event.plugin_id,
        status: event.status || 'building',
        percent: normalizePercent(event.percent, { round: true }),
        phase: event.phase || ''
    });
    state.building = true;
    state.buildProgress = nextBuildProgressState(state.buildProgress, event);
    queueSync(event.plugin_id);
}

function handleBuildComplete(state, event, overlay, clearSync, onBuildComplete) {
    logLoading('event:build_complete', {
        results: Array.isArray(event.results) ? event.results.length : 0
    });
    const completedPluginIds = Object.keys(state.buildProgress);
    overlay.completeRows(completedPluginIds, () => {
        clearSync();
        Object.assign(state, nextBuildCompletedState(event.results));
        onBuildComplete();
    });
}

export function completeLocalBuild(state, overlay, clearSync, maybeRender, results) {
    const ids = Object.keys(state.buildProgress);
    overlay.completeRows(ids, () => {
        clearSync();
        state.building = false;
        state.buildResults = Array.isArray(results) ? results : [];
        maybeRender();
    });
}
