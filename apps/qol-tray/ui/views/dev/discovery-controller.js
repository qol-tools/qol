import {
    parseDiscoveryPayload,
    parseLogControlsPayload
} from './discovery/reducer.js';

export function createDiscoveryController({ state, onNeedsRender }) {
    async function loadLogControls(skipUpdate = false) {
        try {
            const res = await fetch('/api/dev/log-controls');
            if (res.ok) {
                const payload = await res.json();
                state.logControls = parseLogControlsPayload(payload);
            }
        } catch (e) {}

        if (!skipUpdate && !state.linkingId) {
            onNeedsRender();
        }
    }

    async function refreshDiscoveryState() {
        try {
            const res = await fetch('/api/dev/discovery-state');
            if (!res.ok) return;
            const data = await res.json();
            const nextState = parseDiscoveryPayload(data, state.discovered);
            state.discovering = nextState.discovering;
            state.discovered = nextState.discovered;
        } catch (e) {}
    }

    async function fetchDiscoveryState(skipUpdate = false) {
        await refreshDiscoveryState();
        if (!skipUpdate && !state.linkingId) {
            onNeedsRender();
        }
    }

    async function loadPlugins(skipUpdate = false) {
        try {
            const res = await fetch('/api/dev/links');
            if (res.ok) {
                state.plugins = await res.json();
            }
        } catch (e) {
            console.error('Failed to load plugins:', e);
        }

        if (!skipUpdate && !state.linkingId) {
            onNeedsRender();
        }
    }

    async function loadLinkedPlugins() {
        if (state.linkingId) {
            return;
        }
        try {
            const res = await fetch('/api/dev/links');
            if (res.ok) {
                state.plugins = await res.json();
            }
            onNeedsRender();
        } catch (e) {}
    }

    async function triggerDiscovery() {
        if (state.discovering) {
            return;
        }
        await fetch('/api/dev/discover', { method: 'POST' });
    }

    return {
        loadLogControls,
        refreshDiscoveryState,
        fetchDiscoveryState,
        loadPlugins,
        loadLinkedPlugins,
        triggerDiscovery
    };
}
