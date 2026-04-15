import { html } from '../../lib/html.js';
import { Table, TableHeader, TableCell } from '../../lib/components/TableRow.js';
import { ShortcutRow } from '../../components/domain-rows/ShortcutRow.js';

const TYPE_LABELS = { open_url: 'URL', launch_app: 'App' };

function actionSummary(action) {
    if (action.type === 'open_url') return action.url || '(no url)';
    if (action.type === 'launch_app') return appRefLabel(action.app);
    return '(unknown)';
}

function appRefLabel(appRef) {
    if (!appRef) return '';
    if (appRef.type === 'bundle_id') return appRef.id;
    if (appRef.type === 'path') return appRef.path;
    if (appRef.type === 'name') return appRef.name;
    return '';
}

export function ShortcutsList({ shortcuts, selectedIndex, onSelect, onEdit }) {
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
                data-dive-target="shortcuts-editor"
                onActivate=${() => { if (i !== selectedIndex) onSelect(s.id); onEdit(s); }} />
        `)}
    <//>`;
}
