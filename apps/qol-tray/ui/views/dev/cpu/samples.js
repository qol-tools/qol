import { isNumber, normalizeCpuSample } from './normalize.js';

export function buildNextSamples(plugins, monitoredSet, fallbackTimestamp) {
    const next = {};
    for (const plugin of plugins) {
        const pluginId = plugin?.plugin_id;
        if (!pluginId || !monitoredSet.has(pluginId)) continue;
        next[pluginId] = normalizeCpuSample(plugin, fallbackTimestamp);
    }
    return next;
}

export function anySampleChanged(prev, next, monitoredIds) {
    return monitoredIds.some(
        pluginId => cpuSamplesChanged(prev[pluginId], next[pluginId])
    );
}

function cpuSamplesChanged(prev, current) {
    if (!prev && !current) return false;
    if (!prev || !current) return true;
    if (prev.cpu_percent !== current.cpu_percent) return true;
    if (prev.cpu_seconds_total !== current.cpu_seconds_total) return true;
    if (prev.timestamp_ms !== current.timestamp_ms) return true;
    return historyTailChanged(prev, current);
}

function historyTailChanged(prev, current) {
    const prevHistory = Array.isArray(prev.history) ? prev.history : [];
    const currentHistory = Array.isArray(current.history) ? current.history : [];
    if (prevHistory.length !== currentHistory.length) return true;
    const prevLast = lastPoint(prevHistory);
    const currentLast = lastPoint(currentHistory);
    if (!prevLast && !currentLast) return false;
    if (!prevLast || !currentLast) return true;
    if (prevLast.cpu_percent !== currentLast.cpu_percent) return true;
    return prevLast.timestamp_ms !== currentLast.timestamp_ms;
}

function lastPoint(history) {
    return history.length === 0 ? null : history[history.length - 1];
}
