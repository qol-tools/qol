import { html } from '../../../lib/html.js';
import { useState } from 'preact/hooks';
import { DropdownMenu } from '../../../components/DropdownMenu.js';
import { isDebugEnabled, setDebugEnabled } from '../../../lib/debug.js';

const CORE_SECTIONS = [
    { id: 'runtime', name: 'Runtime', description: 'Socket, state, polling' },
    { id: 'plugins', name: 'Plugins', description: 'Daemon lifecycle, loading' },
    { id: 'core', name: 'Core', description: 'Tray, hotkeys, menu, updates' }
];

function CoreLogMenu({ section, muted, filterCount, menuOpen, ctrl }) {
    const onToggle = () => ctrl.toggleCoreMenu(section.id);
    const onMute = () => { ctrl.closeMenus(); ctrl.toggleCoreLogs(section.id); };
    const onFilters = () => { ctrl.closeMenus(); ctrl.editCoreLogFilters(section.id); };
    return html`
        <${DropdownMenu}
            open=${menuOpen}
            onToggle=${onToggle}
            onClose=${ctrl.closeMenus}
            triggerLabel=${`Log options for ${section.name}`}
        >
            <button type="button" class="context-action" onClick=${onMute} aria-label=${(muted ? 'Unmute' : 'Mute') + ' ' + section.name + ' logs'}>
                ${muted ? 'Unmute Logs' : 'Mute Logs'}
            </button>
            <button type="button" class="context-action" onClick=${onFilters} aria-label=${`Edit log filters for ${section.name}`}>
                ${filterCount > 0 ? `Edit Filters (${filterCount})` : 'Edit Filters'}
            </button>
        </${DropdownMenu}>
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

function FrontendDebugToggle() {
    const [on, setOn] = useState(isDebugEnabled);
    const [menuOpen, setMenuOpen] = useState(false);
    const toggle = () => { const next = !on; setDebugEnabled(next); setOn(next); };
    return html`
        <div class=${`plugin-row table-list-row core-log-row ${on ? 'status-debug-on' : 'status-debug-off'}`}>
            <div class="plugin-main table-grid">
                <div class="plugin-info table-col">
                    <div class="plugin-copy">
                        <div class="plugin-title-row"><span class="plugin-name">Frontend Debug</span></div>
                        <span class="plugin-path">Console logging for UI navigation, focus, surface</span>
                    </div>
                </div>
                <div class="plugin-action-column table-col">
                    <button type="button" class="plugin-action-zone" onClick=${toggle}
                        aria-label=${on ? 'Disable frontend debug' : 'Enable frontend debug'}>
                        <span class="debug-toggle-label">${on ? 'ON' : 'OFF'}</span>
                    </button>
                    <${DropdownMenu}
                        open=${menuOpen}
                        onToggle=${() => setMenuOpen(!menuOpen)}
                        onClose=${() => setMenuOpen(false)}
                        triggerLabel="Frontend debug filter options"
                    >
                        <button type="button" class="context-action" onClick=${() => setMenuOpen(false)}>
                            Edit Filters
                        </button>
                    <//>
                </div>
            </div>
        </div>
    `;
}

export function CoreLogSection({ ctrl }) {
    return html`
        <section class="dev-section">
            <div class="section-header"><h2>Logs</h2></div>
            <div class="plugin-list-container">
                <div class="plugin-list table-list">
                    ${CORE_SECTIONS.map(s => html`<${CoreLogRow} key=${s.id} section=${s} ctrl=${ctrl} />`)}
                    <${FrontendDebugToggle} />
                </div>
            </div>
        </section>
    `;
}
