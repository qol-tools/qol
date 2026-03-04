import { html } from '../lib/html.js';

export function PageHeader({ title, subtitle = '', actions = null, className = '' }) {
    const cls = className ? `page-header ${className}` : 'page-header';
    return html`
        <div class=${cls}>
            <div class="page-header-main">
                <h1>${title}</h1>
                <p>${subtitle}</p>
            </div>
            ${actions ? html`<div class="page-header-actions">${actions}</div>` : ''}
        </div>
    `;
}
