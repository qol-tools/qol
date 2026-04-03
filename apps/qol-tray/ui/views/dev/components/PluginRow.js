import { html } from '../../../lib/html.js';
import { safeStatusToken } from '../../../utils/escape-html.js';
import { BuildMeta } from './BuildMeta.js';
import { StatusBadges } from './StatusBadges.js';
import { CpuStrip } from './CpuStrip.js';
import { PluginMenu } from './PluginMenu.js';

function PluginInfo({ plugin, statusToken, ctrl }) {
    return html`
        <div class="plugin-info table-col">
            <div class="plugin-copy">
                <div class="plugin-title-row">
                    <span class="plugin-name" data-selected-text="">${plugin.name || plugin.id || 'Unknown plugin'}</span>
                </div>
                <span class="plugin-path" data-selected-text="">${plugin.path || ''}</span>
                <${BuildMeta} plugin=${plugin} />
            </div>
            <${StatusBadges} plugin=${plugin} statusToken=${statusToken} />
            <${CpuStrip} plugin=${plugin} cpuMonitoring=${ctrl.cpuMonitoring} cpuByPlugin=${ctrl.cpuByPlugin} />
        </div>
    `;
}

function makeMenuHandlers(plugin, ctrl) {
    const close = cb => () => { ctrl.closeMenus(); cb(); };
    return {
        onToggleMenu: () => ctrl.togglePluginMenu(plugin.id),
        onCloseMenu: ctrl.closeMenus,
        onToggleLogs: close(() => ctrl.togglePluginLogs(plugin.id)),
        onEditFilters: close(() => ctrl.editPluginLogFilters(plugin.id)),
        onToggleCpu: close(() => ctrl.toggleCpu(plugin.id))
    };
}

function ActionColumn({ plugin, index, statusToken, actionDisabled, isLinking, rebuildActive, ctrl }) {
    const menuOpen = ctrl.openPluginMenuId === plugin.id;
    const icon = isLinking
        ? html`<span class="refresh-btn spinning" aria-hidden="true"></span>`
        : html`<img class="plugin-action-rebuild-icon" src="assets/qol-tray.png?v=1" alt="" aria-hidden="true" />`;
    const onToggleLink = () => { ctrl.setSelectedIndex(index); ctrl.handleItemActivation(); };
    const menuHandlers = makeMenuHandlers(plugin, ctrl);
    return html`
        <div class="plugin-action-column table-col">
            <button type="button" class=${'plugin-action-zone ' + (actionDisabled ? 'is-disabled' : '') + ' ' + (rebuildActive ? 'has-rebuild' : 'rebuild-idle')} onClick=${onToggleLink} aria-label=${(statusToken === 'linked' ? 'Unlink' : 'Link') + ' ' + (plugin.name || plugin.id)} disabled=${actionDisabled}>
                ${icon}
            </button>
            <${PluginMenu} plugin=${plugin} menuOpen=${menuOpen} cpuMonitoring=${ctrl.cpuMonitoring} ...${menuHandlers} />
        </div>
    `;
}

export function PluginRow({ plugin, index, ctrl }) {
    const isSelected = ctrl.selectedIndex === index;
    const statusToken = safeStatusToken(plugin.status);
    const isBuilding = !!ctrl.getActivePluginBuildState(plugin);
    const isLinking = ctrl.linkingId === plugin.id;
    const actionDisabled = isBuilding || !!ctrl.linkingId;
    const rebuildActive = plugin.status === 'linked' && plugin.has_cargo && plugin.needs_rebuild;
    return html`
        <div class=${'plugin-row table-list-row status-' + statusToken + (isSelected ? ' selected' : '') + (isBuilding ? ' is-building' : '') + (isLinking ? ' is-linking' : '')} data-selected-surface="" tabIndex="-1" data-status=${statusToken} data-selected=${isSelected ? 'true' : 'false'} data-index=${index} data-plugin-id=${plugin.id} onFocus=${() => ctrl.setSelectedIndex(index)}>
            <div class="plugin-main table-grid">
                <${PluginInfo} plugin=${plugin} statusToken=${statusToken} ctrl=${ctrl} />
                <${ActionColumn} plugin=${plugin} index=${index} statusToken=${statusToken} actionDisabled=${actionDisabled} isLinking=${isLinking} rebuildActive=${rebuildActive} ctrl=${ctrl} />
            </div>
            <div class="plugin-build-overlay-host"></div>
        </div>
    `;
}
