import { apiJson, apiResponse, readResponseText } from '../../api/client.js';

const POLL_TIMEOUT_MS = 900000;

export async function fetchGitHubAuthStatus() {
    return apiJson('/api/github-auth/status');
}

export async function disconnectGitHubAuth() {
    const response = await apiResponse('/api/github-auth', { method: 'DELETE' });
    await throwIfNotOk(response, 'Failed to disconnect GitHub');
}

export async function startGitHubAuth() {
    return apiJson('/api/github-auth/start', { method: 'POST' });
}

export async function waitForGitHubAuth(sessionId, interval) {
    const pollMs = Math.max((interval || 5) * 1000, 5000);
    const deadline = Date.now() + POLL_TIMEOUT_MS;
    while (Date.now() < deadline) {
        await sleep(pollMs);
        const session = await apiJson(
            `/api/github-auth/poll/${encodeURIComponent(sessionId)}`,
            { method: 'POST' },
        );
        if (session.state === 'authorized') {
            return session;
        }
        if (session.state === 'failed') {
            throw new Error(session.error || 'GitHub authorization failed');
        }
    }
    throw new Error('GitHub authorization timed out');
}

function sleep(ms) {
    return new Promise(resolve => window.setTimeout(resolve, ms));
}

async function throwIfNotOk(response, fallback) {
    if (response.ok) {
        return;
    }
    const message = (await readResponseText(response)) || fallback;
    throw new Error(message);
}
