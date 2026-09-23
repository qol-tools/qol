export function isNumber(value) {
    return typeof value === 'number' && Number.isFinite(value);
}

export function normalizeCpuSample(plugin, fallbackTimestamp) {
    return {
        cpu_percent: isNumber(plugin?.cpu_percent) ? plugin.cpu_percent : 0,
        cpu_seconds_total: isNumber(plugin?.cpu_seconds_total) ? plugin.cpu_seconds_total : 0,
        timestamp_ms: fallbackTimestamp,
        history: normalizeCpuHistory(plugin?.history, fallbackTimestamp)
    };
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
