import { isVisible, directSurfaces } from './surface-traits.js';
import { findActiveSelectedSurface } from './selected-surface.js';
import { createDebug, elLabel, rectLabel } from './debug.js';

const log = createDebug('qol:surfaces');

const SURFACE_SELECTOR = '[data-selected-surface]';
const CONTAINER_SELECTOR = '[data-surface-container]';

const INTERACTIVE_SELECTOR = [
    '[role=slider]', '[role=switch]', '[role=checkbox]', '[role=radio]',
    '[role=button]', '[role=tab]', '[data-slider-thumb]',
    'button', 'input:not([type=hidden])', 'select', 'textarea',
].join(',');

const SURFACE_STATUS = {
    reachable: 'reachable',
    invisible: 'invisible',
    disabled: 'disabled',
    inert: 'inert',
    shadowed: 'shadowed',
};

export function surfaceStatus({ visible, disabled, inert, shadowed }) {
    if (!visible) return SURFACE_STATUS.invisible;
    if (disabled) return SURFACE_STATUS.disabled;
    if (inert) return SURFACE_STATUS.inert;
    if (shadowed) return SURFACE_STATUS.shadowed;
    return SURFACE_STATUS.reachable;
}

export function effectiveReachable(status, parentReachable) {
    if (status === SURFACE_STATUS.reachable) return true;
    if (status === SURFACE_STATUS.shadowed) return parentReachable === true;
    return false;
}

export function classifyInteractable({ hasSurface, reachable }) {
    if (!hasSurface) return 'orphan';
    if (!reachable) return 'unreachable';
    return 'ok';
}

function containerOf(el) {
    return el.closest(CONTAINER_SELECTOR);
}

function parentSurfaceOf(el) {
    return el.parentElement?.closest(SURFACE_SELECTOR) || null;
}

function isShadowed(el) {
    const parent = parentSurfaceOf(el);
    return !!(parent && parent.closest(CONTAINER_SELECTOR) === containerOf(el));
}

function statusOf(el) {
    return surfaceStatus({
        visible: isVisible(el),
        disabled: !!el.disabled,
        inert: !!el.closest('[inert]'),
        shadowed: isShadowed(el),
    });
}

function reachabilityResolver() {
    const cache = new Map();
    const resolve = (el) => {
        if (!(el instanceof HTMLElement)) return false;
        if (cache.has(el)) return cache.get(el);
        cache.set(el, false);
        const status = statusOf(el);
        const parent = status === SURFACE_STATUS.shadowed ? parentSurfaceOf(el) : null;
        const result = effectiveReachable(status, parent ? resolve(parent) : false);
        cache.set(el, result);
        return result;
    };
    return resolve;
}

function auditSurfaces(doc = document) {
    const resolve = reachabilityResolver();

    const surfaces = Array.from(doc.querySelectorAll(SURFACE_SELECTOR)).map(el => ({
        label: elLabel(el),
        container: elLabel(containerOf(el)),
        status: statusOf(el),
        reachable: resolve(el),
        priority: Number(el.getAttribute('data-selected-surface-priority')) || 0,
        selected: el.getAttribute('data-selected') === 'true',
        rect: rectLabel(el.getBoundingClientRect()),
        el,
    }));

    const interactables = [];
    const problems = [];
    for (const el of doc.querySelectorAll(INTERACTIVE_SELECTOR)) {
        if (!isVisible(el)) continue;
        if (el.closest('[inert]')) continue;
        const surface = el.closest(SURFACE_SELECTOR);
        const reachable = surface ? resolve(surface) : false;
        const verdict = classifyInteractable({ hasSurface: !!surface, reachable });
        interactables.push({ label: elLabel(el), surface: elLabel(surface), reachable, verdict, rect: rectLabel(el.getBoundingClientRect()), el });
        if (verdict !== 'ok') problems.push({ kind: verdict, label: elLabel(el), surface: elLabel(surface) });
    }

    return { surfaces, interactables, problems, ok: problems.length === 0 };
}

function logSurfaceAudit(report = auditSurfaces()) {
    log(report.ok ? '✓ all interactables reachable' : `✗ ${report.problems.length} problem(s)`,
        `| surfaces: ${report.surfaces.length} | interactables: ${report.interactables.length}`);
    if (typeof console.table === 'function') {
        console.table(report.surfaces.map(({ label, container, status, reachable, priority, selected, rect }) =>
            ({ label, container, status, reachable, priority, selected, rect })));
        if (report.problems.length) console.table(report.problems);
    }
    return report;
}

function selectionState(doc = document) {
    const app = doc.querySelector('.app-container');
    const focused = doc.activeElement instanceof HTMLElement ? doc.activeElement : null;
    const container = (focused && focused.closest(CONTAINER_SELECTOR))
        || doc.querySelector('.plugin-config-detail')
        || null;
    const direct = container instanceof HTMLElement ? directSurfaces(container) : [];
    const resolved = findActiveSelectedSurface({});
    const sliders = Array.from(doc.querySelectorAll('.slider-control, .field-slider'))
        .filter(el => isVisible(el))
        .map(el => ({
            label: elLabel(el),
            isSurface: el.hasAttribute('data-selected-surface'),
            inDirectSet: direct.includes(el),
            surfaceAncestor: elLabel(el.closest(SURFACE_SELECTOR)),
        }));
    return {
        inputMode: app?.dataset.inputMode || null,
        focused: elLabel(focused),
        resolvedWedgeTarget: elLabel(resolved),
        activeContainer: elLabel(container),
        directSurfaces: direct.map(elLabel),
        visibleSliders: sliders,
    };
}

export function installSurfaceAudit(win = typeof window !== 'undefined' ? window : null, doc = typeof document !== 'undefined' ? document : null) {
    if (!win || !doc) return;
    win.qolAuditSurfaces = () => logSurfaceAudit(auditSurfaces(doc));
    win.qolSelectionState = () => { const state = selectionState(doc); log('selection', state); return state; };
}
