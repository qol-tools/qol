import { html } from '../../lib/html.js';
import { Table, TableHeader, TableCell } from '../../lib/components/TableRow.js';
import { HotkeyRow } from '../../components/domain-rows/HotkeyRow.js';

function getActionLabel(plugin, actionId) {
    if (!plugin) return actionId;
    const action = plugin.actions?.find(a => a.id === actionId);
    return action ? action.label : actionId;
}

export function HotkeysList({ hotkeys, plugins, selectedIndex, onSelect, onEdit }) {
    if (hotkeys.length === 0) {
        return html`<${Table} className="hotkeys-list">
            <div class="empty">No hotkeys configured. Press <kbd>a</kbd> to add one.</div>
        <//>`;
    }
    return html`<${Table} className="hotkeys-list">
        <${TableHeader}>
            <${TableCell}>Shortcut<//>
            <${TableCell}>Plugin<//>
            <${TableCell}>Action<//>
        <//>
        ${hotkeys.map((hk, i) => {
            const plugin = plugins.find(p => p.id === hk.plugin_id);
            return html`
                <${HotkeyRow} key=${hk.id}
                    shortcut=${hk.key}
                    pluginName=${plugin?.name || hk.plugin_id}
                    actionLabel=${getActionLabel(plugin, hk.action)}
                    status=${plugin?.status || 'installed'}
                    index=${i} selected=${i === selectedIndex} onSelect=${onSelect}
                    data-dive-target="hotkeys-editor"
                    onActivate=${() => { if (i !== selectedIndex) onSelect(i); onEdit(hk); }} />
            `;
        })}
    <//>`;
}
