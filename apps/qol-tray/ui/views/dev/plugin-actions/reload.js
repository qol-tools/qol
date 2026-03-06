import { readResponseText } from '../../../api/client.js';

async function handleReloadResponse(state, reloadResponse) {
    if (reloadResponse.status === 409) { state.building = false; return false; }
    if (!reloadResponse.ok) {
        state.building = false;
        state.error = await readResponseText(reloadResponse) || 'Reload failed';
        return false;
    }
    return true;
}

async function doReload(state, discoveryController, onNeedsRender) {
    try {
        const [reloadResponse, discoverResponse] = await Promise.all([
            fetch('/api/dev/reload', { method: 'POST' }),
            fetch('/api/dev/discover', { method: 'POST' })
        ]);
        if (!await handleReloadResponse(state, reloadResponse)) return;
        if (discoverResponse.ok) state.lastReload = new Date().toLocaleTimeString();
        await discoveryController.loadPlugins();
    } catch (error) {
        state.building = false;
        state.error = error.message;
    } finally {
        onNeedsRender();
    }
}

async function reloadPlugins(state, cooldown, discoveryController, onNeedsRender) {
    if (state.building || Date.now() < cooldown.until) return;
    state.building = true;
    state.error = null;
    state.buildResults = null;
    state.buildProgress = {};
    onNeedsRender();
    await doReload(state, discoveryController, onNeedsRender);
}

async function triggerReload() {
    const response = await fetch('/api/dev/reload', { method: 'POST' });
    if (response.ok || response.status === 409) return response;
    const message = await readResponseText(response);
    throw new Error(message || 'Failed to queue reload');
}

export function createReloadActions({ state, discoveryController, onNeedsRender }) {
    const cooldown = { until: 0 };
    return {
        markReloadComplete: () => { cooldown.until = Date.now() + 1000; },
        reloadPlugins: () => reloadPlugins(state, cooldown, discoveryController, onNeedsRender),
        triggerReload
    };
}
