import { html } from '../lib/html.js';
import { IconChevron } from '../assets/icon-chevron.js';
import { Surface } from './Surface.js';

export function Expander({ open, onToggle, className, children, ...rest }) {
    const cls = ['btn btn-ghost btn-expander', className].filter(Boolean).join(' ');
    return html`
        <${Surface} className=${cls} ...${rest}
            aria-expanded=${open ? 'true' : 'false'}
            onActivate=${(e) => {
                if (e.target.closest('.btn-expander-body')) return;
                onToggle();
            }}>
            ${children}
        <//>
    `;
}

export function ExpanderTrigger({ children }) {
    return html`<div class="btn-expander-trigger"><span class="btn-icon btn-icon-chevron"><${IconChevron} size=${11} /></span>${children}</div>`;
}

export function ExpanderBody({ children }) {
    return html`<div class="btn-expander-body" onClick=${(e) => e.stopPropagation()}>${children}</div>`;
}
