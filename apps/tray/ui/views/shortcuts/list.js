import { html } from '../../lib/html.js';
import { Table, TableHeader, TableCell } from '../../lib/components/TableRow.js';
import { ShortcutRow } from '../../components/domain-rows/ShortcutRow.js';
import { openShortcutsFile } from '../../api/config-files.js';
import { toast } from '../../lib/toast.js';
import { TYPE_LABELS, actionSummary, isManagedPluginShortcut } from './action.js';

export function ShortcutsList({ shortcuts, selectedIndex, onSelect, onEdit, onRun }) {
    if (shortcuts.length === 0) {
        return html`<${Table} className="shortcuts-list">
            <div class="empty">No shortcuts configured. Press <kbd>a</kbd> to add one.</div>
        <//>`;
    }
    return html`<${Table} className="shortcuts-list">
        <${TableHeader}>
            <${TableCell}>Name<//>
            <${TableCell}>Type<//>
            <${TableCell}>Target<//>
            <${TableCell}>Launcher<//>
        <//>
        ${shortcuts.map((s, i) => html`
            <${ShortcutRow} key=${s.id}
                name=${s.name || s.id}
                type=${TYPE_LABELS[s.action.type] || s.action.type}
                target=${actionSummary(s.action)}
                launcher=${s.export_to_launcher}
                enabled=${s.enabled}
                selectValue=${s.id}
                index=${i} selected=${i === selectedIndex} onSelect=${onSelect}
                data-dive-target=${isManagedPluginShortcut(s) ? undefined : 'shortcuts-editor'}
                data-secondary-label="Open shortcuts.json"
                onActivate=${() => {
                    if (i !== selectedIndex) onSelect(s.id);
                    if (isManagedPluginShortcut(s)) { onRun?.(s.id); return; }
                    onEdit(s);
                }}
                onSecondaryActivate=${() => {
                    openShortcutsFile().catch((err) => toast('error', `Failed to open shortcuts file: ${err.message}`));
                }} />
        `)}
    <//>`;
}
