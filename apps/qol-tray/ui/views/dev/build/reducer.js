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
    return {
        ...currentProgress,
        [event.plugin_id]: {
            status: event.status || 'building',
            percent: normalizePercent(event.percent, { round: true }),
            phase: event.phase || ''
        }
    };
}

export function nextBuildCompletedState(results) {
    return {
        building: false,
        error: null,
        buildResults: results || []
    };
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
            percent: normalizePercent(entry.percent, { round: true }),
            phase: typeof entry.phase === 'string' ? entry.phase : ''
        };
    }

    const buildResults = Array.isArray(payload?.results) ? payload.results : null;

    return { building, buildProgress, buildResults };
}
