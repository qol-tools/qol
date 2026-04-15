import { html } from '../html.js';

const DEFAULT_ICON = html`
    <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.5">
        <path d="M9 12h6M12 9v6M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z" />
    </svg>
`;

export function EmptyState({ icon, message, hint, className }) {
    const cls = ['empty-state-block', className].filter(Boolean).join(' ');
    return html`
        <div class=${cls}>
            <div class="empty-state-icon">${icon || DEFAULT_ICON}</div>
            <div class="empty-state-message">${message}</div>
            ${hint && html`<div class="empty-state-hint">${hint}</div>`}
        </div>
    `;
}
