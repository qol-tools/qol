import { html } from '../../../lib/html.js';
import { DropdownMenu } from '../../../components/DropdownMenu.js';

function MuteLogsAction({ plugin, pluginId, onToggleLogs }) {
    const label = `${plugin.logs_muted ? 'Unmute logs' : 'Mute logs'} for ${plugin.name}`;
    return html`
        <button type="button" class="context-action" onClick=${onToggleLogs} aria-label=${label}>
            ${plugin.logs_muted ? 'Unmute Logs' : 'Mute Logs'}
        </button>
    `;
}

function EditFiltersAction({ plugin, pluginId, onEditFilters }) {
    const count = Array.isArray(plugin.suppressed_log_patterns) ? plugin.suppressed_log_patterns.length : 0;
    return html`
        <button type="button" class="context-action" onClick=${onEditFilters} aria-label=${`Edit log filters for ${plugin.name}`}>
            ${count > 0 ? `Edit Filters (${count})` : 'Edit Filters'}
        </button>
    `;
}

function CpuAction({ plugin, cpuMonitoring, onToggleCpu }) {
    if (plugin.status !== 'linked') return null;
    const enabled = !!cpuMonitoring[plugin.id];
    return html`
        <button type="button" class=${'context-action context-cpu ' + (enabled ? 'stop' : 'start')} onClick=${onToggleCpu} aria-label=${enabled ? `Disable CPU monitoring for ${plugin.name}` : `Enable CPU monitoring for ${plugin.name}`}>
            ${enabled ? 'Disable CPU Monitor' : 'Enable CPU Monitor'}
        </button>
    `;
}

export function PluginMenu({ plugin, menuOpen, cpuMonitoring, onToggleMenu, onCloseMenu, onToggleLogs, onEditFilters, onToggleCpu }) {
    return html`
        <${DropdownMenu}
            open=${menuOpen}
            onToggle=${onToggleMenu}
            onClose=${onCloseMenu}
            triggerLabel=${`Plugin options for ${plugin.name}`}
        >
            <${MuteLogsAction} plugin=${plugin} onToggleLogs=${onToggleLogs} />
            <${EditFiltersAction} plugin=${plugin} onEditFilters=${onEditFilters} />
            <${CpuAction} plugin=${plugin} cpuMonitoring=${cpuMonitoring} onToggleCpu=${onToggleCpu} />
        </${DropdownMenu}>
    `;
}
