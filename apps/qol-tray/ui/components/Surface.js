import { html } from '../lib/html.js';
import { useRef } from 'preact/hooks';

export function useSurface({ index, selected, onSelect, onActivate } = {}) {
    return {
        attrs: {
            'data-selected-surface': '',
            'data-selected': selected != null ? (selected ? 'true' : 'false') : undefined,
            'data-index': index != null ? String(index) : undefined,
            onFocus: onSelect ? () => onSelect(index) : undefined,
            onClick: onActivate,
        },
    };
}

export function useInputSurface(opts) {
    const ref = useRef(null);
    const surface = useSurface(opts);
    return { ref, ...surface };
}

export function Surface({ as = 'div', index, selected, onSelect, onActivate, className, children, ...rest }) {
    const { attrs } = useSurface({ index, selected, onSelect, onActivate });
    return html`
        <${as} class=${className} ...${attrs} ...${rest}>
            ${children}
        <//>
    `;
}
