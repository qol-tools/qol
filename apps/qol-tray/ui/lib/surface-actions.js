import { toast } from './toast.js';

const NULL_ACTION = Object.freeze({ kind: 'null', label: '', run: () => {}, isNoop: true });

export function diveAction(target, dive, { label = 'Open' } = {}) {
    return { kind: 'dive', label, run: () => dive?.(target) };
}

export function openExternalAction(invoke, { label = 'Open in editor' } = {}) {
    return { kind: 'open-external', label, run: invoke };
}

export function revealInFolderAction(invoke, { label = 'Reveal in folder' } = {}) {
    return { kind: 'reveal-in-folder', label, run: invoke };
}

export function copyTextAction(text, { label = 'Copy', message = 'Copied to clipboard' } = {}) {
    return {
        kind: 'copy-text',
        label,
        run: async () => {
            await navigator.clipboard.writeText(text);
            toast('success', message);
        },
    };
}

export function copyPathAction(path, { label = 'Copy path' } = {}) {
    return copyTextAction(path, { label, message: 'Path copied' });
}

export function customAction(run, { label = 'Activate', kind = 'custom' } = {}) {
    return { kind, label, run };
}

export function modifierIndex(event = {}) {
    const ctrl = Boolean(event.ctrlKey || event.metaKey);
    const shift = Boolean(event.shiftKey);
    if (ctrl && shift) return 3;
    if (ctrl) return 2;
    if (shift) return 1;
    return 0;
}

export function pickAction(actions, event) {
    if (!Array.isArray(actions) || actions.length === 0) return NULL_ACTION;
    const idx = modifierIndex(event);
    const direct = actions[idx];
    if (direct) return direct;
    return actions[0] || NULL_ACTION;
}

export function runActions(actions, event) {
    const action = pickAction(actions, event);
    if (action && !action.isNoop) action.run(event);
    return action;
}

export function secondaryActionFrom(actions) {
    return Array.isArray(actions) && actions.length > 1 ? actions[1] : null;
}
