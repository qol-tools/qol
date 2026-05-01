import { html } from '../html.js';
import { useRef } from 'preact/hooks';
import { secondaryActionFrom } from '../surface-actions.js';
import { diveFromSurface } from '../world-navigation-singleton.js';

const INTERACTIVE_DESCENDANT = 'input, textarea, select, button, a, label, [contenteditable="true"]';

function runPrimary({ actions, onActivate }, event) {
    if (onActivate) { onActivate(event); return true; }
    if (Array.isArray(actions) && actions.length > 0 && !actions[0].isNoop) {
        actions[0].run(event); return true;
    }
    return false;
}

function runSecondary({ actions, onSecondaryActivate }, event) {
    if (Array.isArray(actions) && actions.length > 1 && !actions[1].isNoop) {
        actions[1].run(event); return true;
    }
    if (onSecondaryActivate) { onSecondaryActivate(event); return true; }
    return false;
}

function maybeDive(target) {
    if (!(target instanceof HTMLElement)) return;
    const diveTarget = target.getAttribute('data-dive-target');
    if (!diveTarget) return;
    target.setAttribute('data-dive-source', '');
    requestAnimationFrame(() => diveFromSurface(target));
}

function handleSurfaceClick({ actions, onActivate, onSecondaryActivate }, event) {
    if (event.target !== event.currentTarget) {
        const inner = event.target.closest?.(INTERACTIVE_DESCENDANT);
        if (inner && inner !== event.currentTarget) return;
    }
    const hasSecondary =
        Boolean(onSecondaryActivate) || (Array.isArray(actions) && actions.length > 1);
    if (event.shiftKey && hasSecondary) {
        runSecondary({ actions, onSecondaryActivate }, event);
        return;
    }
    runPrimary({ actions, onActivate }, event);
    maybeDive(event.currentTarget);
}

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
            onClick: (e) => handleSurfaceClick({ actions, onActivate, onSecondaryActivate }, e),
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
