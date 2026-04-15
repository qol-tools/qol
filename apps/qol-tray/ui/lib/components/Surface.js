import { html } from '../html.js';
import { useRef } from 'preact/hooks';

export function useSurface({ index, selected, onSelect, onActivate, selectValue } = {}) {
    const focusValue = selectValue !== undefined ? selectValue : index;
    return {
        attrs: {
            'data-selected-surface': '',
            'data-selected': selected != null ? (selected ? 'true' : 'false') : undefined,
            'data-index': index != null ? String(index) : undefined,
            tabIndex: -1,
            onFocus: onSelect ? () => onSelect(focusValue) : undefined,
            onClick: onActivate,
        },
    };
}

export function useInputSurface(opts) {
    const ref = useRef(null);
    const surface = useSurface(opts);
    return { ref, ...surface };
}

export function Surface({ as = 'div', index, selected, onSelect, onActivate, selectValue, className, children, ...rest }) {
    const { attrs } = useSurface({ index, selected, onSelect, onActivate, selectValue });
    return html`
        <${as} class=${className} ...${attrs} ...${rest}>
            ${children}
        <//>
    `;
}
