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

function detectLegacyFailures(updateRes, recompileRes, buildRes, needsLocalFallback) {
    const updateUnsupported = !!updateRes && updateRes.status === 404;
    const recompileUnsupported = !!recompileRes && recompileRes.status === 404;
    return {
        update: !updateUnsupported && (!updateRes || !updateRes.ok),
        recompile: !recompileUnsupported && (!recompileRes || !recompileRes.ok),
        build: !needsLocalFallback && !!buildRes && !buildRes.ok
    };
}

async function formatFailureMessage(res, fallback) {
    const text = res ? await readResponseText(res) : '';
    return text || fallback;
}

export async function buildLegacyStartErrorMessage(
    updateRes, recompileRes, buildRes, needsLocalFallback
) {
    const failed = detectLegacyFailures(updateRes, recompileRes, buildRes, needsLocalFallback);
    if (!failed.update && !failed.recompile && !failed.build) {
        return { message: null, buildFailed: false };
    }
    const messages = [];
    if (failed.update) messages.push(await formatFailureMessage(updateRes, 'Failed to trigger mock update flow'));
    if (failed.recompile) messages.push(await formatFailureMessage(recompileRes, 'Failed to trigger mock recompile flow'));
    if (failed.build) messages.push(await formatFailureMessage(buildRes, 'Failed to trigger mock plugin build flow'));
    return { message: messages.join(' • '), buildFailed: !!failed.build };
}
