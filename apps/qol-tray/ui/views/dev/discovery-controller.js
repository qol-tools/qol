import { tryFetchJson } from '../../api/client.js';
import { parseDiscoveryPayload, parseLogControlsPayload } from './discovery/reducer.js';

export function createDiscoveryController({ state, onNeedsRender }) {
    const ctx = { state, onNeedsRender };
    return {
        loadLogControls: skip => loadLogControls(ctx, skip),
        loadCoreLogControls: skip => loadCoreLogControls(ctx, skip),
        refreshDiscoveryState: () => refreshDiscoveryState(ctx),
        fetchDiscoveryState: skip => fetchDiscoveryState(ctx, skip),
        loadPlugins: skip => loadPlugins(ctx, skip),
        loadLinkedPlugins: () => loadLinkedPlugins(ctx),
        triggerDiscovery: () => triggerDiscovery(ctx)
    };
}

function maybeRender(ctx, skipUpdate) {
    if (!skipUpdate && !ctx.state.linkingId) ctx.onNeedsRender();
}

async function loadLogControls(ctx, skipUpdate = false) {
    const payload = await tryFetchJson('/api/dev/log-controls');
    if (payload) ctx.state.logControls = parseLogControlsPayload(payload);
    maybeRender(ctx, skipUpdate);
}

async function loadCoreLogControls(ctx, skipUpdate = false) {
    const payload = await tryFetchJson('/api/dev/core-log-controls');
    if (payload) ctx.state.coreLogControls = payload;
    maybeRender(ctx, skipUpdate);
}

async function refreshDiscoveryState(ctx) {
    const data = await tryFetchJson('/api/dev/discovery-state');
    if (!data) return;
    const nextState = parseDiscoveryPayload(data, ctx.state.discovered);
    ctx.state.discovering = nextState.discovering;
    ctx.state.discovered = nextState.discovered;
}

async function fetchDiscoveryState(ctx, skipUpdate = false) {
    await refreshDiscoveryState(ctx);
    maybeRender(ctx, skipUpdate);
}

async function loadPlugins(ctx, skipUpdate = false) {
    const plugins = await tryFetchJson('/api/dev/links');
    if (plugins) ctx.state.plugins = plugins;
    maybeRender(ctx, skipUpdate);
}

async function loadLinkedPlugins(ctx) {
    if (ctx.state.linkingId) return;
    const plugins = await tryFetchJson('/api/dev/links');
    if (plugins) ctx.state.plugins = plugins;
    ctx.onNeedsRender();
}

async function triggerDiscovery(ctx) {
    if (ctx.state.discovering) return;
    await fetch('/api/dev/discover', { method: 'POST' });
}
