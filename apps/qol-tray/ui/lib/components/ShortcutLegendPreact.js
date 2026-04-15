import { html } from '../html.js';

export function ShortcutLegend({ shortcuts }) {
    if (!shortcuts || shortcuts.length === 0) return null;
    return html`
        <div class="help">
            ${shortcuts.map(({ key, label }) =>
                html`<span key=${key} class="help-item"><kbd>${key}</kbd> ${label}</span>`
            )}
        </div>
    `;
}
