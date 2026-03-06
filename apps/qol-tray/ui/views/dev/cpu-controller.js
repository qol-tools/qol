import { jsonRequest, tryFetchJson } from '../../api/client.js';

const CPU_MONITORING_STORAGE_KEY = 'dev-cpu-monitoring';

export function isSafePluginId(pluginId) {
    if (typeof pluginId !== 'string') return false;
    if (!/^[A-Za-z0-9_-]{1,64}$/.test(pluginId)) return false;
    return !pluginId.startsWith('-');
}

export function readSavedCpuMonitoring() {
    try {
        const raw = localStorage.getItem(CPU_MONITORING_STORAGE_KEY);
        if (!raw) return {};
        const parsed = JSON.parse(raw);
        if (Array.isArray(parsed)) {
            return parsed.reduce((acc, pluginId) => {
                if (!isSafePluginId(pluginId)) return acc;
                acc[pluginId] = true;
                return acc;
            }, {});
        }
        if (!parsed || typeof parsed !== 'object') return {};
        return Object.entries(parsed).reduce((acc, [pluginId, enabled]) => {
            if (!isSafePluginId(pluginId)) return acc;
            if (!enabled) return acc;
            acc[pluginId] = true;
            return acc;
        }, {});
    } catch {
        return {};
    }
}

export function createCpuController({
    state,
    getVisiblePluginIds,
    onNeedsRender,
    onMissingMenuPlugin
}) {
    let cpuMonitoringSyncChain = Promise.resolve();
    let cpuEnableHydrationTimer = null;

    function monitoredCpuPluginIds(visibleOnly = false) {
        const monitored = Object.keys(state.cpuMonitoring).filter(pluginId => {
            if (!isSafePluginId(pluginId)) return false;
            return !!state.cpuMonitoring[pluginId];
        });
        if (!visibleOnly) return monitored;
        const visiblePluginIds = getVisiblePluginIds();
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

    function queueSync() {
        const pluginIds = monitoredCpuPluginIds();
        cpuMonitoringSyncChain = cpuMonitoringSyncChain
            .catch(() => {})
            .then(() => syncCpuMonitoringState(pluginIds));
        return cpuMonitoringSyncChain;
    }

    async function syncCpuMonitoringState(pluginIds) {
        try {
            await fetch('/api/dev/plugin-cpu/monitoring', {
                ...jsonRequest('PUT', { plugin_ids: pluginIds })
            });
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

    function handleEvent(event) {
        if (event.type !== 'plugin_cpu_snapshot') return false;
        const payload = {
            timestamp_ms: event.timestamp_ms,
            plugins: event.plugins
        };
        if (!updateCpuSamples(payload)) return true;
        if (state.linkingId) return true;
        onNeedsRender();
        return true;
    }

    async function hydrate(skipUpdate = false) {
        const payload = await tryFetchJson('/api/dev/plugin-cpu');
        if (!payload) return;
        if (!updateCpuSamples(payload)) return;
        if (skipUpdate) return;
        if (state.linkingId) return;
        onNeedsRender(true);
    }

    function scheduleHydrationRetry(pluginId, attempts = 6) {
        if (!state.cpuMonitoring[pluginId]) return;
        if (state.cpuByPlugin[pluginId]) return;
        if (attempts <= 0) return;
        clearHydrationTimer();
        cpuEnableHydrationTimer = setTimeout(() => {
            cpuEnableHydrationTimer = null;
            if (!state.cpuMonitoring[pluginId]) return;
            void hydrate().then(() => {
                scheduleHydrationRetry(pluginId, attempts - 1);
            });
        }, 1000);
    }

    function clearHydrationTimer() {
        if (cpuEnableHydrationTimer === null) return;
        clearTimeout(cpuEnableHydrationTimer);
        cpuEnableHydrationTimer = null;
    }

    function prune(mergedList) {
        const existingSamples = Object.keys(state.cpuByPlugin);
        for (const pluginId of existingSamples) {
            if (state.cpuMonitoring[pluginId]) continue;
            delete state.cpuByPlugin[pluginId];
        }

        if (!state.openPluginMenuId) return;
        const hasMenuPlugin = mergedList.some(plugin => plugin.id === state.openPluginMenuId);
        if (hasMenuPlugin) return;
        onMissingMenuPlugin();
    }

    function toggle(pluginId) {
        if (!isSafePluginId(pluginId)) return;

        if (state.cpuMonitoring[pluginId]) {
            delete state.cpuMonitoring[pluginId];
            delete state.cpuByPlugin[pluginId];
            clearHydrationTimer();
            persistCpuMonitoring();
            void queueSync();
            onNeedsRender(true);
            return;
        }

        state.cpuMonitoring[pluginId] = true;
        persistCpuMonitoring();
        void queueSync().finally(() => {
            void hydrate().then(() => {
                scheduleHydrationRetry(pluginId);
            });
        });
        onNeedsRender(true);
    }

    function destroy() {
        clearHydrationTimer();
    }

    return {
        destroy,
        handleEvent,
        hydrate,
        prune,
        queueSync,
        toggle
    };
}
