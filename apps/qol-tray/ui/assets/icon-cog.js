import { html } from '../lib/html.js';

export function IconCog({ size = 14 }) {
    return html`
        <svg viewBox="0 0 16 16" width=${size} height=${size} fill="currentColor">
            <path d="M7 1h2l.3 1.8.8.3L11.7 2l1.4 1.4-1.1 1.6.3.8L14 6.1v2l-1.8.3-.3.8 1.1 1.6-1.4 1.4-1.6-1.1-.8.3L9 13H7l-.3-1.8-.8-.3-1.6 1.1-1.4-1.4 1.1-1.6-.3-.8L2 8.1v-2l1.8-.3.3-.8L3 3.4 4.4 2l1.6 1.1.8-.3L7 1zM8 5.5a2.5 2.5 0 100 5 2.5 2.5 0 000-5z"/>
        </svg>
    `;
}
