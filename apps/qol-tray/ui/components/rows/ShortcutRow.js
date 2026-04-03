import { html } from '../../lib/html.js';
import { TableRow, TableCell } from '../TableRow.js';

export function ShortcutRow({ name, type, target, launcher, enabled, selectValue, index, selected, onSelect, onActivate, className, ...rest }) {
    const cls = ['shortcut-row', className].filter(Boolean).join(' ');
    return html`
        <${TableRow} className=${cls} index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onActivate}
            selectValue=${selectValue} data-enabled=${enabled ? 'true' : 'false'} ...${rest}>
            <${TableCell}>${name}<//>
            <${TableCell}>${type}<//>
            <${TableCell}>${target}<//>
            <${TableCell}>${launcher ? 'Yes' : 'No'}<//>
        <//>
    `;
}
