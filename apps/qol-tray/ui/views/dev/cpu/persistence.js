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
        if (Array.isArray(parsed)) return arrayToMonitoringMap(parsed);
        if (!parsed || typeof parsed !== 'object') return {};
        return objectToMonitoringMap(parsed);
    } catch {
        return {};
    }
}

function arrayToMonitoringMap(arr) {
    return arr.reduce((acc, pluginId) => {
        if (!isSafePluginId(pluginId)) return acc;
        acc[pluginId] = true;
        return acc;
    }, {});
}

function objectToMonitoringMap(obj) {
    return Object.entries(obj).reduce((acc, [pluginId, enabled]) => {
        if (!isSafePluginId(pluginId)) return acc;
        if (!enabled) return acc;
        acc[pluginId] = true;
        return acc;
    }, {});
}

export function persistCpuMonitoring(monitoredIds) {
    try {
        localStorage.setItem(CPU_MONITORING_STORAGE_KEY, JSON.stringify(monitoredIds));
    } catch {}
}
