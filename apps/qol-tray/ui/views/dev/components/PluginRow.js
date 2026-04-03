import { html } from '../../../lib/html.js';
import { safeStatusToken } from '../../../utils/escape-html.js';
import { DevPluginRow } from '../../../components/rows/DevPluginRow.js';
import { BuildMeta } from './BuildMeta.js';
import { StatusBadges } from './StatusBadges.js';
import { CpuStrip } from './CpuStrip.js';

export function PluginRow({ plugin, index, ctrl }) {
    const isSelected = ctrl.selectedIndex === index;
    const statusToken = safeStatusToken(plugin.status);
    const isBuilding = !!ctrl.getActivePluginBuildState(plugin);
    const isLinking = ctrl.linkingId === plugin.id;
    const actionDisabled = isBuilding || !!ctrl.linkingId;
    const rebuildActive = plugin.status === 'linked' && plugin.has_cargo && plugin.needs_rebuild;

    const actions = buildActions(plugin, statusToken, index, ctrl);

    const icon = isLinking
        ? html`<span class="refresh-btn spinning" aria-hidden="true"></span>`
        : html`<img class="plugin-action-rebuild-icon ${rebuildActive ? 'has-rebuild' : 'rebuild-idle'}" src="assets/qol-tray.png?v=1" alt="" aria-hidden="true" />`;

    return html`
        <${DevPluginRow}
            name=${plugin.name || plugin.id || 'Unknown plugin'}
            path=${plugin.path || ''}
            status=${statusToken}
            pluginId=${plugin.id}
            index=${index}
            selected=${isSelected}
            onSelect=${ctrl.setSelectedIndex}
            actions=${actionDisabled ? [] : actions}
            actionIcon=${icon}
            className=${[isBuilding && 'is-building', isLinking && 'is-linking'].filter(Boolean).join(' ') || undefined}
            badges=${html`<${StatusBadges} plugin=${plugin} statusToken=${statusToken} />`}
            meta=${html`<${BuildMeta} plugin=${plugin} /><${CpuStrip} plugin=${plugin} cpuMonitoring=${ctrl.cpuMonitoring} cpuByPlugin=${ctrl.cpuByPlugin} />`}
        />
    `;
}

function buildActions(plugin, statusToken, index, ctrl) {
    const actions = [];
    actions.push({
        label: statusToken === 'linked' ? 'Unlink' : 'Link',
        run: () => { ctrl.setSelectedIndex(index); ctrl.handleItemActivation(); },
    });
    actions.push({
        label: plugin.logs_muted ? 'Unmute Logs' : 'Mute Logs',
        run: () => { ctrl.closeMenus(); ctrl.togglePluginLogs(plugin.id); },
    });
    const filterCount = Array.isArray(plugin.suppressed_log_patterns) ? plugin.suppressed_log_patterns.length : 0;
    actions.push({
        label: filterCount > 0 ? `Edit Filters (${filterCount})` : 'Edit Filters',
        run: () => { ctrl.closeMenus(); ctrl.editPluginLogFilters(plugin.id); },
    });
    if (plugin.status === 'linked') {
        const enabled = !!ctrl.cpuMonitoring[plugin.id];
        actions.push({
            label: enabled ? 'Disable CPU Monitor' : 'Enable CPU Monitor',
            run: () => { ctrl.closeMenus(); ctrl.toggleCpu(plugin.id); },
        });
    }
    return actions;
}
