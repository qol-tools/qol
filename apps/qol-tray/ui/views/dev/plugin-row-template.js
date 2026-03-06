import { escapeAttr, escapeHtml, safeStatusToken } from '../../utils/escape-html.js';

const SPARKLINE_WIDTH = 120;
const SPARKLINE_HEIGHT = 28;
const SPARKLINE_FLOOR = SPARKLINE_HEIGHT - 1;
const SPARKLINE_CEILING = 2;
const SPARKLINE_SPREAD = SPARKLINE_FLOOR - SPARKLINE_CEILING;
const SPARKLINE_MIN_MAX_CPU = 5;
const CPU_HISTORY_GRAPH_LIMIT = 36;

export function renderPluginRows({
    state,
    mergedList,
    getActivePluginBuildState,
    renderPluginBuildMeta
}) {
    return mergedList.map((plugin, index) => renderPluginRow({
        state,
        plugin,
        index,
        getActivePluginBuildState,
        renderPluginBuildMeta
    })).join('');
}

function renderPluginRow({
    state,
    plugin,
    index,
    getActivePluginBuildState,
    renderPluginBuildMeta
}) {
    const isSelected = state.selectedIndex === index;
    const menuOpen = state.openPluginMenuId === plugin.id;
    const statusToken = safeStatusToken(plugin.status);
    const pluginId = escapeAttr(plugin.id);
    const pluginName = escapeHtml(plugin.name || plugin.id || 'Unknown plugin');
    const pluginNameAttr = escapeAttr(plugin.name || plugin.id || 'Unknown plugin');
    const pluginPath = escapeHtml(plugin.path || '');
    const cpuEnabled = cpuMonitoringEnabled(state, plugin.id);
    const buildState = getActivePluginBuildState(plugin);
    const isRowBuilding = !!buildState;
    const isLinking = state.linkingId === plugin.id;
    const actionDisabled = isRowBuilding || !!state.linkingId;
    const rebuildActive = plugin.status === 'linked' && plugin.has_cargo && plugin.needs_rebuild;
    const filterCount = Array.isArray(plugin.suppressed_log_patterns)
        ? plugin.suppressed_log_patterns.length
        : 0;
    const cpuActionLabel = cpuEnabled ? 'Disable CPU Monitor' : 'Enable CPU Monitor';
    const cpuActionClass = cpuEnabled ? 'stop' : 'start';
    const menuControls = `
        <button type="button" class="plugin-menu-trigger" data-action="toggle-plugin-menu" data-id="${pluginId}" aria-label="Plugin options for ${pluginNameAttr}" aria-expanded="${menuOpen ? 'true' : 'false'}">
            <svg class="plugin-menu-trigger-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
                <circle cx="6" cy="3.5" r="1.8"></circle>
                <circle cx="6" cy="10" r="1.8"></circle>
                <circle cx="6" cy="16.5" r="1.8"></circle>
            </svg>
        </button>
        <div class="plugin-context-menu ${menuOpen ? 'open' : ''}">
            <button type="button" class="context-action" data-action="toggle-plugin-logs" data-id="${pluginId}" aria-label="${plugin.logs_muted ? 'Unmute logs' : 'Mute logs'} for ${pluginNameAttr}">
                ${plugin.logs_muted ? 'Unmute Logs' : 'Mute Logs'}
            </button>
            <button type="button" class="context-action" data-action="edit-plugin-log-filters" data-id="${pluginId}" aria-label="Edit log filters for ${pluginNameAttr}">
                ${filterCount > 0 ? `Edit Filters (${filterCount})` : 'Edit Filters'}
            </button>
            <button type="button" class="context-action context-cpu ${cpuActionClass}" data-action="toggle-plugin-cpu" data-id="${pluginId}" aria-label="${escapeAttr(cpuBadgeAria(state, plugin))}">
                ${cpuActionLabel}
            </button>
        </div>
    `;
    const statusBadges = `
        <div class="plugin-status-badges">
            ${statusBadge(statusToken)}
            ${buildBadge(plugin, statusToken)}
            ${plugin.hasStoreInstall ? '<span class="badge badge-installed-dim">+Store</span>' : ''}
        </div>
    `;

    return `
        <div class="plugin-row table-list-row status-${statusToken} ${isSelected ? 'selected' : ''} ${isRowBuilding ? 'is-building' : ''} ${isLinking ? 'is-linking' : ''}" data-status="${statusToken}" data-selected="${isSelected ? 'true' : 'false'}" data-index="${index}" data-plugin-id="${pluginId}">
            <div class="plugin-main table-grid">
                <div class="plugin-info table-col">
                    <div class="plugin-copy">
                        <div class="plugin-title-row">
                            <span class="plugin-name">${pluginName}</span>
                        </div>
                        <span class="plugin-path">${pluginPath}</span>
                        ${renderPluginBuildMeta(plugin)}
                    </div>
                    ${statusBadges}
                    ${renderCpuStrip(state, plugin)}
                </div>
                <div class="plugin-action-column table-col">
                    <button type="button" class="plugin-action-zone ${actionDisabled ? 'is-disabled' : ''} ${rebuildActive ? 'has-rebuild' : 'rebuild-idle'}" data-action="toggle-link" data-id="${pluginId}" aria-label="${statusToken === 'linked' ? 'Unlink' : 'Link'} ${pluginNameAttr}" ${actionDisabled ? 'disabled' : ''}>
                        <img class="plugin-action-rebuild-icon" src="assets/qol-tray.png?v=1" alt="" aria-hidden="true">
                    </button>
                    ${menuControls}
                </div>
            </div>
            <div class="plugin-build-overlay-host"></div>
        </div>
    `;
}

function cpuMonitoringEnabled(state, pluginId) {
    return !!state.cpuMonitoring[pluginId];
}

function cpuBadgeAria(state, plugin) {
    if (!cpuMonitoringEnabled(state, plugin.id)) {
        return `Enable CPU monitoring for ${plugin.name}`;
    }
    return `Disable CPU monitoring for ${plugin.name}`;
}

function renderCpuStrip(state, plugin) {
    if (!cpuMonitoringEnabled(state, plugin.id)) return '';
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

function statusBadge(statusToken) {
    return {
        linked: '<span class="badge badge-linked">Linked</span>',
        installed: '<span class="badge badge-installed">Installed</span>',
        local: '<span class="badge badge-local">Local Clone</span>'
    }[statusToken] || '';
}

function buildBadge(plugin, statusToken) {
    if (statusToken === 'linked' && !plugin.supports_platform) {
        return '<span class="badge badge-build-skip">Unsupported</span>';
    }
    if (statusToken === 'linked' && plugin.supports_platform && !plugin.has_cargo) {
        return '<span class="badge badge-build-skip">No Cargo</span>';
    }
    return '';
}
