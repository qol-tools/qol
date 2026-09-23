import { normalizePercent } from '../../../utils/progress.js';

export function getActivePluginBuildState(state, plugin, mockTesting) {
    if (!state.building) return null;
    if (!mockTesting && plugin.status !== 'linked') return null;
    const progress = state.buildProgress[plugin.id];
    if (!progress) return null;
    const status = progress.status || 'building';
    if (!isVisibleStatus(status)) return null;
    return formatActiveState(status, progress);
}

function isVisibleStatus(status) {
    return status === 'queued' || status === 'building' || status === 'completed';
}

function formatActiveState(status, progress) {
    const percent = status === 'completed' ? 100 : normalizePercent(progress.percent);
    const phase = (progress.phase || '').trim()
        || (status === 'queued' ? 'Queued' : status === 'completed' ? 'Completed' : 'Compiling');
    return { status, percent, phase };
}

export function pruneInvisibleProgress(state, visibleIds) {
    if (state.building) return;
    for (const pluginId of Object.keys(state.buildProgress)) {
        if (!visibleIds.has(pluginId)) delete state.buildProgress[pluginId];
    }
}
