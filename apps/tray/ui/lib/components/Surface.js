import { html } from '../html.js';
import { useRef } from 'preact/hooks';
import { secondaryActionFrom } from '../surface-actions.js';
import { diveFromSurface } from '../world-navigation-singleton.js';
import { handleSurfaceClick } from './surface-click.js';

export function useSurface({
    index, selected, onSelect, onActivate, onSecondaryActivate, actions,
    selectValue, priority, motion,
} = {}) {
    const focusValue = selectValue !== undefined ? selectValue : index;
    const secondary = secondaryActionFrom(actions);
    const secondaryLabel = secondary?.label || (onSecondaryActivate ? 'Secondary' : undefined);
    return {
        attrs: {
            'data-selected-surface': '',
            'data-selected': selected != null ? (selected ? 'true' : 'false') : undefined,
            'data-index': index != null ? String(index) : undefined,
            'data-selected-surface-priority': priority != null ? String(priority) : undefined,
            'data-selected-surface-motion': motion || undefined,
            'data-secondary-label': secondaryLabel,
            tabIndex: -1,
            onFocus: onSelect ? () => onSelect(focusValue) : undefined,
            onClick: (e) => handleSurfaceClick(
                { actions, onActivate, onSecondaryActivate },
                e,
                { diveFromSurface },
            ),
        },
    };
}

export function useInputSurface(opts) {
    const ref = useRef(null);
    const surface = useSurface(opts);
    return { ref, ...surface };
}

export function Surface({
    as = 'div', index, selected, onSelect, onActivate, onSecondaryActivate, actions,
    selectValue, className, children, ...rest
}) {
    const { attrs } = useSurface({
        index, selected, onSelect, onActivate, onSecondaryActivate, actions, selectValue,
    });
    return html`
        <${as} class=${className} ...${attrs} ...${rest}>
            ${children}
        <//>
    `;
}
