import { readResponseText } from '../../../api/client.js';

export function createReloadActions({ state, discoveryController, onNeedsRender }) {
    let reloadCooldownUntil = 0;

    function markReloadComplete() {
        reloadCooldownUntil = Date.now() + 1000;
    }

    async function triggerReload() {
        const response = await fetch('/api/dev/reload', { method: 'POST' });
        if (response.ok || response.status === 409) {
            return response;
        }

        const message = await readResponseText(response);
        throw new Error(message || 'Failed to queue reload');
    }

    async function reloadPlugins() {
        if (state.building || Date.now() < reloadCooldownUntil) {
            return;
        }

        state.building = true;
        state.error = null;
        state.buildResults = null;
        state.buildProgress = {};
        onNeedsRender();

        try {
            const [reloadResponse, discoverResponse] = await Promise.all([
                fetch('/api/dev/reload', { method: 'POST' }),
                fetch('/api/dev/discover', { method: 'POST' })
            ]);
            if (reloadResponse.status === 409) {
                state.building = false;
                return;
            }

            if (!reloadResponse.ok) {
                state.building = false;
                state.error = await readResponseText(reloadResponse) || 'Reload failed';
                return;
            }

            if (discoverResponse.ok) {
                state.lastReload = new Date().toLocaleTimeString();
            }

            await discoveryController.loadPlugins();
        } catch (error) {
            state.building = false;
            state.error = error.message;
        } finally {
            onNeedsRender();
        }
    }

    return {
        markReloadComplete,
        reloadPlugins,
        triggerReload
    };
}
