import { readResponseText } from '../../../api/client.js';

export async function fetchMockTargetsState() {
    let res;
    try {
        res = await fetch('/api/dev/mock-targets');
    } catch (error) {
        return null;
    }
    if (!res.ok) {
        return null;
    }

    let payload;
    try {
        payload = await res.json();
    } catch (error) {
        return null;
    }

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

export async function startLegacyMockTargets() {
    let updateRes = null;
    let recompileRes = null;
    let buildRes = null;

    try {
        updateRes = await fetch('/api/dev/mock-self-update', { method: 'POST' });
    } catch (error) {
        updateRes = null;
    }

    try {
        recompileRes = await fetch('/api/dev/mock-self-recompile', { method: 'POST' });
    } catch (error) {
        recompileRes = null;
    }

    try {
        buildRes = await fetch('/api/dev/mock-plugin-build', { method: 'POST' });
    } catch (error) {
        buildRes = null;
    }

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
