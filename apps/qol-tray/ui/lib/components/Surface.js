import { html } from '../html.js';
import { useRef } from 'preact/hooks';
import { pickAction, secondaryActionFrom } from '../surface-actions.js';

const INTERACTIVE_DESCENDANT = 'input, textarea, select, button, a, label, [contenteditable="true"]';

function resolveActivation({ actions, onActivate, onSecondaryActivate }, event) {
    if (Array.isArray(actions) && actions.length > 0) {
        const action = pickAction(actions, event);
        return action && !action.isNoop ? () => action.run(event) : null;
    }
    if (event.shiftKey && onSecondaryActivate) return () => onSecondaryActivate(event);
    if (onActivate) return () => onActivate(event);
    return null;
}

export function useSurface({
    index, selected, onSelect, onActivate, onSecondaryActivate, actions,
    selectValue, priority, motion,
} = {}) {
    const focusValue = selectValue !== undefined ? selectValue : index;
    const hasActivation = Boolean(onActivate || onSecondaryActivate || (Array.isArray(actions) && actions.length > 0));
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
            onClick: hasActivation ? (e) => {
                if (e.target !== e.currentTarget) {
                    const inner = e.target.closest?.(INTERACTIVE_DESCENDANT);
                    if (inner && inner !== e.currentTarget) return;
                }
                const run = resolveActivation({ actions, onActivate, onSecondaryActivate }, e);
                if (run) run();
            } : undefined,
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
