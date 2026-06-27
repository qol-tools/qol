export const SURFACE_FOCUS_TARGET = '[data-surface-focus-target]';
export const SURFACE_FOCUS_RETURN = '[data-surface-focus-return]';
export const SURFACE_CONTROL_COMMIT = 'surface-control-commit';

export function focusSurfaceTarget(target) {
    const focusTarget = target?.querySelector?.(SURFACE_FOCUS_TARGET);
    if (typeof focusTarget?.focus !== 'function') return false;
    focusTarget.focus({ preventScroll: true });
    return true;
}

export function finishSurfaceFocusTarget(target) {
    if (!isSurfaceFocusTarget(target)) return false;
    dispatchSurfaceCommit(target);
    const returnTarget = surfaceFocusReturnTarget(target);
    if (typeof returnTarget?.focus !== 'function') return false;
    returnTarget.focus({ preventScroll: true });
    return true;
}

export function surfaceFocusReturnTarget(target) {
    const explicit = target?.closest?.(SURFACE_FOCUS_RETURN);
    if (explicit && explicit !== target) return explicit;
    const parentSurface = target?.parentElement?.closest?.('[data-selected-surface]');
    if (parentSurface) return parentSurface;
    const surface = target?.closest?.('[data-selected-surface]');
    return surface && surface !== target ? surface : null;
}

function isSurfaceFocusTarget(target) {
    return target?.matches?.(SURFACE_FOCUS_TARGET) === true;
}

function dispatchSurfaceCommit(target) {
    if (typeof target?.dispatchEvent !== 'function') return;
    target.dispatchEvent(new CustomEvent(SURFACE_CONTROL_COMMIT, {
        bubbles: true,
        cancelable: true,
    }));
}
