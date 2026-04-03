import { html } from '../lib/html.js';
import { Surface } from './Surface.js';

export function CardGrid({ className, onDeselect, children, ...rest }) {
    const cls = ['card-grid', className].filter(Boolean).join(' ');
    const onFocusOut = onDeselect ? (e) => {
        if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) onDeselect();
    } : undefined;
    return html`<div class=${cls} onFocusOut=${onFocusOut} ...${rest}>${children}</div>`;
}

export function Card({ className, children, ...rest }) {
    const cls = ['card', className].filter(Boolean).join(' ');
    return html`<${Surface} className=${cls} ...${rest}>${children}<//>`;
}
