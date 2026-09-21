import { html } from '../../../lib/html.js';

export function FieldLabel({ text, description }) {
    const cls = text.length > 28 ? 'field-label field-label-tight'
        : text.length > 20 ? 'field-label field-label-compact'
        : 'field-label';
    return html`
        <div class="field-label-group">
            <div class=${cls} title=${text}>${text}</div>
            ${description && html`<div class="field-help">${description}</div>`}
        </div>
    `;
}
