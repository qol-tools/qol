import { html } from '../lib/html.js';

export function ListGroup({ className, onDeselect, children, ...rest }) {
    const cls = ['list-group', className].filter(Boolean).join(' ');
    const onFocusOut = onDeselect ? (e) => {
        if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) onDeselect();
    } : undefined;
    return html`<div class=${cls} onFocusOut=${onFocusOut} ...${rest}>${children}</div>`;
}

export function ListRow({ index, selected, onSelect, accent, onClick, className, children, ...rest }) {
    const cls = ['list-row', className].filter(Boolean).join(' ');
    return html`
        <div class=${cls}
            data-selected-surface=""
            data-selected=${selected ? 'true' : 'false'}
            data-index=${String(index)}
            data-accent=${accent}
            onFocus=${() => onSelect(index)}
            onClick=${onClick}
            ...${rest}>
            ${children}
        </div>
    `;
}

export function ListRowHeader({ className, children }) {
    const cls = ['list-row-header', className].filter(Boolean).join(' ');
    return html`<div class=${cls}>${children}</div>`;
}

export function ListRowBody({ className, children }) {
    const cls = ['list-row-body', className].filter(Boolean).join(' ');
    return html`<div class=${cls}>${children}</div>`;
}

export function ListRowTitle({ mono, className, children }) {
    const cls = ['list-row-title', mono && 'list-row-mono', className].filter(Boolean).join(' ');
    return html`<span class=${cls} data-selected-text="">${children}</span>`;
}

export function ListRowText({ mono, className, children }) {
    const cls = ['list-row-text', mono && 'list-row-mono', className].filter(Boolean).join(' ');
    return html`<span class=${cls} data-selected-text="">${children}</span>`;
}
