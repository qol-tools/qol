import { html } from '../../lib/html.js';
import { TableRow, TableCell } from '../../lib/components/TableRow.js';

export function HotkeyRow({ shortcut, pluginName, actionLabel, status, enabled = true, index, selected, onSelect, onActivate, className, ...rest }) {
    const cls = ['hotkey-row', !enabled && 'disabled', className].filter(Boolean).join(' ');
    return html`
        <${TableRow} className=${cls} index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onActivate}
            data-status=${status} ...${rest}>
            <${TableCell} className="col-key">
                <kbd>${shortcut}</kbd>
                ${!enabled && html`<span class="hotkey-disabled-badge">disabled</span>`}
            <//>
            <${TableCell}>${pluginName}<//>
            <${TableCell}>${actionLabel}<//>
        <//>
    `;
}
