import { escapeAttr } from '../../../utils/escape-html.js';
import { cpuBadgeAria, cpuMonitoringEnabled } from './cpu.js';

export function renderPluginMenuControls({ state, plugin, menuOpen, pluginId, pluginNameAttr }) {
    const filterCount = Array.isArray(plugin.suppressed_log_patterns)
        ? plugin.suppressed_log_patterns.length
        : 0;
    const cpuEnabled = cpuMonitoringEnabled(state, plugin.id);
    const cpuActionLabel = cpuEnabled ? 'Disable CPU Monitor' : 'Enable CPU Monitor';
    const cpuActionClass = cpuEnabled ? 'stop' : 'start';

    return `
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
}
