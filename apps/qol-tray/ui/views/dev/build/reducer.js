import { normalizePercent } from '../../../utils/progress.js';

export function nextBuildStartedState() {
    return {
        building: true,
        error: null,
        buildResults: null,
        buildProgress: {}
    };
}

export function nextBuildProgressState(currentProgress, event) {
    const rawStatus = event.status || 'building';
    const status = rawStatus === 'success' ? 'completed' : rawStatus;
    const normalizedPercent = normalizePercent(event.percent);
    const previous = currentProgress[event.plugin_id];
    return {
        ...currentProgress,
        [event.plugin_id]: {
            status,
            percent: resolveProgressPercent(previous, status, normalizedPercent),
            phase: event.phase || ''
        }
    };
}

function resolveProgressPercent(previous, status, normalizedPercent) {
    if (!previous) return normalizedPercent;
    if (status !== 'building') return normalizedPercent;
    if (previous.status !== 'building') return normalizedPercent;
    if (normalizedPercent <= 1) return normalizedPercent;
    if (normalizedPercent >= previous.percent) return normalizedPercent;
    return previous.percent;
}

export function nextBuildCompletedState(results) {
    return {
        building: false,
        error: null,
        buildResults: results || []
    };
}

export function resetBuildState(state) {
    state.building = false;
    state.buildProgress = {};
    state.buildResults = null;
}

export function parseHydratedBuildState(payload) {
    const building = !!payload?.building;
    const rawProgress = payload?.progress && typeof payload.progress === 'object'
        ? payload.progress
        : {};
    const buildProgress = {};

    for (const [pluginId, entry] of Object.entries(rawProgress)) {
        if (!pluginId || !entry || typeof entry !== 'object') {
            continue;
        }
        buildProgress[pluginId] = {
            status: typeof entry.status === 'string' ? entry.status : 'building',
            percent: normalizePercent(entry.percent),
            phase: typeof entry.phase === 'string' ? entry.phase : ''
        };
    }

    const buildResults = Array.isArray(payload?.results) ? payload.results : null;

    return { building, buildProgress, buildResults };
}
