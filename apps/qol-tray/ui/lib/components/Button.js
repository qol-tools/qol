import { html } from '../html.js';
import { Surface } from './Surface.js';

export function Button({ variant, small, className, children, ...rest }) {
    const cls = ['btn', variant, small && 'btn-sm', className].filter(Boolean).join(' ');
    return html`<${Surface} as="button" className=${cls} ...${rest}>${children}<//>`;
}

export function RefreshButton({ spinning, className, ...rest }) {
    const cls = ['refresh-btn', spinning && 'spinning', className].filter(Boolean).join(' ');
    return html`<button class=${cls} disabled=${spinning} ...${rest}></button>`;
}
