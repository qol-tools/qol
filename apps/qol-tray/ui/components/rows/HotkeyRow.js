import { html } from '../../lib/html.js';
import { TableRow, TableCell } from '../TableRow.js';

export function HotkeyRow({ shortcut, pluginName, actionLabel, index, selected, onSelect, onActivate, ...rest }) {
    return html`
        <${TableRow} index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onActivate} ...${rest}>
            <${TableCell}><kbd>${shortcut}</kbd><//>
            <${TableCell}>${pluginName}<//>
            <${TableCell}>${actionLabel}<//>
        <//>
    `;
}
