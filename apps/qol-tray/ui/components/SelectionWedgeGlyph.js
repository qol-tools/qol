import { html } from '../lib/html.js';

export function SelectionWedgeGlyph({ className = '', depth = 0 }) {
    const classes = className ? `selection-wedge-icon ${className}` : 'selection-wedge-icon';

    return html`<div class=${classes} aria-hidden="true"></div>`;
}
