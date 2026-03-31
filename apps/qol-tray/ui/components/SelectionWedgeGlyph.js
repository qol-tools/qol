import { html } from '../lib/html.js';

export function SelectionWedgeGlyph({ className = '', depth = 0 }) {
    const classes = className ? `selection-wedge-icon ${className}` : 'selection-wedge-icon';

    return html`
        <div class=${classes} aria-hidden="true">
            ${depth > 1 && html`<span class="selection-wedge-depth">${depth}</span>`}
        </div>
    `;
}
