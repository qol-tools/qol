const INTERACTIVE_DESCENDANT = 'input, textarea, select, label, [contenteditable="true"]';

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

function diveIfRequested(target, diveFromSurface) {
    if (!(target instanceof HTMLElement)) return false;
    const diveTarget = target.getAttribute('data-dive-target');
    if (!diveTarget) return false;
    target.setAttribute('data-dive-source', '');
    requestAnimationFrame(() => diveFromSurface(target));
    return true;
}

export function handleSurfaceClick(handlers, event, { diveFromSurface } = {}) {
    const { actions, onActivate, onSecondaryActivate } = handlers;
    if (event.target !== event.currentTarget) {
        const inner = event.target.closest?.(INTERACTIVE_DESCENDANT);
        if (inner && inner !== event.currentTarget) return;
    }
    const hasSecondary =
        Boolean(onSecondaryActivate) || (Array.isArray(actions) && actions.length > 1);
    if (event.shiftKey && hasSecondary) {
        if (runSecondary({ actions, onSecondaryActivate }, event)) event.stopPropagation?.();
        return;
    }
    const ran = runPrimary({ actions, onActivate }, event);
    const dove = diveFromSurface ? diveIfRequested(event.currentTarget, diveFromSurface) : false;
    if (ran || dove) event.stopPropagation?.();
}
