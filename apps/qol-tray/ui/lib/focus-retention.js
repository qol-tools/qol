import { createDebug, elLabel } from './debug.js';
import { KEYBOARD_ISOLATION_SELECTOR } from './surface-traits.js';

const log = createDebug('qol:focus-retention');

const SURFACE_SELECTOR = '[data-selected-surface]';
const CONTAINER_SELECTOR = '[data-surface-container]';
const SLOT_SELECTOR = '.world-view-slot';
const VIEWPORT_SELECTOR = '#viewport';

export function pickFallbackSurface({ lostContainer, lostSlot, viewport }) {
    const candidate = pickSelectedThenFirst(lostContainer)
        || pickSelectedThenFirst(lostSlot)
        || pickAnchoredSlotSurface(viewport)
        || firstSurfaceInViewport(viewport);
    return candidate || null;
}

function pickSelectedThenFirst(root) {
    if (!isUsable(root)) return null;
    const surfaces = visibleSurfaces(root);
    if (surfaces.length === 0) return null;
    for (const el of surfaces) {
        if (el.getAttribute('data-selected') === 'true') return el;
    }
    return surfaces[0];
}

function pickAnchoredSlotSurface(viewport) {
    if (!isUsable(viewport)) return null;
    const vr = viewportRect(viewport);
    if (!vr) return null;
    const slots = typeof viewport.querySelectorAll === 'function'
        ? Array.from(viewport.querySelectorAll(SLOT_SELECTOR))
        : [];
    for (const slot of slots) {
        if (!isUsable(slot)) continue;
        if (!rectIntersects(boundingRect(slot), vr)) continue;
        const surfaces = visibleSurfaces(slot);
        if (surfaces.length === 0) continue;
        for (const el of surfaces) {
            if (el.getAttribute('data-selected') === 'true') return el;
        }
        return surfaces[0];
    }
    return null;
}

function firstSurfaceInViewport(viewport) {
    if (!isUsable(viewport)) return null;
    const vr = viewportRect(viewport);
    const surfaces = visibleSurfaces(viewport);
    if (!vr) return surfaces[0] || null;
    for (const el of surfaces) {
        if (rectIntersects(boundingRect(el), vr)) return el;
    }
    return null;
}

function visibleSurfaces(root) {
    if (!root || typeof root.querySelectorAll !== 'function') return [];
    return Array.from(root.querySelectorAll(SURFACE_SELECTOR)).filter(isSurfaceUsable);
}

function viewportRect(viewport) {
    return boundingRect(viewport);
}

function boundingRect(el) {
    if (!el || typeof el.getBoundingClientRect !== 'function') return null;
    const r = el.getBoundingClientRect();
    if (!r || (r.width === 0 && r.height === 0)) return null;
    return r;
}

function rectIntersects(a, b) {
    if (!a || !b) return false;
    return a.right > b.left && a.left < b.right && a.bottom > b.top && a.top < b.bottom;
}

function isSurfaceUsable(el) {
    if (!el) return false;
    if (typeof el.isConnected === 'boolean' && !el.isConnected) return false;
    if (el.disabled) return false;
    if (typeof el.closest === 'function' && el.closest('[inert]')) return false;
    if (typeof el.getClientRects === 'function') {
        const rects = el.getClientRects();
        if (rects && rects.length === 0) return false;
    }
    return true;
}

function isUsable(node) {
    if (!node) return false;
    if (typeof node.isConnected === 'boolean' && !node.isConnected) return false;
    return true;
}

export function createFocusRetention(doc = typeof document !== 'undefined' ? document : null) {
    if (!doc) return { dispose: () => {} };

    const tracked = { surface: null, container: null, slot: null, viewport: null };
    let pendingRaf = 0;
    let mutationDirty = false;

    const captureFromFocus = (target) => {
        if (!(target instanceof HTMLElement)) return;
        const surface = target.closest(SURFACE_SELECTOR);
        if (!surface) return;
        tracked.surface = surface;
        tracked.container = surface.closest(CONTAINER_SELECTOR);
        tracked.slot = surface.closest(SLOT_SELECTOR);
        tracked.viewport = doc.querySelector(VIEWPORT_SELECTOR);
    };

    const schedule = () => {
        if (pendingRaf) return;
        pendingRaf = requestAnimationFrame(() => {
            pendingRaf = 0;
            attemptRecovery();
        });
    };

    const attemptRecovery = () => {
        const active = doc.activeElement;
        if (active && active !== doc.body) return;
        const lost = tracked.surface;
        if (!lost) return;
        if (lost.isConnected) return;
        if (hasModalCapturingFocus(doc)) return;
        const fallback = pickFallbackSurface({
            lostContainer: tracked.container,
            lostSlot: tracked.slot,
            viewport: tracked.viewport || doc.querySelector(VIEWPORT_SELECTOR),
        });
        if (!fallback) {
            log('recover: no fallback', elLabel(lost));
            tracked.surface = null;
            return;
        }
        if (fallback === lost) return;
        log('recover:', elLabel(lost), '→', elLabel(fallback));
        fallback.focus({ preventScroll: true });
        tracked.surface = fallback;
        tracked.container = fallback.closest(CONTAINER_SELECTOR);
        tracked.slot = fallback.closest(SLOT_SELECTOR);
    };

    const onFocusIn = (event) => {
        captureFromFocus(event.target);
    };

    const onFocusOut = () => {
        schedule();
    };

    const observer = new MutationObserver(() => {
        mutationDirty = true;
        if (doc.activeElement === doc.body) schedule();
        else if (tracked.surface && !tracked.surface.isConnected) schedule();
    });

    doc.addEventListener('focusin', onFocusIn, true);
    doc.addEventListener('focusout', onFocusOut, true);
    observer.observe(doc.body, { childList: true, subtree: true });
    if (doc.activeElement && doc.activeElement !== doc.body) {
        captureFromFocus(doc.activeElement);
    }

    return {
        dispose() {
            doc.removeEventListener('focusin', onFocusIn, true);
            doc.removeEventListener('focusout', onFocusOut, true);
            observer.disconnect();
            if (pendingRaf) cancelAnimationFrame(pendingRaf);
        },
    };
}

function hasModalCapturingFocus(doc) {
    const modal = doc.querySelector(KEYBOARD_ISOLATION_SELECTOR);
    if (!modal) return false;
    const rects = typeof modal.getClientRects === 'function' ? modal.getClientRects() : null;
    return !rects || rects.length > 0;
}
