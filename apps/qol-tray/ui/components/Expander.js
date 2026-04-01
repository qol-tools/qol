import { html } from '../lib/html.js';
import { IconChevron } from '../assets/icon-chevron.js';

export function Expander({ open, onToggle, className, children, ...rest }) {
    const cls = ['btn btn-ghost btn-expander', className].filter(Boolean).join(' ');
    return html`
        <div class=${cls} data-selected-surface="" ...${rest}
            aria-expanded=${open ? 'true' : 'false'}
            onClick=${(e) => {
                if (e.target.closest('.btn-expander-body')) return;
                onToggle();
            }}>
            ${children}
        </div>
    `;
}

export function ExpanderTrigger({ children }) {
    return html`<div class="btn-expander-trigger"><span class="btn-icon btn-icon-chevron"><${IconChevron} size=${11} /></span>${children}</div>`;
}

export function ExpanderBody({ children }) {
    return html`<div class="btn-expander-body" onClick=${(e) => e.stopPropagation()}>${children}</div>`;
}
