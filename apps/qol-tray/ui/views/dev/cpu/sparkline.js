const WIDTH = 120;
const HEIGHT = 28;
const FLOOR = HEIGHT - 1;
const CEILING = 2;
const SPREAD = FLOOR - CEILING;
const MIN_MAX_CPU = 5;

export function renderCpuSparkline(history) {
    if (!history.length) {
        return '<div class="plugin-cpu-empty">Waiting for samples</div>';
    }
    const maxCpu = peakCpu(history);
    const toY = value => FLOOR - (Math.max(0, Math.min(value, maxCpu)) / maxCpu) * SPREAD;
    if (history.length === 1) return singlePointSvg(toY, history[0]);
    return multiPointSvg(toY, history);
}

function peakCpu(history) {
    return history.reduce(
        (max, point) => Math.max(max, samplePercent(point)),
        MIN_MAX_CPU
    );
}

function singlePointSvg(toY, point) {
    const y = toY(samplePercent(point)).toFixed(2);
    return sparklineSvg(
        `<polyline class="plugin-cpu-sparkline-line" points="0,${y} ${WIDTH},${y}"></polyline>`
    );
}

function multiPointSvg(toY, history) {
    const points = history.map((point, i) => {
        const x = (i / (history.length - 1)) * WIDTH;
        return `${x.toFixed(2)},${toY(samplePercent(point)).toFixed(2)}`;
    }).join(' ');
    return sparklineSvg(
        `<polyline class="plugin-cpu-sparkline-line" points="${points}"></polyline>`
    );
}

function sparklineSvg(content) {
    return `
        <svg class="plugin-cpu-sparkline" viewBox="0 0 ${WIDTH} ${HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
            <line class="plugin-cpu-sparkline-base" x1="0" y1="${FLOOR}" x2="${WIDTH}" y2="${FLOOR}"></line>
            ${content}
        </svg>
    `;
}

function samplePercent(sample) {
    return Number.isFinite(sample?.cpu_percent) ? sample.cpu_percent : 0;
}
