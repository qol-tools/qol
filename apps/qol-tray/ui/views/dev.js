import { subscribe, onReconnect } from '../events.js';
import { jsonRequest, readResponseText } from '../api/client.js';
import { mergePlugins, renderBuildResults, renderPluginBuildMeta } from './dev/plugin-model.js';
import { renderDevView } from './dev/template.js';
import { createBuildController } from './dev/build-controller.js';
import { createDiscoveryController } from './dev/discovery-controller.js';
import { createMockController } from './dev/mock-controller.js';
import {
    nextDiscoveryCompletedState,
    nextDiscoveryStartedState
} from './dev/discovery/reducer.js';

export const id = 'dev';

function readSavedIndex() {
    const saved = parseInt(localStorage.getItem('dev-selected-index'), 10);
    return Number.isFinite(saved) && saved >= 0 ? saved : 0;
}

const state = {
    building: false,
    buildResults: null,
    lastReload: null,
    error: null,
    plugins: [],
    discovered: [],
    discovering: false,
    selectedIndex: readSavedIndex(),
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
let unsubscribeReconnect = null;
let reloadCooldownUntil = 0;

const discoveryController = createDiscoveryController({
    state,
    onNeedsRender: updateView
});

let mockController = null;

const buildController = createBuildController({
    state,
    getContainer: () => container,
    getPluginById: getMergedPluginById,
    onNeedsRender: updateView,
    onBuildComplete: () => {
        reloadCooldownUntil = Date.now() + 1000;
        mockController?.completeMockTarget('plugin_build');
        updateView();
        void discoveryController.loadLinkedPlugins();
    }
});

mockController = createMockController({
    state,
    buildController,
    getMergedPlugins: () => state.mergedList,
    onNeedsRender: updateView
});

export function render(containerEl) {
    container = containerEl;
    container.addEventListener('click', handleClick);
    unsubscribe = subscribe(handleEvent);
    if (!unsubscribeReconnect) {
        unsubscribeReconnect = onReconnect(() => {
            if (!state.building) return;
            void buildController.hydrateBuildState();
        });
    }
    void Promise.all([
        discoveryController.loadPlugins(true),
        discoveryController.fetchDiscoveryState(true),
        discoveryController.loadLogControls(true),
        buildController.hydrateBuildState(true),
        mockController.hydrateMockTargets(true)
    ]).finally(() => {
        if (!state.linkingId) {
            updateView();
        }
    });
}

function handleEvent(event) {
    if (
        state.linkingId &&
        (event.type === 'discovery_started' ||
            event.type === 'discovery_complete' ||
            event.type === 'plugins_changed')
    ) {
        return;
    }

    if (event.type === 'discovery_started') {
        Object.assign(state, nextDiscoveryStartedState());
        updateView();
        return;
    }

    if (event.type === 'discovery_complete') {
        Object.assign(state, nextDiscoveryCompletedState(event.plugins));
        updateView();
        return;
    }

    if (event.type === 'plugins_changed') {
        void discoveryController.loadLinkedPlugins();
        return;
    }

    buildController.handleEvent(event);
    mockController.handleEvent(event);
}

function totalItems() {
    return state.mergedCount || 0;
}

function getMergedPluginById(pluginId) {
    return state.mergedList.find(plugin => plugin.id === pluginId) || null;
}

function getActivePluginBuildState(plugin) {
    return buildController.getActivePluginBuildState(plugin, state.mockTesting);
}

function updateView() {
    const mergedList = mergePlugins(state.discovered, state.plugins, state.logControls);
    state.mergedCount = mergedList.length;
    state.mergedList = mergedList;
    state.selectedIndex = Math.max(0, Math.min(state.selectedIndex, mergedList.length - 1));
    localStorage.setItem('dev-selected-index', String(state.selectedIndex));

    buildController.pruneInvisibleProgress(new Set(mergedList.map(plugin => plugin.id)));

    const prevScrollTop = container.querySelector('.view-body')?.scrollTop ?? 0;

    container.innerHTML = renderDevView({
        state,
        mergedList,
        getActivePluginBuildState,
        renderPluginBuildMeta,
        renderBuildResults
    });

    const viewBody = container.querySelector('.view-body');
    if (viewBody) viewBody.scrollTop = prevScrollTop;

    const input = container.querySelector('#link-path');
    if (input) {
        input.addEventListener('input', e => {
            state.linkPath = e.target.value;
        });
        input.addEventListener('keydown', e => {
            if (e.key === 'Enter') confirmLink();
            if (e.key === 'Escape') cancelLink();
        });
    }

    buildController.cacheRows();
    buildController.syncAll();
}

function handleClick(e) {
    const action = e.target.closest('[data-action]')?.dataset.action;
    const actionId = e.target.closest('[data-id]')?.dataset.id;

    if (action === 'mock-update') {
        void mockController.triggerMockFlows();
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
    if (action === 'reload') {
        void reloadPlugins();
    }
    if (action === 'refresh-discovery') {
        void discoveryController.triggerDiscovery();
    }
    if (action === 'add-link') showLinkInput();
    if (action === 'confirm-link') void confirmLink();
    if (action === 'cancel-link') cancelLink();
}

function handleItemActivation() {
    const item = state.mergedList[state.selectedIndex];
    if (!item) return;
    if (getActivePluginBuildState(item)) return;

    if (item.status === 'linked') {
        void deleteLink(item.id);
        return;
    }

    if (item.path) {
        void quickLink(item.path, item.id);
        return;
    }

    showLinkInput();
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
        await discoveryController.loadPlugins(true);
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
        await discoveryController.loadPlugins();
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
            discoveryController.loadPlugins(true),
            discoveryController.refreshDiscoveryState()
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
            discoveryController.loadPlugins(true),
            discoveryController.loadLogControls(true)
        ]);
    } catch (error) {
        state.error = error?.message || 'Failed to toggle plugin logs';
    }
    if (!state.linkingId) updateView();
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
            discoveryController.loadPlugins(true),
            discoveryController.loadLogControls(true)
        ]);
    } catch (error) {
        state.error = error?.message || 'Failed to update plugin log filters';
    }
    if (!state.linkingId) updateView();
}

async function triggerReload() {
    const res = await fetch('/api/dev/reload', { method: 'POST' });
    if (!res.ok && res.status !== 409) {
        const message = await readResponseText(res);
        throw new Error(message || 'Failed to queue reload');
    }
    return res;
}

async function reloadPlugins() {
    if (state.building || Date.now() < reloadCooldownUntil) return;

    state.building = true;
    state.error = null;
    state.buildResults = null;
    state.buildProgress = {};
    updateView();

    try {
        const [reloadRes, discoverRes] = await Promise.all([
            fetch('/api/dev/reload', { method: 'POST' }),
            fetch('/api/dev/discover', { method: 'POST' })
        ]);

        if (reloadRes.status === 409) {
            state.building = false;
            return;
        }

        if (!reloadRes.ok) {
            state.building = false;
            state.error = await readResponseText(reloadRes) || 'Reload failed';
            return;
        }

        if (discoverRes.ok) {
            state.lastReload = new Date().toLocaleTimeString();
        }
        await discoveryController.loadPlugins();
    } catch (err) {
        state.building = false;
        state.error = err.message;
    } finally {
        updateView();
    }
}

export function handleKey(e) {
    if (state.showLinkInput) return;

    if ((e.ctrlKey || e.metaKey) && (e.key === 'r' || e.key === 'R')) {
        e.preventDefault();
        void reloadPlugins();
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
        void discoveryController.triggerDiscovery();
    }
}

export function onFocus() {
    if (!state.linkingId) {
        void Promise.all([
            discoveryController.loadPlugins(true),
            discoveryController.fetchDiscoveryState(true),
            discoveryController.loadLogControls(true),
            mockController.hydrateMockTargets(true)
        ]).finally(() => {
            updateView();
        });
    }
    mockController.onFocus();
}

export function onBlur() {
    mockController.onBlur();
}
