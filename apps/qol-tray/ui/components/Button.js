import { html } from '../lib/html.js';
import { Surface } from './Surface.js';

export function Button({ variant, small, className, children, ...rest }) {
    const cls = ['btn', variant, small && 'btn-sm', className].filter(Boolean).join(' ');
    return html`<${Surface} as="button" className=${cls} ...${rest}>${children}<//>`;
}

export function RefreshButton({ spinning, className, ...rest }) {
    if (spinning) return html`<button class="refresh-btn spinning" disabled></button>`;
    const cls = ['refresh-btn', className].filter(Boolean).join(' ');
    return html`<${Surface} as="button" className=${cls} ...${rest} />`;
}
