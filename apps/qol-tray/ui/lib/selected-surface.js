const SURFACE_SELECTOR = '[data-selected-surface]';

export function findActiveSelectedSurface({ currentTarget = null, includeFocus = true } = {}) {
    let bestSurface = null;
    let bestPriority = Number.NEGATIVE_INFINITY;
    let bestState = 0;

    for (const surface of document.querySelectorAll(SURFACE_SELECTOR)) {
        if (!isVisibleSurface(surface)) continue;

        const state = selectedSurfaceState(surface, includeFocus);
        if (state === 0) continue;

        const priority = selectedSurfacePriority(surface);
        if (priority < bestPriority) continue;
        if (priority === bestPriority && state <= bestState) continue;

        bestSurface = surface;
        bestPriority = priority;
        bestState = state;
    }

    if (bestSurface) return bestSurface;
    if (isVisibleSurface(currentTarget)) return currentTarget;
    return null;
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
