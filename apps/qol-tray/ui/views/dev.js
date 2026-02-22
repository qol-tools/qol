import { subscribe } from '../events.js';
import { jsonRequest, readResponseText } from '../api/client.js';
import { clampPercent, formatBuildOverlayDetail, normalizePercent } from '../utils/progress.js';
import { mergePlugins, renderBuildResults, renderPluginBuildMeta } from './dev/plugin-model.js';
import { renderDevView } from './dev/template.js';
import { createPluginBuildOverlayController } from './dev/build-overlay.js';

export const id = 'dev';

const state = {
    reloading: false,
    building: false,
    buildResults: null,
    lastReload: null,
    error: null,
    plugins: [],
    discovered: [],
    discovering: false,
    selectedIndex: 0,
    showLinkInput: false,
    linkPath: '',
    linkError: null,
    mergedList: [],
    mergedCount: 0,
    logControls: {},
    linkingId: null,
    buildProgress: {},
    mockTesting: false
};

let container = null;
let unsubscribe = null;
let activeMockRunId = 0;
let mockBuildSource = null;
const activeMockTargets = new Set();
const buildOverlayController = createPluginBuildOverlayController({
    getContainer: () => container,
    getPluginById: getMergedPluginById,
    getBuildState: getActivePluginBuildState,
    formatDetail: formatBuildOverlayDetail,
    normalizePercent
});

export function render(containerEl) {
    container = containerEl;
    container.addEventListener('click', handleClick);
    unsubscribe = subscribe(handleEvent);
    void Promise.all([
        loadPlugins(true),
        fetchDiscoveryState(true),
        loadLogControls(true),
        hydrateBuildState(true)
    ]).finally(() => {
        if (!state.linkingId) updateView();
    });
}

function handleEvent(event) {
    if (state.linkingId && (event.type === 'discovery_started' || event.type === 'discovery_complete' || event.type === 'plugins_changed')) {
        return;
    }
    if (event.type === 'discovery_started') {
        state.discovering = true;
        updateView();
    } else if (event.type === 'discovery_complete') {
        state.discovering = false;
        state.discovered = event.plugins || [];
        updateView();
    } else if (event.type === 'plugins_changed') {
        loadLinkedPlugins();
    } else if (event.type === 'build_started') {
        state.building = true;
        state.buildResults = null;
        state.buildProgress = {};
        clearQueuedBuildRowSync();
        updateView();
    } else if (event.type === 'build_plugin_progress') {
        state.buildProgress[event.plugin_id] = {
            status: event.status || 'building',
            percent: clampPercent(event.percent),
            phase: event.phase || ''
        };
        queueBuildRowSync(event.plugin_id);
    } else if (event.type === 'build_complete') {
        clearQueuedBuildRowSync();
        state.building = false;
        state.buildResults = event.results || [];
        completeMockTarget('plugin_build');
        updateView();
        loadLinkedPlugins();
    } else if (event.type === 'update_complete' || event.type === 'update_failed') {
        completeMockTarget('self_update');
    } else if (event.type === 'self_recompile_complete' || event.type === 'self_recompile_failed') {
        completeMockTarget('self_recompile');
    }
}

async function fetchDiscoveryState(skipUpdate = false) {
    await refreshDiscoveryState();
    if (!skipUpdate && !state.linkingId) updateView();
}

async function loadLogControls(skipUpdate = false) {
    try {
        const res = await fetch('/api/dev/log-controls');
        if (res.ok) {
            const payload = await res.json();
            state.logControls = payload && typeof payload === 'object' ? payload : {};
        }
    } catch (e) {}

    if (!skipUpdate && !state.linkingId) updateView();
}

async function hydrateBuildState(skipUpdate = false) {
    try {
        const res = await fetch('/api/dev/build-state');
        if (!res.ok) return;

        const payload = await res.json();
        state.building = !!payload?.building;

        const progress = payload?.progress && typeof payload.progress === 'object'
            ? payload.progress
            : {};
        const nextProgress = {};

        for (const [pluginId, entry] of Object.entries(progress)) {
            if (!pluginId || !entry || typeof entry !== 'object') continue;
            nextProgress[pluginId] = {
                status: typeof entry.status === 'string' ? entry.status : 'building',
                percent: normalizePercent(entry.percent, { round: true }),
                phase: typeof entry.phase === 'string' ? entry.phase : ''
            };
        }

        state.buildProgress = nextProgress;
        if (!state.building) {
            clearQueuedBuildRowSync();
        }

        if (!skipUpdate && !state.linkingId) updateView();
    } catch (e) {}
}

async function loadLinkedPlugins() {
    if (state.linkingId) return;
    try {
        const res = await fetch('/api/dev/links');
        if (res.ok) state.plugins = await res.json();
        updateView();
    } catch (e) {}
}

async function loadPlugins(skipUpdate = false) {
    try {
        const res = await fetch('/api/dev/links');
        if (res.ok) state.plugins = await res.json();
    } catch (e) {
        console.error('Failed to load plugins:', e);
    }
    if (!skipUpdate && !state.linkingId) updateView();
}

function totalItems() {
    return state.mergedCount || 0;
}

function getActivePluginBuildState(plugin) {
    if (!state.building) return null;
    if (!state.mockTesting && plugin.status !== 'linked') return null;
    const progress = state.buildProgress[plugin.id];
    if (!progress) return null;

    const status = progress.status || 'building';
    if (status !== 'queued' && status !== 'building') return null;

    // In normal builds only plugins that actually need rebuild should render
    // queued/compiling overlays. During mock test flows we intentionally show
    // progress for all linked plugins to validate animation behavior.
    if (!state.mockTesting && (!plugin.has_cargo || !plugin.needs_rebuild)) {
        return null;
    }

    const percent = normalizePercent(progress.percent, { round: true });
    const phase = (progress.phase || '').trim() || (status === 'queued' ? 'Queued' : 'Compiling');
    return { status, percent, phase };
}

function getMergedPluginById(pluginId) {
    return state.mergedList.find(plugin => plugin.id === pluginId) || null;
}

function clearQueuedBuildRowSync() {
    buildOverlayController.clearQueued();
}

function queueBuildRowSync(pluginId) {
    buildOverlayController.queue(pluginId, updateView);
}

function updateView() {
    const mergedList = mergePlugins(state.discovered, state.plugins, state.logControls);
    state.mergedCount = mergedList.length;
    state.mergedList = mergedList;
    state.selectedIndex = Math.max(0, Math.min(state.selectedIndex, mergedList.length - 1));

    const visibleIds = new Set(mergedList.map(plugin => plugin.id));
    for (const pluginId of Object.keys(state.buildProgress)) {
        if (!visibleIds.has(pluginId)) {
            delete state.buildProgress[pluginId];
        }
    }

    container.innerHTML = renderDevView({
        state,
        mergedList,
        getActivePluginBuildState,
        renderPluginBuildMeta,
        renderBuildResults
    });

    const input = container.querySelector('#link-path');
    if (input) {
        input.addEventListener('input', e => { state.linkPath = e.target.value; });
        input.addEventListener('keydown', e => {
            if (e.key === 'Enter') confirmLink();
            if (e.key === 'Escape') cancelLink();
        });
    }

    buildOverlayController.cacheRows();
    if (state.building) {
        buildOverlayController.syncAll(Object.keys(state.buildProgress), updateView);
    }
}

function handleClick(e) {
    const action = e.target.closest('[data-action]')?.dataset.action;
    const actionId = e.target.closest('[data-id]')?.dataset.id;

    if (action === 'mock-update') {
        void triggerMockFlows();
    }
    if (action === 'toggle-plugin-logs' && actionId) {
        e.preventDefault();
        e.stopPropagation();
        void togglePluginLogs(actionId);
        return;
    }
    if (action === 'edit-plugin-log-filters' && actionId) {
        e.preventDefault();
        e.stopPropagation();
        void editPluginLogFilters(actionId);
        return;
    }
    if (action === 'toggle-link' && actionId) {
        if (state.linkingId) return;
        const row = e.target.closest('.plugin-row');
        if (row) {
            state.selectedIndex = parseInt(row.dataset.index);
        }
        handleItemActivation();
        updateView();
        return;
    }
    if (action === 'reload') reloadPlugins();
    if (action === 'refresh-discovery') triggerDiscovery();
    if (action === 'add-link') showLinkInput();
    if (action === 'confirm-link') confirmLink();
    if (action === 'cancel-link') cancelLink();

}

function handleItemActivation() {
    const item = state.mergedList[state.selectedIndex];
    if (!item) return;
    if (getActivePluginBuildState(item)) return;

    if (item.status === 'linked') {
        deleteLink(item.id);
    } else if (item.path) {
        quickLink(item.path, item.id);
    } else {
        showLinkInput();
    }
}

function seedDiscoveredFromLinked(pluginId) {
    if (!pluginId) return;

    const linked = state.plugins.find(plugin => plugin.id === pluginId);
    const merged = state.mergedList.find(plugin => plugin.id === pluginId);
    const path = linked?.source || merged?.path || '';
    if (!path) return;

    const seeded = {
        id: pluginId,
        name: linked?.name || merged?.name || pluginId,
        path
    };

    const existingIndex = state.discovered.findIndex(plugin => plugin.id === pluginId);
    if (existingIndex >= 0) {
        state.discovered[existingIndex] = {
            ...state.discovered[existingIndex],
            ...seeded
        };
        return;
    }

    state.discovered.push(seeded);
}

async function quickLink(path, id) {
    if (state.linkingId) return;
    state.linkingId = id;
    updateView();

    try {
        const res = await fetch('/api/dev/links', {
            ...jsonRequest('POST', { path, id })
        });
        if (!res.ok) {
            console.error('Failed to link:', await readResponseText(res));
            return;
        }
        await triggerReload();
        await loadPlugins(true);
    } catch (e) {
        console.error('Failed to link:', e);
    } finally {
        state.linkingId = null;
        updateView();
    }
}

function showLinkInput() {
    state.showLinkInput = true;
    state.linkError = null;
    updateView();
}

function cancelLink() {
    state.showLinkInput = false;
    state.linkPath = '';
    state.linkError = null;
    updateView();
}

async function confirmLink() {
    if (!state.linkPath.trim()) {
        state.linkError = 'Enter a path';
        updateView();
        return;
    }

    try {
        const res = await fetch('/api/dev/links', {
            ...jsonRequest('POST', { path: state.linkPath })
        });

        if (!res.ok) {
            state.linkError = await readResponseText(res);
            updateView();
            return;
        }

        state.showLinkInput = false;
        state.linkPath = '';
        state.linkError = null;
        await triggerReload();
        await loadPlugins();
    } catch (e) {
        state.linkError = e.message;
        updateView();
    }
}

async function deleteLink(id) {
    if (state.linkingId) return;
    state.linkingId = id;
    seedDiscoveredFromLinked(id);
    updateView();

    try {
        const res = await fetch(`/api/dev/links/${id}`, { method: 'DELETE' });
        if (!res.ok) {
            console.error('Failed to delete link:', await readResponseText(res));
            return;
        }
        await triggerReload();
        await Promise.all([
            loadPlugins(true),
            refreshDiscoveryState()
        ]);
    } catch (e) {
        console.error('Failed to delete link:', e);
    } finally {
        state.linkingId = null;
        updateView();
    }
}

function getCurrentLinkedPlugin(pluginId) {
    return state.plugins.find(plugin => plugin.id === pluginId) || null;
}

function normalizePatternsInput(raw) {
    if (!raw) return [];
    return raw
        .split(',')
        .map(value => value.trim())
        .filter(Boolean);
}

async function savePluginLogControl(pluginId, control) {
    const res = await fetch(`/api/dev/log-controls/${encodeURIComponent(pluginId)}`, {
        ...jsonRequest('PUT', control)
    });
    if (!res.ok) {
        const message = await readResponseText(res);
        throw new Error(message || 'Failed to update plugin log control');
    }
}

async function togglePluginLogs(pluginId) {
    const plugin = getCurrentLinkedPlugin(pluginId) || getMergedPluginById(pluginId);
    if (!plugin) return;

    try {
        await savePluginLogControl(pluginId, {
            muted: !plugin.logs_muted,
            suppress_patterns: Array.isArray(plugin.suppressed_log_patterns)
                ? plugin.suppressed_log_patterns
                : []
        });
        await Promise.all([
            loadPlugins(true),
            loadLogControls(true)
        ]);
        updateView();
    } catch (error) {
        state.error = error?.message || 'Failed to toggle plugin logs';
        updateView();
    }
}

async function editPluginLogFilters(pluginId) {
    const plugin = getCurrentLinkedPlugin(pluginId) || getMergedPluginById(pluginId);
    if (!plugin) return;

    const current = Array.isArray(plugin.suppressed_log_patterns)
        ? plugin.suppressed_log_patterns
        : [];
    const initial = current.join(', ');
    const value = window.prompt(
        'Mute log lines containing these comma-separated substrings (leave empty to clear):',
        initial
    );
    if (value === null) return;

    const suppress_patterns = normalizePatternsInput(value);

    try {
        await savePluginLogControl(pluginId, {
            muted: !!plugin.logs_muted,
            suppress_patterns
        });
        await Promise.all([
            loadPlugins(true),
            loadLogControls(true)
        ]);
        updateView();
    } catch (error) {
        state.error = error?.message || 'Failed to update plugin log filters';
        updateView();
    }
}

async function refreshDiscoveryState() {
    try {
        const res = await fetch('/api/dev/discovery-state');
        if (!res.ok) return;
        const data = await res.json();
        state.discovering = data.status === 'discovering';
        if (data.status === 'complete') {
            state.discovered = data.plugins;
        }
    } catch (e) {}
}

async function triggerReload() {
    const res = await fetch('/api/dev/reload', { method: 'POST' });
    if (!res.ok && res.status !== 409) {
        const message = await readResponseText(res);
        throw new Error(message || 'Failed to queue reload');
    }
    return res;
}

async function triggerDiscovery() {
    if (state.discovering) return;
    await fetch('/api/dev/discover', { method: 'POST' });
}

function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

function setActiveMockTargets(targetIds) {
    activeMockTargets.clear();
    for (const targetId of targetIds || []) {
        if (typeof targetId === 'string' && targetId.length) {
            activeMockTargets.add(targetId);
        }
    }
}

function clearActiveMockTargets() {
    activeMockTargets.clear();
}

function completeMockTarget(targetId) {
    if (!state.mockTesting || mockBuildSource !== 'backend') return;
    if (!activeMockTargets.delete(targetId)) return;
    if (activeMockTargets.size > 0) return;

    state.mockTesting = false;
    mockBuildSource = null;
    updateView();
}

function isCurrentMockRun(runId) {
    return state.mockTesting && activeMockRunId === runId;
}

function stopLocalMockBuildUi() {
    clearQueuedBuildRowSync();
    state.building = false;
    state.buildProgress = {};
    state.buildResults = null;
}

async function runLocalMockPluginBuild(runId) {
    if (!isCurrentMockRun(runId)) return false;

    const pluginIds = state.mergedList
        .map(plugin => plugin.id)
        .filter(Boolean)
        .sort((a, b) => a.localeCompare(b));

    clearQueuedBuildRowSync();
    state.building = true;
    state.buildResults = null;
    state.buildProgress = {};

    for (const pluginId of pluginIds) {
        state.buildProgress[pluginId] = {
            status: 'queued',
            percent: 0,
            phase: 'Queued'
        };
    }
    updateView();

    if (pluginIds.length === 0) {
        await sleep(100);
        if (!isCurrentMockRun(runId)) return false;
        state.building = false;
        state.buildResults = [];
        updateView();
        return true;
    }

    for (const pluginId of pluginIds) {
        if (!isCurrentMockRun(runId)) return false;
        state.buildProgress[pluginId] = {
            status: 'building',
            percent: 0,
            phase: '0/24 preparing'
        };
        queueBuildRowSync(pluginId);
        await sleep(120);

        for (let done = 1; done <= 24; done += 1) {
            if (!isCurrentMockRun(runId)) return false;
            state.buildProgress[pluginId] = {
                status: 'building',
                percent: Math.floor((done * 100) / 24),
                phase: `${done}/24 compiling`
            };
            queueBuildRowSync(pluginId);
            await sleep(55);
        }
    }

    if (!isCurrentMockRun(runId)) return false;
    state.building = false;
    state.buildResults = pluginIds.map(plugin_id => ({
        plugin_id,
        success: true,
        output: 'Local mock build completed',
        skipped: false
    }));
    clearQueuedBuildRowSync();
    updateView();
    return true;
}

async function stopMockFlows() {
    if (!state.mockTesting) return;

    activeMockRunId += 1;
    state.mockTesting = false;
    const source = mockBuildSource;
    mockBuildSource = null;
    clearActiveMockTargets();

    if (source === 'local') {
        stopLocalMockBuildUi();
    }
    updateView();

    if (source === 'backend') {
        try {
            const res = await fetch('/api/dev/mock-targets/stop', { method: 'POST' });
            if (res.status === 404) {
                await Promise.allSettled([
                    fetch('/api/dev/mock-self-update/stop', { method: 'POST' }),
                    fetch('/api/dev/mock-self-recompile/stop', { method: 'POST' }),
                    fetch('/api/dev/mock-plugin-build/stop', { method: 'POST' })
                ]);
            }
        } catch (err) {}
    }
}

async function triggerMockFlows() {
    if (state.mockTesting) {
        await stopMockFlows();
        return;
    }

    const runId = activeMockRunId + 1;
    activeMockRunId = runId;
    mockBuildSource = null;
    clearActiveMockTargets();
    state.mockTesting = true;
    state.error = null;
    updateView();

    try {
        const startRes = await fetch('/api/dev/mock-targets/start', { method: 'POST' });
        if (!isCurrentMockRun(runId)) return;

        if (startRes.ok) {
            let started = [];
            try {
                const payload = await startRes.json();
                if (Array.isArray(payload?.started)) {
                    started = payload.started.filter(id => typeof id === 'string');
                }
            } catch (err) {}

            if (started.length === 0) {
                started = ['self_update', 'self_recompile', 'plugin_build'];
            }

            setActiveMockTargets(started);
            mockBuildSource = 'backend';
            state.mockTesting = activeMockTargets.size > 0;
            if (!state.mockTesting) {
                mockBuildSource = null;
            }
            updateView();
            return;
        }

        if (startRes.status !== 404) {
            const message = await readResponseText(startRes);
            state.error = message || 'Failed to trigger mock targets';
            state.mockTesting = false;
            mockBuildSource = null;
            clearActiveMockTargets();
            updateView();
            return;
        }
    } catch (err) {
        if (!isCurrentMockRun(runId)) return;
        state.error = err?.message || 'Failed to trigger mock targets';
        state.mockTesting = false;
        mockBuildSource = null;
        clearActiveMockTargets();
        updateView();
        return;
    }

    // Legacy fallback for older daemons that don't provide /dev/mock-targets/start.
    let updateRes = null;
    let recompileRes = null;
    let buildRes = null;

    try {
        updateRes = await fetch('/api/dev/mock-self-update', { method: 'POST' });
    } catch (err) {
        updateRes = null;
    }
    if (!isCurrentMockRun(runId)) return;

    try {
        recompileRes = await fetch('/api/dev/mock-self-recompile', { method: 'POST' });
    } catch (err) {
        recompileRes = null;
    }
    if (!isCurrentMockRun(runId)) return;

    try {
        buildRes = await fetch('/api/dev/mock-plugin-build', { method: 'POST' });
    } catch (err) {
        buildRes = null;
    }
    if (!isCurrentMockRun(runId)) return;

    const needsLocalFallback = !buildRes || buildRes.status === 404;
    if (needsLocalFallback) {
        mockBuildSource = 'local';
        const completed = await runLocalMockPluginBuild(runId);
        if (!completed || !isCurrentMockRun(runId)) return;
        state.mockTesting = false;
        mockBuildSource = null;
    } else if (buildRes.ok) {
        mockBuildSource = 'backend';
        setActiveMockTargets(['plugin_build']);
    }

    const updateUnsupported = !!updateRes && updateRes.status === 404;
    const recompileUnsupported = !!recompileRes && recompileRes.status === 404;
    const updateFailed = !updateUnsupported && (!updateRes || !updateRes.ok);
    const recompileFailed = !recompileUnsupported && (!recompileRes || !recompileRes.ok);
    const buildFailed = !needsLocalFallback && buildRes && !buildRes.ok;

    if (updateFailed || recompileFailed || buildFailed) {
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
        state.error = messages.join(' • ');
    }

    if (buildFailed) {
        state.mockTesting = false;
        mockBuildSource = null;
        clearActiveMockTargets();
    }

    updateView();
}

async function reloadPlugins() {
    if (state.reloading || state.building) return;

    state.reloading = true;
    state.error = null;
    state.buildResults = null;
    updateView();

    try {
        const [reloadRes, discoverRes] = await Promise.all([
            fetch('/api/dev/reload', { method: 'POST' }),
            fetch('/api/dev/discover', { method: 'POST' })
        ]);

        if (reloadRes.ok && discoverRes.ok) {
            state.lastReload = new Date().toLocaleTimeString();
            await loadPlugins();
        } else if (reloadRes.status === 409) {
            state.error = 'Build already in progress';
            await Promise.all([
                loadPlugins(true),
                hydrateBuildState(true)
            ]);
        } else {
            const [reloadText, discoverText] = await Promise.all([
                readResponseText(reloadRes),
                readResponseText(discoverRes)
            ]);
            state.error = reloadText || discoverText || 'Reload or discovery trigger failed';
        }
    } catch (err) {
        state.error = err.message;
    } finally {
        state.reloading = false;
        updateView();
    }
}

export function handleKey(e) {
    if (state.showLinkInput) return;

    if ((e.ctrlKey || e.metaKey) && (e.key === 'r' || e.key === 'R')) {
        e.preventDefault();
        reloadPlugins();
        return;
    }

    if (e.ctrlKey || e.altKey || e.metaKey) return;

    const total = totalItems();

    if (e.key === 'ArrowDown' && total > 0) {
        e.preventDefault();
        state.selectedIndex = Math.min(state.selectedIndex + 1, total - 1);
        updateView();
    }

    if (e.key === 'ArrowUp' && total > 0) {
        e.preventDefault();
        state.selectedIndex = Math.max(state.selectedIndex - 1, 0);
        updateView();
    }

    if (e.key === ' ' || e.key === 'Enter') {
        e.preventDefault();
        handleItemActivation();
    }

    if (e.key === 'r' || e.key === 'R') {
        e.preventDefault();
        triggerDiscovery();
    }
}

export function onFocus() {
    if (!state.linkingId) {
        void Promise.all([
            loadPlugins(true),
            fetchDiscoveryState(true),
            loadLogControls(true),
            hydrateBuildState(true)
        ]).finally(() => {
            updateView();
        });
    }
    if (!unsubscribe) {
        unsubscribe = subscribe(handleEvent);
    }
}

export function onBlur() {
    unsubscribe?.();
    unsubscribe = null;
}
