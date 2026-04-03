export const MODAL_SELECTOR = '.edit-modal, .confirm-modal';

export function isVisible(el) {
    return el.getClientRects().length > 0;
}

export function activateSurface(el) {
    if (el instanceof HTMLElement) el.click();
}

export function surfaceContainsChildContainer(el) {
    if (!(el instanceof HTMLElement)) return false;
    const child = el.querySelector('[data-surface-container]');
    return child !== null && isVisible(child);
}

export function surfaceDepth(el) {
    let depth = 0;
    let node = el;
    while (node) {
        node = node.parentElement?.closest('[data-surface-container]');
        if (!node) break;
        const base = Number(node.getAttribute('data-surface-depth-base'));
        if (base > 0) return depth + base + 1;
        depth++;
    }
    return depth;
}

export function activeContainer(el) {
    return el?.closest('[data-surface-container]') || null;
}

export function parentContainer(container) {
    return container?.parentElement?.closest('[data-surface-container]') || null;
}

export function directSurfaces(container) {
    if (!(container instanceof HTMLElement)) return [];
    return Array.from(container.querySelectorAll('[data-selected-surface]'))
        .filter(el => {
            if (el.closest('[data-surface-container]') !== container) return false;
            if (!isVisible(el)) return false;
            if (el.disabled) return false;
            const parentSurface = el.parentElement?.closest('[data-selected-surface]');
            if (parentSurface && parentSurface.closest('[data-surface-container]') === container) return false;
            return true;
        });
}

export function firstChildContainer(container) {
    if (!(container instanceof HTMLElement)) return null;
    for (const child of container.querySelectorAll('[data-surface-container]')) {
        if (child.parentElement?.closest('[data-surface-container]') !== container) continue;
        if (!isVisible(child)) continue;
        if (directSurfaces(child).length === 0) continue;
        return child;
    }
    return null;
}
