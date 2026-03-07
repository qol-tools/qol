import { html } from '../../../lib/html.js';

const CORE_SECTIONS = [
    { id: 'runtime', name: 'Runtime', description: 'Socket, state, polling' },
    { id: 'plugins', name: 'Plugins', description: 'Daemon lifecycle, loading' },
    { id: 'core', name: 'Core', description: 'Tray, hotkeys, menu, updates' }
];

function MenuIcon() {
    return html`
        <svg class="plugin-menu-trigger-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
            <circle cx="6" cy="3.5" r="1.8" />
            <circle cx="6" cy="10" r="1.8" />
            <circle cx="6" cy="16.5" r="1.8" />
        </svg>
    `;
}

function CoreLogMenu({ section, muted, filterCount, menuOpen, ctrl }) {
    const onToggle = e => { e.preventDefault(); e.stopPropagation(); ctrl.toggleCoreMenu(section.id); };
    const onMute = e => { e.preventDefault(); e.stopPropagation(); ctrl.closeMenus(); ctrl.toggleCoreLogs(section.id); };
    const onFilters = e => { e.preventDefault(); e.stopPropagation(); ctrl.closeMenus(); ctrl.editCoreLogFilters(section.id); };
    return html`
        <button type="button" class="plugin-menu-trigger" onClick=${onToggle} aria-label=${`Log options for ${section.name}`} aria-expanded=${menuOpen ? 'true' : 'false'}>
            <${MenuIcon} />
        </button>
        <div class=${'plugin-context-menu ' + (menuOpen ? 'open' : '')}>
            <button type="button" class="context-action" onClick=${onMute} aria-label=${(muted ? 'Unmute' : 'Mute') + ' ' + section.name + ' logs'}>
                ${muted ? 'Unmute Logs' : 'Mute Logs'}
            </button>
            <button type="button" class="context-action" onClick=${onFilters} aria-label=${`Edit log filters for ${section.name}`}>
                ${filterCount > 0 ? `Edit Filters (${filterCount})` : 'Edit Filters'}
            </button>
        </div>
    `;
}

function CoreLogInfo({ section, muted }) {
    return html`
        <div class="plugin-info table-col">
            <div class="plugin-copy">
                <div class="plugin-title-row"><span class="plugin-name">${section.name}</span></div>
                <span class="plugin-path">${section.description}</span>
            </div>
            ${muted && html`<div class="plugin-status-badges"><span class="status-badge badge-muted">Muted</span></div>`}
        </div>
    `;
}

function CoreLogRow({ section, ctrl }) {
    const control = ctrl.coreLogControls[section.id] || {};
    const muted = !!control.muted;
    const patterns = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    const menuOpen = ctrl.openCoreMenuId === section.id;
    return html`
        <div class="plugin-row table-list-row status-linked core-log-row" data-core-section=${section.id}>
            <div class="plugin-main table-grid">
                <${CoreLogInfo} section=${section} muted=${muted} />
                <div class="plugin-action-column table-col">
                    <${CoreLogMenu} section=${section} muted=${muted} filterCount=${patterns.length} menuOpen=${menuOpen} ctrl=${ctrl} />
                </div>
            </div>
        </div>
    `;
}

export function CoreLogSection({ ctrl }) {
    return html`
        <section class="dev-section">
            <div class="section-header"><h2>Core Logs</h2></div>
            <div class="plugin-list-container">
                <div class="plugin-list table-list">
                    ${CORE_SECTIONS.map(s => html`<${CoreLogRow} key=${s.id} section=${s} ctrl=${ctrl} />`)}
                </div>
            </div>
        </section>
    `;
}
