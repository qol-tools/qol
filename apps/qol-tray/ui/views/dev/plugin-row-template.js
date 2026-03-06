import { escapeAttr, escapeHtml, safeStatusToken } from '../../utils/escape-html.js';
import { renderCpuStrip } from './plugin-row/cpu.js';
import { renderPluginMenuControls } from './plugin-row/menu.js';
import { renderStatusBadges } from './plugin-row/status.js';

function renderPluginInfo(plugin, statusBadges, state, renderPluginBuildMeta) {
    const pluginName = escapeHtml(plugin.name || plugin.id || 'Unknown plugin');
    const pluginPath = escapeHtml(plugin.path || '');
    return `
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
    `;
}

function renderActionColumn(actionDisabled, rebuildActive, statusToken, pluginId, pluginNameAttr, menuControls) {
    return `
        <div class="plugin-action-column table-col">
            <button type="button" class="plugin-action-zone ${actionDisabled ? 'is-disabled' : ''} ${rebuildActive ? 'has-rebuild' : 'rebuild-idle'}" data-action="toggle-link" data-id="${pluginId}" aria-label="${statusToken === 'linked' ? 'Unlink' : 'Link'} ${pluginNameAttr}" ${actionDisabled ? 'disabled' : ''}>
                <img class="plugin-action-rebuild-icon" src="assets/qol-tray.png?v=1" alt="" aria-hidden="true">
            </button>
            ${menuControls}
        </div>
    `;
}

function renderPluginRow(state, plugin, index, getActivePluginBuildState, renderPluginBuildMeta) {
    const isSelected = state.selectedIndex === index;
    const statusToken = safeStatusToken(plugin.status);
    const pluginId = escapeAttr(plugin.id);
    const pluginNameAttr = escapeAttr(plugin.name || plugin.id || 'Unknown plugin');
    const isRowBuilding = !!getActivePluginBuildState(plugin);
    const isLinking = state.linkingId === plugin.id;
    const actionDisabled = isRowBuilding || !!state.linkingId;
    const rebuildActive = plugin.status === 'linked' && plugin.has_cargo && plugin.needs_rebuild;
    const menuControls = renderPluginMenuControls({ state, plugin, menuOpen: state.openPluginMenuId === plugin.id, pluginId, pluginNameAttr });
    const statusBadges = renderStatusBadges(plugin, statusToken);
    const info = renderPluginInfo(plugin, statusBadges, state, renderPluginBuildMeta);
    const actions = renderActionColumn(actionDisabled, rebuildActive, statusToken, pluginId, pluginNameAttr, menuControls);
    return `
        <div class="plugin-row table-list-row status-${statusToken} ${isSelected ? 'selected' : ''} ${isRowBuilding ? 'is-building' : ''} ${isLinking ? 'is-linking' : ''}" data-status="${statusToken}" data-selected="${isSelected ? 'true' : 'false'}" data-index="${index}" data-plugin-id="${pluginId}">
            <div class="plugin-main table-grid">${info}${actions}</div>
            <div class="plugin-build-overlay-host"></div>
        </div>
    `;
}

export function renderPluginRows({
    state,
    mergedList,
    getActivePluginBuildState,
    renderPluginBuildMeta
}) {
    return mergedList.map((plugin, index) => renderPluginRow(state, plugin, index, getActivePluginBuildState, renderPluginBuildMeta)).join('');
}
