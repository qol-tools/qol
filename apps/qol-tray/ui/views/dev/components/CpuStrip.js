import { html } from '../../../lib/html.js';

const WIDTH = 120;
const HEIGHT = 28;
const FLOOR = HEIGHT - 1;
const SPREAD = FLOOR - 2;
const MIN_MAX_CPU = 5;
const CPU_HISTORY_GRAPH_LIMIT = 36;

function samplePercent(sample) {
    return Number.isFinite(sample?.cpu_percent) ? sample.cpu_percent : 0;
}

function peakCpu(history) {
    return history.reduce((max, p) => Math.max(max, samplePercent(p)), MIN_MAX_CPU);
}

function CpuSparkline({ history }) {
    if (!history.length) return html`<div class="plugin-cpu-empty">Waiting for samples</div>`;
    const maxCpu = peakCpu(history);
    const toY = v => FLOOR - (Math.max(0, Math.min(v, maxCpu)) / maxCpu) * SPREAD;
    const pts = history.length === 1
        ? `0,${toY(samplePercent(history[0])).toFixed(2)} ${WIDTH},${toY(samplePercent(history[0])).toFixed(2)}`
        : history.map((p, i) => `${((i / (history.length - 1)) * WIDTH).toFixed(2)},${toY(samplePercent(p)).toFixed(2)}`).join(' ');
    return html`
        <svg class="plugin-cpu-sparkline" viewBox="0 0 ${WIDTH} ${HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
            <line class="plugin-cpu-sparkline-base" x1="0" y1="${FLOOR}" x2="${WIDTH}" y2="${FLOOR}" />
            <polyline class="plugin-cpu-sparkline-line" points="${pts}" />
        </svg>
    `;
}

export function CpuStrip({ plugin, cpuMonitoring, cpuByPlugin }) {
    if (plugin.status !== 'linked') return null;
    if (!cpuMonitoring[plugin.id]) return null;
    const sample = cpuByPlugin[plugin.id];
    const history = Array.isArray(sample?.history)
        ? sample.history.slice(-CPU_HISTORY_GRAPH_LIMIT)
        : [];
    const cpuPercent = Number.isFinite(sample?.cpu_percent) ? sample.cpu_percent : 0;
    return html`
        <div class="plugin-cpu-strip">
            <span class="plugin-cpu-strip-value">${cpuPercent.toFixed(2)}%</span>
            <div class="plugin-cpu-strip-graph"><${CpuSparkline} history=${history} /></div>
        </div>
    `;
}
