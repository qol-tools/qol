import { html } from '../../lib/html.js';

function getActionLabel(plugin, actionId) {
    if (!plugin) return actionId;
    const action = plugin.actions?.find(a => a.id === actionId);
    return action ? action.label : actionId;
}

export function HotkeysList({ hotkeys, plugins, selectedIndex, onSelect, onEdit }) {
    if (hotkeys.length === 0) {
        return html`<div class="hotkeys-list table-list">
            <div class="empty">No hotkeys configured. Press <kbd>a</kbd> to add one.</div>
        </div>`;
    }
    return html`<div class="hotkeys-list table-list">
        <div class="hotkey-header table-list-header table-grid">
            <span class="col-key table-cell">Shortcut</span>
            <span class="col-plugin table-cell">Plugin</span>
            <span class="col-action table-cell">Action</span>
        </div>
        ${hotkeys.map((hk, i) => html`
            <${HotkeyRow} key=${hk.id} hk=${hk} index=${i} plugin=${plugins.find(p => p.id === hk.plugin_id)}
                selected=${i === selectedIndex}
                onClick=${() => i !== selectedIndex ? onSelect(i) : onEdit(hk)} />
        `)}
    </div>`;
}

function HotkeyRow({ hk, plugin, index, selected, onClick }) {
    return html`
        <div class="hotkey-row table-list-row table-grid"
             data-selected-surface=""
             data-status="${plugin?.status || 'installed'}"
             data-selected="${selected ? 'true' : 'false'}"
             data-index="${index}" onClick=${onClick}>
            <span class="col-key table-cell" data-selected-text=""><kbd>${hk.key}</kbd></span>
            <span class="col-plugin table-cell" data-selected-text="">${plugin?.name || hk.plugin_id}</span>
            <span class="col-action table-cell" data-selected-text="">${getActionLabel(plugin, hk.action)}</span>
        </div>
    `;
}
