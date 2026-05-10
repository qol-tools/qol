import { html } from '../html.js';

export function KeyLegend({ bindings }) {
    if (!bindings?.length) return null;
    return html`
        <div class="key-legend" aria-hidden="true">
            ${bindings.map(b => html`
                <span class="key-legend-item" key=${b.action}>
                    <kbd>${b.key}</kbd><span class="key-legend-label">${b.label}</span>
                </span>
            `)}
        </div>
    `;
}
