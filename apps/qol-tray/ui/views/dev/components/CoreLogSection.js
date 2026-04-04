import { html } from '../../../lib/html.js';
import { useState } from 'preact/hooks';
import { TableRow } from '../../../components/TableRow.js';
import { DropdownMenu } from '../../../components/DropdownMenu.js';
import { useListSelection } from '../../../hooks/useListSelection.js';
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

function CoreLogRow({ section, ctrl, index, selected, onSelect }) {
    const control = ctrl.coreLogControls[section.id] || {};
    const muted = !!control.muted;
    const patterns = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    const menuOpen = ctrl.openCoreMenuId === section.id;
    return html`
        <${TableRow} className="plugin-row status-linked core-log-row" accent="success"
            index=${index} selected=${selected} onSelect=${onSelect}
            data-core-section=${section.id}>
            <div class="plugin-info">
                <div class="plugin-copy">
                    <div class="plugin-title-row"><span class="plugin-name">${section.name}</span></div>
                    <span class="plugin-path">${section.description}</span>
                </div>
                ${muted && html`<div class="plugin-status-badges"><span class="status-badge badge-muted">Muted</span></div>`}
            </div>
            <div class="plugin-action-column">
                <${CoreLogMenu} section=${section} muted=${muted} filterCount=${patterns.length} menuOpen=${menuOpen} ctrl=${ctrl} />
            </div>
        <//>
    `;
}

function FrontendDebugToggle({ index, selected, onSelect }) {
    const [on, setOn] = useState(isDebugEnabled);
    const toggle = () => { const next = !on; setDebugEnabled(next); setOn(next); };
    return html`
        <${TableRow} className=${`plugin-row core-log-row ${on ? 'status-debug-on' : 'status-debug-off'}`}
            index=${index} selected=${selected} onSelect=${onSelect}
            onActivate=${toggle}>
            <div class="plugin-info">
                <div class="plugin-copy">
                    <div class="plugin-title-row"><span class="plugin-name">Frontend Debug</span></div>
                    <span class="plugin-path">Console logging for UI navigation, focus, surface</span>
                </div>
            </div>
            <div class="plugin-action-column">
                <button type="button" class="plugin-action-zone" onClick=${toggle} tabIndex="-1"
                    aria-label=${on ? 'Disable frontend debug' : 'Enable frontend debug'}>
                    <span class="debug-toggle-label">${on ? 'ON' : 'OFF'}</span>
                </button>
            </div>
        <//>
    `;
}

export function CoreLogSection({ ctrl }) {
    const sel = useListSelection();
    return html`
        <section class="dev-section">
            <div class="section-header"><h2>Logs</h2></div>
            <div class="plugin-list-container">
                <div class="plugin-list table-list" onFocusOut=${(e) => {
                    if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) sel.deselect();
                }}>
                    ${CORE_SECTIONS.map((s, i) => html`<${CoreLogRow} key=${s.id} section=${s} ctrl=${ctrl}
                        index=${i} selected=${sel.selected(i)} onSelect=${sel.select} />`)}
                    <${FrontendDebugToggle} index=${CORE_SECTIONS.length} selected=${sel.selected(CORE_SECTIONS.length)} onSelect=${sel.select} />
                </div>
            </div>
        </section>
    `;
}
