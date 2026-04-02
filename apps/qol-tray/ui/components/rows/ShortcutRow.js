import { html } from '../../lib/html.js';
import { TableRow, TableCell } from '../TableRow.js';

export function ShortcutRow({ name, type, target, launcher, enabled, selectValue, index, selected, onSelect, onActivate, ...rest }) {
    return html`
        <${TableRow} index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onActivate}
            selectValue=${selectValue} data-enabled=${enabled ? 'true' : 'false'} ...${rest}>
            <${TableCell}>${name}<//>
            <${TableCell}>${type}<//>
            <${TableCell}>${target}<//>
            <${TableCell}>${launcher ? 'Yes' : 'No'}<//>
        <//>
    `;
}
