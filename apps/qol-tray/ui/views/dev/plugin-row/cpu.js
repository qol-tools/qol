import { renderCpuSparkline } from '../cpu/sparkline.js';

const CPU_HISTORY_GRAPH_LIMIT = 36;

export function cpuMonitoringEnabled(state, pluginId) {
    return !!state.cpuMonitoring[pluginId];
}

export function cpuBadgeAria(state, plugin) {
    if (!cpuMonitoringEnabled(state, plugin.id)) {
        return `Enable CPU monitoring for ${plugin.name}`;
    }
    return `Disable CPU monitoring for ${plugin.name}`;
}

export function renderCpuStrip(state, plugin) {
    if (!cpuMonitoringEnabled(state, plugin.id)) return '';
    const sample = state.cpuByPlugin[plugin.id];
    const history = Array.isArray(sample?.history)
        ? sample.history.slice(-CPU_HISTORY_GRAPH_LIMIT)
        : [];
    const cpuPercent = Number.isFinite(sample?.cpu_percent) ? sample.cpu_percent : 0;
    return `
        <div class="plugin-cpu-strip">
            <span class="plugin-cpu-strip-value">${cpuPercent.toFixed(2)}%</span>
            <div class="plugin-cpu-strip-graph">
                ${renderCpuSparkline(history)}
            </div>
        </div>
    `;
}
