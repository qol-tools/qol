import { tryFetchJson } from '../../api/client.js';
import {
    parseDiscoveryPayload,
    parseLogControlsPayload
} from './discovery/reducer.js';

export function createDiscoveryController({ state, onNeedsRender }) {
    function maybeRender(skipUpdate) {
        if (!skipUpdate && !state.linkingId) onNeedsRender();
    }

    async function loadLogControls(skipUpdate = false) {
        const payload = await tryFetchJson('/api/dev/log-controls');
        if (payload) {
            state.logControls = parseLogControlsPayload(payload);
        }
        maybeRender(skipUpdate);
    }

    async function refreshDiscoveryState() {
        const data = await tryFetchJson('/api/dev/discovery-state');
        if (!data) return;
        const nextState = parseDiscoveryPayload(data, state.discovered);
        state.discovering = nextState.discovering;
        state.discovered = nextState.discovered;
    }

    async function fetchDiscoveryState(skipUpdate = false) {
        await refreshDiscoveryState();
        maybeRender(skipUpdate);
    }

    async function loadPlugins(skipUpdate = false) {
        const plugins = await tryFetchJson('/api/dev/links');
        if (plugins) {
            state.plugins = plugins;
        }
        maybeRender(skipUpdate);
    }

    async function loadLinkedPlugins() {
        if (state.linkingId) return;
        const plugins = await tryFetchJson('/api/dev/links');
        if (plugins) {
            state.plugins = plugins;
        }
        onNeedsRender();
    }

    async function triggerDiscovery() {
        if (state.discovering) return;
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
