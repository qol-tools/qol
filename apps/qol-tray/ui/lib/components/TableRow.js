import { html } from '../html.js';
import { Surface } from './Surface.js';

export function Table({ columns, className, onDeselect, children, ...rest }) {
    const cls = ['table-list', className].filter(Boolean).join(' ');
    const colStyle = columns ? { '--table-cols': columns } : undefined;
    const onFocusOut = onDeselect ? (e) => {
        if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) onDeselect();
    } : undefined;
    return html`<div class=${cls} style=${colStyle} onFocusOut=${onFocusOut} ...${rest}>${children}</div>`;
}

export function TableHeader({ className, children }) {
    const cls = ['table-list-header table-grid', className].filter(Boolean).join(' ');
    return html`<div class=${cls}>${children}</div>`;
}

export function TableRow({ accent, className, children, ...rest }) {
    const cls = ['table-list-row table-grid', className].filter(Boolean).join(' ');
    return html`
        <${Surface} className=${cls} data-accent=${accent} ...${rest}>
            ${children}
        <//>
    `;
}

export function TableCell({ className, children, ...rest }) {
    const cls = ['table-cell', className].filter(Boolean).join(' ');
    return html`<span class=${cls} data-selected-text="" ...${rest}>${children}</span>`;
}
