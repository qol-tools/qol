import { html } from '../lib/html.js';

export function Surface({ as = 'div', index, selected, onSelect, onActivate, className, children, ...rest }) {
    return html`
        <${as} class=${className}
            data-selected-surface=""
            data-selected=${selected != null ? (selected ? 'true' : 'false') : undefined}
            data-index=${index != null ? String(index) : undefined}
            onFocus=${onSelect ? () => onSelect(index) : undefined}
            onClick=${onActivate}
            ...${rest}>
            ${children}
        <//>
    `;
}
