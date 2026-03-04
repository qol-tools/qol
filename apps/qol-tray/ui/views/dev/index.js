import { subscribe, onReconnect } from '../../events.js';
import { jsonRequest, readResponseText } from '../../api/client.js';
import { mergePlugins, renderBuildResults, renderPluginBuildMeta } from './plugin-model.js';
import { renderDevView } from './template.js';
import { createBuildController } from './build-controller.js';
import { createDiscoveryController } from './discovery-controller.js';
import { createMockController } from './mock-controller.js';
import {
    nextDiscoveryCompletedState,
    nextDiscoveryStartedState
} from './discovery/reducer.js';

export const id = 'dev';
const CPU_MONITORING_STORAGE_KEY = 'dev-cpu-monitoring';

function saveSpinnerTimes(root) {
    const times = [];
    for (const btn of root.querySelectorAll('.refresh-btn.spinning')) {
        const anim = btn.getAnimations?.()[0];
        times.push(anim ? anim.currentTime : null);
    }
    return times;
}

function restoreSpinnerTimes(root, times) {
    if (!times.length) return;
    const buttons = root.querySelectorAll('.refresh-btn.spinning');
    for (let i = 0; i < buttons.length && i < times.length; i++) {
        if (times[i] === null) continue;
        const anim = buttons[i].getAnimations?.()[0];
        if (anim) anim.currentTime = times[i];
    }
}

function readSavedIndex() {
    return -1;
}

function readSavedCpuMonitoring() {
    try {
        const raw = localStorage.getItem(CPU_MONITORING_STORAGE_KEY);
        if (!raw) return {};
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed)) {
            return parsed.reduce((acc, pluginId) => {
                if (typeof pluginId !== 'string' || !pluginId) return acc;
                acc[pluginId] = true;
                return acc;
            }, {});
        }
        if (!parsed || typeof parsed !== 'object') return {};
        return Object.entries(parsed).reduce((acc, [pluginId, enabled]) => {
            if (typeof pluginId !== 'string' || !pluginId) return acc;
            if (!enabled) return acc;
            acc[pluginId] = true;
            return acc;
        }, {});
    } catch {
        return {};
    }
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
    mockTesting: false,
    cpuMonitoring: readSavedCpuMonitoring(),
    cpuByPlugin: {},
    openPluginMenuId: null
};

let container = null;
let unsubscribe = null;
let unsubscribeReconnect = null;
let reloadCooldownUntil = 0;
let focusRefreshPending = true;
let actionInteractionLocks = 0;
let deferredUpdatePending = false;

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
    if (container) {
        container.removeEventListener('click', handleClick);
    }
    if (unsubscribe) {
        unsubscribe();
        unsubscribe = null;
    }
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

    if (event.type === 'plugin_cpu_snapshot') {
        handleCpuSnapshot(event);
        return;
    }

    buildController.handleEvent(event);
    mockController.handleEvent(event);
}

function totalItems() {
    return state.mergedCount || 0;
}

function closePluginMenu() {
    if (!state.openPluginMenuId) return;
    state.openPluginMenuId = null;
}

function togglePluginMenu(pluginId) {
    if (state.openPluginMenuId === pluginId) {
        state.openPluginMenuId = null;
        return;
    }
    state.openPluginMenuId = pluginId;
}

function syncPluginMenuDom() {
    if (!container) return;
    const rows = container.querySelectorAll('.plugin-row[data-plugin-id]');
    for (const row of rows) {
        const pluginId = row.dataset.pluginId;
        const isOpen = pluginId === state.openPluginMenuId;
        const menu = row.querySelector('.plugin-context-menu');
        if (menu) {
            menu.classList.toggle('open', isOpen);
        }
        const trigger = row.querySelector('.plugin-menu-trigger');
        if (trigger) {
            trigger.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
        }
    }
}

function visiblePluginIdSet() {
    return new Set(state.mergedList.map(plugin => plugin.id));
}

function monitoredCpuPluginIds(visibleOnly = false) {
    const monitored = Object.keys(state.cpuMonitoring).filter(pluginId => !!state.cpuMonitoring[pluginId]);
    if (!visibleOnly) return monitored;
    const visiblePluginIds = visiblePluginIdSet();
    return monitored.filter(pluginId => visiblePluginIds.has(pluginId));
}

function persistCpuMonitoring() {
    try {
        localStorage.setItem(
            CPU_MONITORING_STORAGE_KEY,
            JSON.stringify(monitoredCpuPluginIds())
        );
    } catch {}
}

function isNumber(value) {
    return typeof value === 'number' && Number.isFinite(value);
}

function normalizeCpuHistory(history, fallbackTimestamp) {
    if (!Array.isArray(history)) return [];
    return history
        .filter(point => point && (isNumber(point.cpu_percent) || isNumber(point.timestamp_ms)))
        .map(point => ({
            cpu_percent: isNumber(point.cpu_percent) ? point.cpu_percent : 0,
            timestamp_ms: isNumber(point.timestamp_ms) ? point.timestamp_ms : fallbackTimestamp
        }));
}

function normalizeCpuSample(plugin, fallbackTimestamp) {
    return {
        cpu_percent: isNumber(plugin?.cpu_percent) ? plugin.cpu_percent : 0,
        cpu_seconds_total: isNumber(plugin?.cpu_seconds_total) ? plugin.cpu_seconds_total : 0,
        timestamp_ms: fallbackTimestamp,
        history: normalizeCpuHistory(plugin?.history, fallbackTimestamp)
    };
}

function lastCpuHistoryPoint(sample) {
    const history = Array.isArray(sample?.history) ? sample.history : [];
    if (history.length === 0) return null;
    return history[history.length - 1];
}

function cpuSamplesChanged(prev, current) {
    if (!prev && !current) return false;
    if (!prev || !current) return true;
    if (prev.cpu_percent !== current.cpu_percent) return true;
    if (prev.cpu_seconds_total !== current.cpu_seconds_total) return true;
    if (prev.timestamp_ms !== current.timestamp_ms) return true;

    const prevHistory = Array.isArray(prev.history) ? prev.history : [];
    const currentHistory = Array.isArray(current.history) ? current.history : [];
    if (prevHistory.length !== currentHistory.length) return true;

    const prevLast = lastCpuHistoryPoint(prev);
    const currentLast = lastCpuHistoryPoint(current);
    if (!prevLast && !currentLast) return false;
    if (!prevLast || !currentLast) return true;
    if (prevLast.cpu_percent !== currentLast.cpu_percent) return true;
    return prevLast.timestamp_ms !== currentLast.timestamp_ms;
}

function updateCpuSamples(payload) {
    if (!payload || !Array.isArray(payload.plugins)) return false;

    const monitored = monitoredCpuPluginIds(true);
    if (monitored.length === 0) return false;

    const monitoredSet = new Set(monitored);
    const next = {};
    const timestamp = isNumber(payload.timestamp_ms) ? payload.timestamp_ms : Date.now();

    for (const plugin of payload.plugins) {
        const pluginId = plugin?.plugin_id;
        if (!pluginId) continue;
        if (!monitoredSet.has(pluginId)) continue;
        next[pluginId] = normalizeCpuSample(plugin, timestamp);
    }

    let changed = false;
    for (const pluginId of monitored) {
        if (!cpuSamplesChanged(state.cpuByPlugin[pluginId], next[pluginId])) continue;
        changed = true;
        break;
    }

    if (!changed) return false;
    state.cpuByPlugin = next;
    return true;
}

function handleCpuSnapshot(event) {
    const payload = {
        timestamp_ms: event.timestamp_ms,
        plugins: event.plugins
    };
    if (!updateCpuSamples(payload)) return;
    if (state.linkingId) return;
    updateView();
}

function pruneCpuMonitoring(mergedList) {
    const existingSamples = Object.keys(state.cpuByPlugin);
    for (const pluginId of existingSamples) {
        if (state.cpuMonitoring[pluginId]) continue;
        delete state.cpuByPlugin[pluginId];
    }

    if (!state.openPluginMenuId) return;
    const hasMenuPlugin = mergedList.some(plugin => plugin.id === state.openPluginMenuId);
    if (hasMenuPlugin) return;
    closePluginMenu();
}

function getMergedPluginById(pluginId) {
    return state.mergedList.find(plugin => plugin.id === pluginId) || null;
}

function getActivePluginBuildState(plugin) {
    return buildController.getActivePluginBuildState(plugin, state.mockTesting);
}

function lockActionInteraction() {
    actionInteractionLocks += 1;
}

function unlockActionInteraction() {
    actionInteractionLocks = Math.max(0, actionInteractionLocks - 1);
    if (actionInteractionLocks !== 0) return;
    if (!deferredUpdatePending) return;
    deferredUpdatePending = false;
    updateView();
}

function bindActionInteractionLocks() {
    if (!container) return;
    const columns = container.querySelectorAll('.plugin-action-column');
    for (const column of columns) {
        column.addEventListener('pointerenter', lockActionInteraction);
        column.addEventListener('pointerleave', unlockActionInteraction);
    }
}

function updateView() {
    if (actionInteractionLocks > 0) {
        deferredUpdatePending = true;
        return;
    }

    const mergedList = mergePlugins(state.discovered, state.plugins, state.logControls);
    state.mergedCount = mergedList.length;
    state.mergedList = mergedList;
    pruneCpuMonitoring(mergedList);
    if (mergedList.length === 0) {
        state.selectedIndex = -1;
    }
    if (mergedList.length > 0) {
        state.selectedIndex = Math.max(-1, Math.min(state.selectedIndex, mergedList.length - 1));
    }

    buildController.pruneInvisibleProgress(new Set(mergedList.map(plugin => plugin.id)));

    const prevScrollTop = container.querySelector('.view-body')?.scrollTop ?? 0;
    const spinnerTimes = saveSpinnerTimes(container);
    const hoveredActionZone = container.querySelector('.plugin-action-zone:hover');
    const hoveredActionId = hoveredActionZone?.dataset.id || null;

    container.innerHTML = renderDevView({
        state,
        mergedList,
        getActivePluginBuildState,
        renderPluginBuildMeta,
        renderBuildResults
    });

    const viewBody = container.querySelector('.view-body');
    if (viewBody) viewBody.scrollTop = prevScrollTop;
    restoreSpinnerTimes(container, spinnerTimes);
    if (hoveredActionId) {
        const actionZones = container.querySelectorAll('.plugin-action-zone[data-id]');
        for (const zone of actionZones) {
            if (zone.dataset.id !== hoveredActionId) continue;
            if (zone.classList.contains('is-disabled')) continue;
            zone.classList.add('is-hovered');
            break;
        }
    }

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
    bindActionInteractionLocks();
}

function handleClick(e) {
    const target = e.target instanceof Element ? e.target : e.target?.parentElement;
    if (!target) return;
    const actionTarget = target.closest('[data-action]');
    const action = actionTarget?.dataset.action;
    const actionId = actionTarget?.dataset.id;

    if (!action) {
        if (!state.openPluginMenuId) return;
        closePluginMenu();
        syncPluginMenuDom();
        return;
    }

    if (action === 'mock-update') {
        void mockController.triggerMockFlows();
    }
    if (action === 'toggle-plugin-menu' && actionId) {
        e.preventDefault();
        e.stopPropagation();
        togglePluginMenu(actionId);
        syncPluginMenuDom();
        return;
    }
    if (action === 'toggle-plugin-logs' && actionId) {
        e.preventDefault();
        e.stopPropagation();
        closePluginMenu();
        syncPluginMenuDom();
        void togglePluginLogs(actionId);
        return;
    }
    if (action === 'edit-plugin-log-filters' && actionId) {
        e.preventDefault();
        e.stopPropagation();
        closePluginMenu();
        syncPluginMenuDom();
        void editPluginLogFilters(actionId);
        return;
    }
    if (action === 'toggle-plugin-cpu' && actionId) {
        e.preventDefault();
        e.stopPropagation();
        closePluginMenu();
        syncPluginMenuDom();
        togglePluginCpuMonitoring(actionId);
        return;
    }
    if (action === 'toggle-link' && actionId) {
        if (state.linkingId) return;
        const row = target.closest('.plugin-row');
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
    closePluginMenu();
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

function togglePluginCpuMonitoring(pluginId) {
    if (state.cpuMonitoring[pluginId]) {
        delete state.cpuMonitoring[pluginId];
        delete state.cpuByPlugin[pluginId];
        persistCpuMonitoring();
        updateView();
        return;
    }

    state.cpuMonitoring[pluginId] = true;
    persistCpuMonitoring();
    updateView();
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

    if (e.key === 'Escape') {
        if (!state.openPluginMenuId) return;
        e.preventDefault();
        closePluginMenu();
        syncPluginMenuDom();
        return;
    }

    const total = totalItems();

    if (e.key === 'ArrowDown' && total > 0) {
        e.preventDefault();
        if (state.selectedIndex < 0) {
            state.selectedIndex = 0;
            updateView();
            return;
        }
        state.selectedIndex = Math.min(state.selectedIndex + 1, total - 1);
        updateView();
    }

    if (e.key === 'ArrowUp' && total > 0) {
        e.preventDefault();
        if (state.selectedIndex < 0) {
            state.selectedIndex = total - 1;
            updateView();
            return;
        }
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
        return;
    }

    if (e.key === 'm' || e.key === 'M') {
        e.preventDefault();
        const item = state.mergedList[state.selectedIndex];
        if (!item) return;
        togglePluginMenu(item.id);
        syncPluginMenuDom();
    }
}

export function onFocus() {
    if (state.linkingId) return;
    if (!focusRefreshPending) return;
    focusRefreshPending = false;
    void Promise.all([
        discoveryController.loadPlugins(true),
        discoveryController.fetchDiscoveryState(true),
        discoveryController.loadLogControls(true),
        mockController.hydrateMockTargets(true)
    ]).finally(() => {
        updateView();
    });
}

export function onBlur() {
    closePluginMenu();
    focusRefreshPending = true;
}

export function destroy() {
    if (unsubscribe) {
        unsubscribe();
        unsubscribe = null;
    }
    if (unsubscribeReconnect) {
        unsubscribeReconnect();
        unsubscribeReconnect = null;
    }
    if (container) {
        container.removeEventListener('click', handleClick);
        container = null;
    }
    actionInteractionLocks = 0;
    deferredUpdatePending = false;
}
