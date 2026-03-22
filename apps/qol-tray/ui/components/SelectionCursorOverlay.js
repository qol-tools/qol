import { html } from '../lib/html.js';
import { useLayoutEffect, useRef, useState } from 'preact/hooks';
import { findActiveSelectedSurface, hasSelectedSurfaceState } from '../lib/selected-surface.js';
import { SelectionWedgeGlyph } from './SelectionWedgeGlyph.js';

const ATTRIBUTES = ['data-selected', 'data-selected-surface', 'data-selected-surface-motion', 'data-selected-surface-priority'];
const MAX_GLIDE_DISTANCE = 120;

export function SelectionCursorOverlay() {
    const [style, setStyle] = useState(hiddenStyle());
    const [ready, setReady] = useState(false);
    const resizeObserverRef = useRef(null);
    const appRef = useRef(null);
    const targetRef = useRef(null);
    const rectRef = useRef(null);
    const readyFrameRef = useRef(0);

    useLayoutEffect(() => {
        const app = document.querySelector('.app-container');
        if (!(app instanceof HTMLElement)) return;
        appRef.current = app;

        const sync = () => {
            const nextTarget = findActiveSelectedSurface({ currentTarget: targetRef.current });
            trackTarget(nextTarget, sync, resizeObserverRef, appRef, targetRef);

            if (!(nextTarget instanceof HTMLElement)) {
                rectRef.current = null;
                setStyle(previous => hiddenStyle(previous));
                return;
            }

            revealSurface(nextTarget);
            const nextRect = nextTarget.getBoundingClientRect();
            const motion = selectedSurfaceMotion(nextTarget);
            const persistentTeleport = motion === 'teleport';
            const maxGlideDistance = selectedSurfaceMaxGlide(nextTarget);
            const viewportTeleport = needsViewportTeleport(nextTarget);
            const shouldTeleport = persistentTeleport || viewportTeleport || needsTeleport(rectRef.current, nextRect, maxGlideDistance);
            rectRef.current = nextRect;
            setStyle(cursorStyle(app, nextTarget, shouldTeleport));
        };

        const setInputMode = (mode) => { app.dataset.inputMode = mode; };
        const onKey = () => setInputMode('keyboard');
        const onMouse = () => setInputMode('mouse');
        setInputMode('keyboard');

        const observer = new MutationObserver((mutations) => {
            ensureSurfacesFocusable(mutations);
            sync();
        });
        observer.observe(document.body, {
            attributes: true,
            attributeFilter: ATTRIBUTES,
            childList: true,
            subtree: true,
        });
        ensureSurfacesFocusable();

        document.addEventListener('scroll', sync, true);
        document.addEventListener('focusin', sync, true);
        document.addEventListener('focusout', sync, true);
        document.addEventListener('keydown', onKey, true);
        document.addEventListener('mousedown', onMouse, true);
        document.addEventListener('mousemove', onMouse, { capture: true, passive: true });
        window.addEventListener('resize', sync);
        sync();
        readyFrameRef.current = requestAnimationFrame(() => {
            readyFrameRef.current = 0;
            setReady(true);
        });

        return () => {
            observer.disconnect();
            document.removeEventListener('scroll', sync, true);
            document.removeEventListener('focusin', sync, true);
            document.removeEventListener('focusout', sync, true);
            document.removeEventListener('keydown', onKey, true);
            document.removeEventListener('mousedown', onMouse, true);
            document.removeEventListener('mousemove', onMouse, true);
            window.removeEventListener('resize', sync);
            if (readyFrameRef.current) cancelAnimationFrame(readyFrameRef.current);
            if (!resizeObserverRef.current) return;
            resizeObserverRef.current.disconnect();
            resizeObserverRef.current = null;
        };
    }, []);

    return html`
        <div class="selection-cursor-overlay ${ready ? 'is-ready' : ''}" style=${style} aria-hidden="true">
            <${SelectionWedgeGlyph} />
        </div>
    `;
}

function trackTarget(target, sync, resizeObserverRef, appRef, targetRef) {
    if (targetRef.current === target) return;
    clearCursorTargets();
    targetRef.current = target;
    markCursorTarget(targetRef.current, true);

    if (!resizeObserverRef.current) {
        resizeObserverRef.current = new ResizeObserver(sync);
        if (appRef.current) resizeObserverRef.current.observe(appRef.current);
    }

    resizeObserverRef.current.disconnect();
    if (appRef.current) resizeObserverRef.current.observe(appRef.current);
    if (!(target instanceof HTMLElement)) return;
    resizeObserverRef.current.observe(target);
}

function markCursorTarget(target, active) {
    if (!(target instanceof HTMLElement)) return;
    if (!active) { target.removeAttribute('data-selection-cursor-active'); return; }
    if (!hasSelectedSurfaceState(target)) return;
    target.setAttribute('data-selection-cursor-active', 'true');
}

function clearCursorTargets() {
    for (const target of document.querySelectorAll('[data-selection-cursor-active="true"]')) {
        target.removeAttribute('data-selection-cursor-active');
    }
}

function cursorStyle(app, target, teleport) {
    const appRect = app.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    const vars = getComputedStyle(target);
    const restX = readVar(vars, '--selected-surface-wedge-rest-x', '0px');
    const restY = readVar(vars, '--selected-surface-wedge-rest-y', '0px');
    const gapX = readVar(vars, '--selected-surface-wedge-gap-x', '0px');
    const gapY = readVar(vars, '--selected-surface-wedge-gap-y', '0px');
    const top = readVar(vars, '--selected-surface-wedge-top', '-20px');
    const left = readVar(vars, '--selected-surface-wedge-left', '-24px');

    return {
        width: `${targetRect.width}px`,
        height: `${targetRect.height}px`,
        opacity: 1,
        transform: `translate(${targetRect.left - appRect.left}px, ${targetRect.top - appRect.top}px)`,
        transition: teleport ? 'none' : '',
        '--selection-wedge-z': readVar(vars, '--selected-surface-wedge-z', 'var(--z-selection-wedge)'),
        '--selection-wedge-size': readVar(vars, '--selected-surface-wedge-size', '22px'),
        '--selection-wedge-top': `calc(${restY} + ${gapY} + ${top})`,
        '--selection-wedge-left': `calc(${restX} + ${gapX} + ${left})`,
    };
}

function hiddenStyle(previous = null) {
    return {
        width: previous?.width || '0px',
        height: previous?.height || '0px',
        opacity: 0,
        transform: previous?.transform || 'translate(0px, 0px)',
        transition: 'none',
    };
}

function readVar(style, name, fallback) {
    return style.getPropertyValue(name).trim() || fallback;
}

function selectedSurfaceMotion(target) {
    return target.getAttribute('data-selected-surface-motion') || 'glide';
}

function selectedSurfaceMaxGlide(target) {
    const value = getComputedStyle(target).getPropertyValue('--selected-surface-wedge-max-glide').trim();
    const parsed = Number.parseFloat(value);
    if (Number.isFinite(parsed)) return parsed;
    return MAX_GLIDE_DISTANCE;
}

function revealSurface(target) {
    if (isFullyVisibleInScrollParent(target)) return;
    target.scrollIntoView({ behavior: 'auto', block: 'nearest', inline: 'nearest' });
}

function needsViewportTeleport(target) {
    if (target.getAttribute('data-scroll-follow-mode') !== 'nearest') return false;

    const scroller = findScrollParent(target);
    if (!(scroller instanceof HTMLElement)) return false;

    return !isFullyVisibleWithin(target, scroller);
}

function isFullyVisibleInScrollParent(target) {
    const scroller = findScrollParent(target);
    if (!(scroller instanceof HTMLElement)) return true;
    return isFullyVisibleWithin(target, scroller);
}

function isFullyVisibleWithin(target, scroller) {
    const scrollerRect = scroller.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    const inset = 2;

    if (targetRect.top < scrollerRect.top + inset) return false;
    if (targetRect.bottom > scrollerRect.bottom - inset) return false;
    return true;
}

function findScrollParent(target) {
    let current = target.parentElement;

    while (current && current !== document.body) {
        if (isScrollable(current)) return current;
        current = current.parentElement;
    }

    const root = document.scrollingElement;
    if (root instanceof HTMLElement) return root;
    return null;
}

function isScrollable(target) {
    const style = getComputedStyle(target);
    if (style.overflowY !== 'auto' && style.overflowY !== 'scroll') return false;
    return target.scrollHeight > target.clientHeight + 1;
}

function needsTeleport(prevRect, nextRect, maxGlideDistance) {
    if (!prevRect) return true;

    const dx = nextRect.left - prevRect.left;
    const dy = nextRect.top - prevRect.top;
    const distance = Math.hypot(dx, dy);
    return distance > maxGlideDistance;
}

function ensureSurfacesFocusable(mutations) {
    const targets = mutations
        ? collectNewSurfaces(mutations)
        : document.querySelectorAll('[data-selected-surface]');

    for (const el of targets) {
        if (el.tabIndex >= 0) continue;
        if (el.hasAttribute('tabindex')) continue;
        el.tabIndex = -1;
    }
}

function collectNewSurfaces(mutations) {
    const surfaces = [];
    for (const mutation of mutations) {
        if (mutation.type !== 'childList') continue;
        for (const node of mutation.addedNodes) {
            if (!(node instanceof HTMLElement)) continue;
            if (node.hasAttribute('data-selected-surface')) surfaces.push(node);
            for (const child of node.querySelectorAll('[data-selected-surface]')) {
                surfaces.push(child);
            }
        }
    }
    return surfaces;
}
