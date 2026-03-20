import { isSafePluginId, persistCpuMonitoring, readSavedCpuMonitoring } from './cpu/persistence.js';
import { isNumber } from './cpu/normalize.js';
import { buildNextSamples, anySampleChanged } from './cpu/samples.js';
import { createSyncChain, fetchCpuSnapshot } from './cpu/sync.js';
import { createHydrationScheduler } from './cpu/hydration.js';

export { isSafePluginId, readSavedCpuMonitoring };

function resolveMonitoredIds(cpuMonitoring, getVisiblePluginIds, visibleOnly) {
    const ids = Object.keys(cpuMonitoring).filter(
        id => isSafePluginId(id) && cpuMonitoring[id]
    );
    if (!visibleOnly) return ids;
    return ids.filter(id => getVisiblePluginIds().has(id));
}

function applyUpdate(state, monitoredIds, payload) {
    if (!payload || !Array.isArray(payload.plugins)) return false;
    if (monitoredIds.length === 0) return false;
    const ts = isNumber(payload.timestamp_ms) ? payload.timestamp_ms : Date.now();
    const next = buildNextSamples(payload.plugins, new Set(monitoredIds), ts);
    if (!anySampleChanged(state.cpuByPlugin, next, monitoredIds)) return false;
    state.cpuByPlugin = next;
    return true;
}

function routeCpuEvent(event, update, state, onNeedsRender) {
    if (event.type !== 'plugin_cpu_snapshot') return false;
    if (!update({ timestamp_ms: event.timestamp_ms, plugins: event.plugins })) return true;
    if (!state.linkingId) onNeedsRender();
    return true;
}

async function hydrateCpu(state, update, onNeedsRender, skipUpdate) {
    const payload = await fetchCpuSnapshot();
    if (!payload || !update(payload)) return;
    if (!skipUpdate && !state.linkingId) onNeedsRender(true);
}

function pruneSamples(state, mergedList, persist, queueSync, onMissingMenuPlugin) {
    const linkedIds = new Set(mergedList.filter(p => p.status === 'linked').map(p => p.id));
    if (linkedIds.size === 0) return;
    let monitoringChanged = false;

    for (const id of Object.keys(state.cpuMonitoring)) {
        if (linkedIds.has(id)) continue;
        delete state.cpuMonitoring[id];
        monitoringChanged = true;
    }
    for (const id of Object.keys(state.cpuByPlugin)) {
        if (!linkedIds.has(id) || !state.cpuMonitoring[id]) delete state.cpuByPlugin[id];
    }
    if (monitoringChanged) {
        persist();
        void queueSync();
    }

    if (!state.openPluginMenuId) return;
    if (mergedList.some(p => p.id === state.openPluginMenuId)) return;
    onMissingMenuPlugin();
}

function toggleCpu(id, state, persist, hydration, queueSync, hydrate, onNeedsRender) {
    if (!isSafePluginId(id)) return;
    if (state.cpuMonitoring[id]) {
        delete state.cpuMonitoring[id];
        delete state.cpuByPlugin[id];
        hydration.clear();
        persist();
        void queueSync();
        onNeedsRender(true);
        return;
    }
    state.cpuMonitoring[id] = true;
    persist();
    void queueSync().finally(() => {
        void hydrate().then(() => hydration.schedule(state, id, hydrate));
    });
    onNeedsRender(true);
}

export function createCpuController({ state, getVisiblePluginIds, onNeedsRender, onMissingMenuPlugin }) {
    const sync = createSyncChain();
    const hydration = createHydrationScheduler();
    const ids = vis => resolveMonitoredIds(state.cpuMonitoring, getVisiblePluginIds, vis);
    const persist = () => persistCpuMonitoring(ids());
    const doSync = () => sync.queue(ids());
    const update = p => applyUpdate(state, ids(true), p);
    const hydrate = skip => hydrateCpu(state, update, onNeedsRender, skip);
    return {
        destroy: hydration.clear,
        handleEvent: e => routeCpuEvent(e, update, state, onNeedsRender),
        hydrate,
        prune: list => pruneSamples(state, list, persist, doSync, onMissingMenuPlugin),
        queueSync: doSync,
        toggle: id => toggleCpu(id, state, persist, hydration, doSync, hydrate, onNeedsRender)
    };
}
