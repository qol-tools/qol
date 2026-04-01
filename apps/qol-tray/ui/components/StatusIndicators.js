import { html } from '../lib/html.js';

export function Badge({ className, style, children }) {
    const cls = ['badge', className].filter(Boolean).join(' ');
    return html`<span class=${cls} style=${style}>${children}</span>`;
}

export function HealthDot({ health }) {
    return html`<span class="profile-health-dot" data-health=${health}></span>`;
}

export function Alert({ variant, children, ...rest }) {
    return html`<div class="profile-sync-alert" data-variant=${variant} ...${rest}>${children}</div>`;
}
