import { html } from '../lib/html.js';

export function SelectionWedgeGlyph({ className = '' }) {
    const classes = className ? `selection-wedge-icon ${className}` : 'selection-wedge-icon';

    return html`<div class=${classes} aria-hidden="true" />`;
}
