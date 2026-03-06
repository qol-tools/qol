import {
    readMockStartError,
    startLegacyMockTargets,
    startMockTargetsApi
} from './api.js';
import { setMockFlowSource } from './reducer.js';

const DEFAULT_TARGETS = ['self_update', 'self_recompile', 'plugin_build'];

export async function tryStartMockTargets() {
    const startRes = await startMockTargetsApi();
    if (startRes instanceof Error) {
        return { error: startRes.message || 'Failed to trigger mock targets' };
    }
    if (startRes.ok) {
        return { targets: await parseStartedTargets(startRes) };
    }
    if (startRes.status !== 404) {
        return { error: await readMockStartError(startRes) };
    }
    return { fallbackToLegacy: true };
}

async function parseStartedTargets(response) {
    try {
        const payload = await response.json();
        if (Array.isArray(payload?.started)) {
            const ids = payload.started.filter(id => typeof id === 'string');
            if (ids.length > 0) return ids;
        }
    } catch {}
    return DEFAULT_TARGETS;
}

export async function runLegacyFallback(model) {
    const { updateRes, recompileRes, buildRes } = await startLegacyMockTargets();
    const needsLocalFallback = !buildRes || buildRes.status === 404;
    const nextModel = needsLocalFallback
        ? setMockFlowSource(model, 'local')
        : model;
    return { updateRes, recompileRes, buildRes, needsLocalFallback, model: nextModel };
}
