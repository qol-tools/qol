import { toast } from './toast.js';

const NULL_ACTION = Object.freeze({ kind: 'null', label: '', run: () => {}, isNoop: true });

function copyTextAction(text, { label = 'Copy', message = 'Copied to clipboard' } = {}) {
    return {
        kind: 'copy-text',
        label,
        run: async () => {
            await navigator.clipboard.writeText(text);
            toast('success', message);
        },
    };
}

function modifierIndex(event = {}) {
    const ctrl = Boolean(event.ctrlKey || event.metaKey);
    const shift = Boolean(event.shiftKey);
    if (ctrl && shift) return 3;
    if (ctrl) return 2;
    if (shift) return 1;
    return 0;
}

function pickAction(actions, event) {
    if (!Array.isArray(actions) || actions.length === 0) return NULL_ACTION;
    const idx = modifierIndex(event);
    const direct = actions[idx];
    if (direct) return direct;
    return actions[0] || NULL_ACTION;
}

export function secondaryActionFrom(actions) {
    return Array.isArray(actions) && actions.length > 1 ? actions[1] : null;
}
