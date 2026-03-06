const SPARKLINE_WIDTH = 120;
const SPARKLINE_HEIGHT = 28;
const SPARKLINE_FLOOR = SPARKLINE_HEIGHT - 1;
const SPARKLINE_CEILING = 2;
const SPARKLINE_SPREAD = SPARKLINE_FLOOR - SPARKLINE_CEILING;
const SPARKLINE_MIN_MAX_CPU = 5;
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
    if (!cpuMonitoringEnabled(state, plugin.id)) {
        return '';
    }

    const sample = state.cpuByPlugin[plugin.id];
    const history = Array.isArray(sample?.history)
        ? sample.history.slice(-CPU_HISTORY_GRAPH_LIMIT)
        : [];
    const cpuPercent = sampleCpuPercent(sample);
    return `
        <div class="plugin-cpu-strip">
            <span class="plugin-cpu-strip-value">${cpuPercent.toFixed(2)}%</span>
            <div class="plugin-cpu-strip-graph">
                ${renderCpuSparkline(history)}
            </div>
        </div>
    `;
}

function renderCpuSparkline(history) {
    if (!history.length) {
        return '<div class="plugin-cpu-empty">Waiting for samples</div>';
    }

    const maxCpu = history.reduce((maxValue, point) => {
        const value = sampleCpuPercent(point);
        return Math.max(maxValue, value);
    }, SPARKLINE_MIN_MAX_CPU);
    const pointY = value => {
        const normalized = Math.max(0, Math.min(value, maxCpu)) / maxCpu;
        return SPARKLINE_FLOOR - normalized * SPARKLINE_SPREAD;
    };

    if (history.length === 1) {
        const y = pointY(sampleCpuPercent(history[0])).toFixed(2);
        return `
            <svg class="plugin-cpu-sparkline" viewBox="0 0 ${SPARKLINE_WIDTH} ${SPARKLINE_HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
                <line class="plugin-cpu-sparkline-base" x1="0" y1="${SPARKLINE_FLOOR}" x2="${SPARKLINE_WIDTH}" y2="${SPARKLINE_FLOOR}"></line>
                <polyline class="plugin-cpu-sparkline-line" points="0,${y} ${SPARKLINE_WIDTH},${y}"></polyline>
            </svg>
        `;
    }

    const points = history.map((point, index) => {
        const value = sampleCpuPercent(point);
        const x = (index / (history.length - 1)) * SPARKLINE_WIDTH;
        const y = pointY(value);
        return `${x.toFixed(2)},${y.toFixed(2)}`;
    }).join(' ');

    return `
        <svg class="plugin-cpu-sparkline" viewBox="0 0 ${SPARKLINE_WIDTH} ${SPARKLINE_HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
            <line class="plugin-cpu-sparkline-base" x1="0" y1="${SPARKLINE_FLOOR}" x2="${SPARKLINE_WIDTH}" y2="${SPARKLINE_FLOOR}"></line>
            <polyline class="plugin-cpu-sparkline-line" points="${points}"></polyline>
        </svg>
    `;
}

function sampleCpuPercent(sample) {
    return Number.isFinite(sample?.cpu_percent) ? sample.cpu_percent : 0;
}
