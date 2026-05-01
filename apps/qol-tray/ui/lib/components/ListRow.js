import { html } from '../html.js';
import { Surface } from './Surface.js';

export function ListGroup({ className, onDeselect, children, ...rest }) {
    const cls = ['list-group', className].filter(Boolean).join(' ');
    const onFocusOut = onDeselect ? (e) => {
        if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) onDeselect();
    } : undefined;
    return html`<div class=${cls} onFocusOut=${onFocusOut} ...${rest}>${children}</div>`;
}

export function ListRow({ accent, action, className, children, ...rest }) {
    const hasAction = action != null;
    const cls = ['list-row', hasAction && 'list-row-with-action', className].filter(Boolean).join(' ');
    if (!hasAction) {
        return html`<${Surface} className=${cls} data-accent=${accent} ...${rest}>${children}<//>`;
    }
    return html`
        <${Surface} className=${cls} data-accent=${accent} ...${rest}>
            <div class="list-row-content">${children}</div>
            <div class="list-row-action">${action}</div>
        <//>
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
