import { readResponseText, tryFetchJson } from '../../../api/client.js';

export async function fetchMockTargetsState() {
    const payload = await tryFetchJson('/api/dev/mock-targets');
    if (!payload) return null;

    const runningIds = [];
    const runningById = new Map();
    for (const entry of Array.isArray(payload) ? payload : []) {
        if (!entry || typeof entry.id !== 'string') {
            continue;
        }
        const running = !!entry.running;
        runningById.set(entry.id, running);
        if (running) {
            runningIds.push(entry.id);
        }
    }

    return { runningIds, runningById };
}

export async function startMockTargetsApi() {
    try {
        const response = await fetch('/api/dev/mock-targets/start', { method: 'POST' });
        return response;
    } catch (error) {
        return error;
    }
}

export async function stopMockTargetsApi() {
    try {
        const res = await fetch('/api/dev/mock-targets/stop', { method: 'POST' });
        if (res.status !== 404) {
            return;
        }
        await Promise.allSettled([
            fetch('/api/dev/mock-self-update/stop', { method: 'POST' }),
            fetch('/api/dev/mock-self-recompile/stop', { method: 'POST' }),
            fetch('/api/dev/mock-plugin-build/stop', { method: 'POST' })
        ]);
    } catch (error) {}
}

async function tryPost(url) {
    try {
        return await fetch(url, { method: 'POST' });
    } catch {
        return null;
    }
}

export async function startLegacyMockTargets() {
    const [updateRes, recompileRes, buildRes] = await Promise.all([
        tryPost('/api/dev/mock-self-update'),
        tryPost('/api/dev/mock-self-recompile'),
        tryPost('/api/dev/mock-plugin-build'),
    ]);
    return { updateRes, recompileRes, buildRes };
}

export async function readMockStartError(startRes) {
    const message = await readResponseText(startRes);
    return message || 'Failed to trigger mock targets';
}

export async function buildLegacyStartErrorMessage({
    updateRes,
    recompileRes,
    buildRes,
    needsLocalFallback
}) {
    const updateUnsupported = !!updateRes && updateRes.status === 404;
    const recompileUnsupported = !!recompileRes && recompileRes.status === 404;
    const updateFailed = !updateUnsupported && (!updateRes || !updateRes.ok);
    const recompileFailed = !recompileUnsupported && (!recompileRes || !recompileRes.ok);
    const buildFailed = !needsLocalFallback && buildRes && !buildRes.ok;

    if (!updateFailed && !recompileFailed && !buildFailed) {
        return { message: null, buildFailed };
    }

    const messages = [];
    if (updateFailed) {
        const updateText = updateRes ? await readResponseText(updateRes) : '';
        messages.push(updateText || 'Failed to trigger mock update flow');
    }
    if (recompileFailed) {
        const recompileText = recompileRes ? await readResponseText(recompileRes) : '';
        messages.push(recompileText || 'Failed to trigger mock recompile flow');
    }
    if (buildFailed) {
        const buildText = await readResponseText(buildRes);
        messages.push(buildText || 'Failed to trigger mock plugin build flow');
    }

    return { message: messages.join(' • '), buildFailed };
}
