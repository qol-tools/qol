import { html } from '../../../lib/html.js';
import { Table } from '../../../lib/components/TableRow.js';
import { DevPluginRow } from '../../../components/rows/DevPluginRow.js';
import { useListSelection } from '../../../lib/hooks/useListSelection.js';

const CORE_SECTIONS = [
    { id: 'runtime', name: 'Runtime', description: 'Socket, state, polling' },
    { id: 'plugins', name: 'Plugins', description: 'Daemon lifecycle, loading' },
    { id: 'core', name: 'Core', description: 'Tray, hotkeys, menu, updates' },
    { id: 'frontend-debug', name: 'Frontend Debug', description: 'Console logging for UI navigation, focus, surface' }
];

function CoreLogRow({ section, ctrl, index, selected, onSelect }) {
    const control = ctrl.coreLogControls[section.id] || {};
    const muted = !!control.muted;
    const patterns = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    const actions = [
        { label: muted ? 'Unmute Logs' : 'Mute Logs', run: () => ctrl.toggleCoreLogs(section.id) },
        { label: patterns.length > 0 ? `Edit Filters (${patterns.length})` : 'Edit Filters', run: () => ctrl.editCoreLogFilters(section.id) },
    ];
    return html`
        <${DevPluginRow}
            name=${section.name}
            path=${section.description}
            status="linked"
            index=${index}
            selected=${selected}
            onSelect=${onSelect}
            actions=${actions}
            badges=${muted && html`<div class="plugin-status-badges"><span class="status-badge badge-muted">Muted</span></div>`}
            className="core-log-row"
            data-core-section=${section.id}
        />
    `;
}

export function CoreLogSection({ ctrl }) {
    const sel = useListSelection();
    return html`
        <section class="dev-section">
            <div class="section-header"><h2>Logs</h2></div>
            <div class="plugin-list-container">
                <${Table} className="plugin-list" onDeselect=${sel.deselect}>
                    ${CORE_SECTIONS.map((s, i) => html`<${CoreLogRow} key=${s.id} section=${s} ctrl=${ctrl}
                        index=${i} selected=${sel.selected(i)} onSelect=${sel.select} />`)}
                <//>
            </div>
        </section>
    `;
}
