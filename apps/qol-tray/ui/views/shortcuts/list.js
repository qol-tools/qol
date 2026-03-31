import { html } from '../../lib/html.js';

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
        return html`<div class="shortcuts-list table-list">
            <div class="empty">No shortcuts configured. Press <kbd>a</kbd> to add one.</div>
        </div>`;
    }
    return html`<div class="shortcuts-list table-list">
        <div class="shortcut-header table-list-header table-grid">
            <span class="col-name table-cell">Name</span>
            <span class="col-type table-cell">Type</span>
            <span class="col-target table-cell">Target</span>
            <span class="col-launcher table-cell">Launcher</span>
        </div>
        ${shortcuts.map((s, i) => html`
            <${ShortcutRow} key=${s.id} shortcut=${s} index=${i}
                selected=${i === selectedIndex} onSelect=${onSelect}
                onClick=${() => i !== selectedIndex ? onSelect(s.id) : onEdit(s)} />
        `)}
    </div>`;
}

function ShortcutRow({ shortcut, index, selected, onSelect, onClick }) {
    return html`
        <div class="shortcut-row table-list-row table-grid"
             data-selected-surface=""
             data-enabled="${shortcut.enabled ? 'true' : 'false'}"
             data-selected="${selected ? 'true' : 'false'}"
             data-index="${index}" onFocus=${() => onSelect(shortcut.id)} onClick=${onClick}>
            <span class="col-name table-cell" data-selected-text="">${shortcut.name || shortcut.id}</span>
            <span class="col-type table-cell" data-selected-text="">${TYPE_LABELS[shortcut.action.type] || shortcut.action.type}</span>
            <span class="col-target table-cell" data-selected-text="">${actionSummary(shortcut.action)}</span>
            <span class="col-launcher table-cell" data-selected-text="">${shortcut.export_to_launcher ? 'Yes' : 'No'}</span>
        </div>
    `;
}
