const SURFACE_SELECTOR = '[data-selected-surface]';

export function findActiveSelectedSurface({ currentTarget = null, includeFocus = true } = {}) {
    if (includeFocus) {
        const focused = focusedSurfaceTarget();
        if (focused) return focused;
    }

    let bestSurface = null;
    let bestPriority = Number.NEGATIVE_INFINITY;

    for (const surface of document.querySelectorAll(SURFACE_SELECTOR)) {
        if (!isVisibleSurface(surface)) continue;
        if (surface.getAttribute('data-selected') !== 'true') continue;

        const priority = selectedSurfacePriority(surface);
        if (priority < bestPriority) continue;

        bestSurface = surface;
        bestPriority = priority;
    }

    if (bestSurface) return bestSurface;
    if (isVisibleSurface(currentTarget)) return currentTarget;
    return null;
}

function focusedSurfaceTarget() {
    const focused = document.activeElement;
    if (!(focused instanceof HTMLElement) || focused === document.body) return null;

    const surface = focused.closest(SURFACE_SELECTOR);
    if (!surface || !isVisibleSurface(surface)) return null;

    const override = highPriorityChildSurface(surface);
    if (override) return override;

    const wedgeRoot = focused.closest('[data-wedge-root]');
    if (wedgeRoot && surface.contains(wedgeRoot) && isVisibleSurface(wedgeRoot)) return wedgeRoot;

    if (focused !== surface && isVisibleSurface(focused)) return focused;
    return surface;
}

function highPriorityChildSurface(parent) {
    const basePriority = selectedSurfacePriority(parent);
    let best = null;
    let bestPriority = basePriority;
    for (const child of parent.querySelectorAll(SURFACE_SELECTOR)) {
        if (child.getAttribute('data-selected') !== 'true') continue;
        if (!isVisibleSurface(child)) continue;
        const p = selectedSurfacePriority(child);
        if (p <= bestPriority) continue;
        best = child;
        bestPriority = p;
    }
    return best;
}

export function isVisibleSurface(surface) {
    if (!(surface instanceof HTMLElement)) return false;
    if (!surface.isConnected) return false;
    return surface.getClientRects().length > 0;
}

export function hasSelectedSurfaceState(surface, includeFocus = true) {
    if (!(surface instanceof HTMLElement)) return false;
    return selectedSurfaceState(surface, includeFocus) > 0;
}

function selectedSurfaceState(surface, includeFocus) {
    if (includeFocus && surface.matches(':focus-within')) return 2;
    if (surface.getAttribute('data-selected') === 'true') return 1;
    return 0;
}

function selectedSurfacePriority(surface) {
    const value = Number(surface.getAttribute('data-selected-surface-priority'));
    if (Number.isFinite(value)) return value;
    return 0;
}
