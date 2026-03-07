import { escapeAttr } from '../../utils/escape-html.js';

const CORE_SECTIONS = [
    { id: 'runtime', name: 'Runtime', description: 'Socket, state, polling' },
    { id: 'plugins', name: 'Plugins', description: 'Daemon lifecycle, loading' },
    { id: 'core', name: 'Core', description: 'Tray, hotkeys, menu, updates' }
];

export function renderCoreLogSection(state) {
    const rows = CORE_SECTIONS.map(section => renderCoreLogRow(state, section)).join('');
    return `
        <section class="dev-section">
            <div class="section-header">
                <h2>Core Logs</h2>
            </div>
            <div class="plugin-list-container">
                <div class="plugin-list table-list">${rows}</div>
            </div>
        </section>
    `;
}

function renderCoreLogRow(state, section) {
    const control = state.coreLogControls[section.id] || {};
    const muted = !!control.muted;
    const patterns = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    const filterCount = patterns.length;
    const sectionId = escapeAttr(section.id);
    const menuOpen = state.openCoreMenuId === section.id;

    return `
        <div class="plugin-row table-list-row status-linked core-log-row" data-core-section="${sectionId}">
            <div class="plugin-main table-grid">
                <div class="plugin-info table-col">
                    <div class="plugin-copy">
                        <div class="plugin-title-row">
                            <span class="plugin-name">${section.name}</span>
                        </div>
                        <span class="plugin-path">${section.description}</span>
                    </div>
                    ${muted ? '<div class="plugin-status-badges"><span class="status-badge badge-muted">Muted</span></div>' : ''}
                </div>
                <div class="plugin-action-column table-col">
                    ${renderCoreLogMenu(sectionId, section.name, muted, filterCount, menuOpen)}
                </div>
            </div>
        </div>
    `;
}

function renderCoreLogMenu(sectionId, sectionName, muted, filterCount, menuOpen) {
    return `
        <button type="button" class="plugin-menu-trigger" data-action="toggle-core-menu" data-id="${sectionId}" aria-label="Log options for ${sectionName}" aria-expanded="${menuOpen ? 'true' : 'false'}">
            <svg class="plugin-menu-trigger-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
                <circle cx="6" cy="3.5" r="1.8"></circle>
                <circle cx="6" cy="10" r="1.8"></circle>
                <circle cx="6" cy="16.5" r="1.8"></circle>
            </svg>
        </button>
        <div class="plugin-context-menu ${menuOpen ? 'open' : ''}">
            <button type="button" class="context-action" data-action="toggle-core-logs" data-id="${sectionId}" aria-label="${muted ? 'Unmute' : 'Mute'} ${sectionName} logs">
                ${muted ? 'Unmute Logs' : 'Mute Logs'}
            </button>
            <button type="button" class="context-action" data-action="edit-core-log-filters" data-id="${sectionId}" aria-label="Edit log filters for ${sectionName}">
                ${filterCount > 0 ? `Edit Filters (${filterCount})` : 'Edit Filters'}
            </button>
        </div>
    `;
}
