/**
 * Surface trait registry. Maps surface elements to Enter behaviors.
 *
 * Traits are checked in order — first match wins. New traits are added
 * via registerSurfaceTrait(). Built-in traits infer behavior from HTML
 * semantics: element type, role, and DOM structure.
 */

const traits = [];

export function registerSurfaceTrait(trait) {
    traits.push(trait);
}

export function activateSurface(el) {
    if (!(el instanceof HTMLElement)) return 'none';
    for (const trait of traits) {
        if (trait.test(el)) {
            trait.activate(el);
            return trait.id;
        }
    }
    el.click();
    return 'action';
}

export function surfaceContainsChildContainer(el) {
    if (!(el instanceof HTMLElement)) return false;
    const child = el.querySelector('[data-surface-container]');
    return child !== null && child.getClientRects().length > 0;
}

export function surfaceDepth(el) {
    let depth = 0;
    let node = el;
    while (node) {
        node = node.parentElement?.closest('[data-surface-container]');
        if (!node) break;
        const base = Number(node.getAttribute('data-surface-depth-base'));
        if (base > 0) depth += base;
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
        .filter(el =>
            el.closest('[data-surface-container]') === container
            && el.getClientRects().length > 0
        );
}

export function firstChildContainer(container) {
    if (!(container instanceof HTMLElement)) return null;
    for (const child of container.querySelectorAll('[data-surface-container]')) {
        if (child.parentElement?.closest('[data-surface-container]') !== container) continue;
        if (child.getClientRects().length === 0) continue;
        if (directSurfaces(child).length === 0) continue;
        return child;
    }
    return null;
}

registerSurfaceTrait({
    id: 'container',
    test: surfaceContainsChildContainer,
    activate: (el) => el.click(),
});

registerSurfaceTrait({
    id: 'toggle',
    test: (el) =>
        el.getAttribute('role') === 'switch'
        || el.querySelector('[role="switch"], input[type="checkbox"]') !== null,
    activate: (el) => el.click(),
});

registerSurfaceTrait({
    id: 'input',
    test: (el) => {
        if (el.matches('input, select, textarea')) return true;
        return el.querySelector('input:not([type="hidden"]):not([type="checkbox"]), select, textarea, [contenteditable="true"]') !== null;
    },
    activate: (el) => {
        const target = el.matches('input, select, textarea')
            ? el
            : el.querySelector('input:not([type="hidden"]):not([type="checkbox"]), select, textarea, [contenteditable="true"]');
        if (target) { target.focus(); target.select?.(); }
    },
});

registerSurfaceTrait({
    id: 'link',
    test: (el) => el.matches('a[href]') || el.querySelector('a[href]') !== null,
    activate: (el) => {
        const link = el.matches('a[href]') ? el : el.querySelector('a[href]');
        if (link) link.click();
    },
});

registerSurfaceTrait({
    id: 'action',
    test: () => true,
    activate: (el) => el.click(),
});
