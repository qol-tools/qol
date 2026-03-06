import { escapeAttr, escapeHtml, safeStatusToken } from '../../utils/escape-html.js';
import { renderCpuStrip } from './plugin-row/cpu.js';
import { renderPluginMenuControls } from './plugin-row/menu.js';
import { renderStatusBadges } from './plugin-row/status.js';

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
    const buildState = getActivePluginBuildState(plugin);
    const isRowBuilding = !!buildState;
    const isLinking = state.linkingId === plugin.id;
    const actionDisabled = isRowBuilding || !!state.linkingId;
    const rebuildActive = plugin.status === 'linked' && plugin.has_cargo && plugin.needs_rebuild;
    const menuControls = renderPluginMenuControls({
        state,
        plugin,
        menuOpen,
        pluginId,
        pluginNameAttr
    });
    const statusBadges = renderStatusBadges(plugin, statusToken);

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
