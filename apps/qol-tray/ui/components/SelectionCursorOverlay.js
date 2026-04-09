import { html } from '../lib/html.js';
import { useLayoutEffect, useRef, useState } from 'preact/hooks';
import { findActiveSelectedSurface, hasSelectedSurfaceState } from '../lib/selected-surface.js';
import { hslLuminance } from '../lib/color.js';
import { surfaceDepth } from '../lib/surface-traits.js';
import { SelectionWedgeGlyph } from './SelectionWedgeGlyph.js';
import { createDebug, elLabel, rectLabel } from '../lib/debug.js';
import { isCtrlHeld, subscribeCtrl } from '../lib/ctrl-state.js';
import { nearestSurfaceToCenter } from '../lib/viewport-spatial.js';

const log = createDebug('qol:wedge');

const ATTRIBUTES = ['data-selected', 'data-selected-surface', 'data-selected-surface-motion', 'data-selected-surface-priority'];
const WEDGE_HUE_BASE = 50;
const WEDGE_HUE_STEP = 45;
const WEDGE_HUE_MAX = 275;

export function SelectionCursorOverlay({ camera }) {
    const [style, setStyle] = useState(hiddenStyle());
    const [depth, setDepth] = useState(0);
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

        function syncFrom(source) {
            const mode = app.dataset.inputMode;
            const focused = document.activeElement;
            const ctrlHeld = isCtrlHeld();

            // CTRL held: wedge at viewport center, highlight nearest surface
            if (ctrlHeld) {
                const { surface: preview } = nearestSurfaceToCenter();
                const prevTarget = targetRef.current;
                if (preview !== prevTarget) {
                    clearCursorTargets();
                    targetRef.current = preview;
                    log(source, '→ CTRL PREVIEW:', elLabel(preview));
                }
                // Always ensure highlight is active on the preview surface
                if (preview && preview.getAttribute('data-selection-cursor-active') !== 'true') {
                    preview.setAttribute('data-selection-cursor-active', 'true');
                }
                setStyle(ctrlPreviewStyle(app));
                setDepth(preview ? surfaceDepth(preview) : 0);
                return;
            }

            if (mode === 'mouse' || (!hasFocusedSurface() && !ctrlHeld)) {
                if (mode === 'mouse' && hasFocusedSurface() && !pointerActive) {
                    log(source, '→ mouse mode but has focused surface, switching to keyboard');
                    app.dataset.inputMode = 'keyboard';
                } else {
                    log(source, '→ HIDE (mode:', mode, 'pointerActive:', pointerActive, 'hasFocused:', hasFocusedSurface(), ')');
                    trackTarget(null, syncFromCamera, resizeObserverRef, appRef, targetRef);
                    rectRef.current = null;
                    setStyle(previous => hiddenStyle(previous));
                    return;
                }
            }

            const prevTarget = targetRef.current;
            const nextTarget = findActiveSelectedSurface({ currentTarget: targetRef.current });
            const changed = nextTarget !== prevTarget;
            trackTarget(nextTarget, syncFromCamera, resizeObserverRef, appRef, targetRef);

            if (!(nextTarget instanceof HTMLElement)) {
                log(source, '→ HIDE (no target) | focused:', elLabel(focused), '| prev:', elLabel(prevTarget));
                rectRef.current = null;
                setStyle(previous => hiddenStyle(previous));
                return;
            }

            const nextRect = nextTarget.getBoundingClientRect();
            const prevRect = rectRef.current;
            rectRef.current = nextRect;
            setDepth(surfaceDepth(nextTarget));
            setStyle(cursorStyle(app, nextTarget, camera));

            if (changed) {
                log(source, '→ TARGET CHANGED:', elLabel(prevTarget), '→', elLabel(nextTarget),
                    '| rect:', rectLabel(nextRect), '| focused:', elLabel(focused));
            } else if (source !== 'camera') {
                const dx = prevRect ? Math.round(nextRect.left - prevRect.left) : 0;
                const dy = prevRect ? Math.round(nextRect.top - prevRect.top) : 0;
                if (dx || dy) {
                    log(source, '→ MOVED: Δ', dx, dy, '| target:', elLabel(nextTarget), '| rect:', rectLabel(nextRect));
                }
            }
        }

        let focusOutRaf = 0;
        const syncFromFocusIn = () => {
            if (focusOutRaf) { cancelAnimationFrame(focusOutRaf); focusOutRaf = 0; }
            syncFrom('focusin');
        };
        const syncFromFocusOut = () => {
            // Focus going to body is always transient (camera pan, view switch) — wait for focusin
            if (document.activeElement === document.body || document.activeElement == null) return;
            if (focusOutRaf) cancelAnimationFrame(focusOutRaf);
            focusOutRaf = requestAnimationFrame(() => { focusOutRaf = 0; syncFrom('focusout'); });
        };
        const syncFromCamera = () => syncFrom('camera');
        const syncFromMutation = () => syncFrom('mutation');
        const syncFromResize = () => syncFrom('resize');

        let pointerActive = false;
        const setInputMode = (mode) => {
            const prev = app.dataset.inputMode;
            if (prev !== mode) log('mode:', prev, '→', mode);
            app.dataset.inputMode = mode;
        };
        const onKey = () => {
            pointerActive = false;
            setInputMode('keyboard');
        };
        const onCtrlChange = (held) => syncFrom(held ? 'ctrl-down' : 'ctrl-up');
        const unsubCtrl = subscribeCtrl(onCtrlChange);
        const onPointer = () => {
            pointerActive = true;
            setInputMode('mouse');
        };
        const onPointerMove = () => {
            pointerActive = true;
            setInputMode('mouse');
        };
        const onWheel = () => setInputMode('mouse');
        setInputMode('keyboard');

        const observer = new MutationObserver(syncFromMutation);
        observer.observe(document.body, {
            attributes: true,
            attributeFilter: ATTRIBUTES,
            childList: true,
            subtree: true,
        });

        const unsubCamera = camera?.subscribe?.(syncFromCamera);
        document.addEventListener('focusin', syncFromFocusIn, true);
        document.addEventListener('focusout', syncFromFocusOut, true);
        document.addEventListener('keydown', onKey, true);
        document.addEventListener('pointerdown', onPointer, true);
        document.addEventListener('pointermove', onPointerMove, true);
        document.addEventListener('wheel', onWheel, { capture: true, passive: true });
        window.addEventListener('resize', syncFromResize);
        syncFrom('init');
        readyFrameRef.current = requestAnimationFrame(() => {
            readyFrameRef.current = 0;
            setReady(true);
        });

        return () => {
            observer.disconnect();
            if (unsubCamera) unsubCamera();
            document.removeEventListener('focusin', syncFromFocusIn, true);
            document.removeEventListener('focusout', syncFromFocusOut, true);
            document.removeEventListener('keydown', onKey, true);
            document.removeEventListener('pointerdown', onPointer, true);
            document.removeEventListener('pointermove', onPointerMove, true);
            unsubCtrl();
            document.removeEventListener('wheel', onWheel, true);
            window.removeEventListener('resize', syncFromResize);
            if (focusOutRaf) cancelAnimationFrame(focusOutRaf);
            if (readyFrameRef.current) cancelAnimationFrame(readyFrameRef.current);
            if (!resizeObserverRef.current) return;
            resizeObserverRef.current.disconnect();
            resizeObserverRef.current = null;
        };
    }, []);

    const wedgeHue = Math.min(WEDGE_HUE_MAX, WEDGE_HUE_BASE + Math.max(0, depth - 1) * WEDGE_HUE_STEP);
    const badgeText = hslLuminance(wedgeHue, 80, 38) > 0.18 ? '#000' : '#fff';
    const overlayStyle = { ...style, '--wedge-hue': String(wedgeHue), '--wedge-badge-color': badgeText };

    return html`
        <div class="selection-cursor-overlay ${ready ? 'is-ready' : ''}" style=${overlayStyle} aria-hidden="true" data-depth=${depth}>
            <${SelectionWedgeGlyph} />
            ${depth > 1 && html`<span class="selection-wedge-depth">${depth}</span>`}
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

function cursorStyle(app, target, cam) {
    const appRect = app.getBoundingClientRect();
    const targetRect = target.getBoundingClientRect();
    const vars = getComputedStyle(target);
    const restX = readVar(vars, '--selected-surface-wedge-rest-x', '0px');
    const restY = readVar(vars, '--selected-surface-wedge-rest-y', '0px');
    const gapX = readVar(vars, '--selected-surface-wedge-gap-x', '0px');
    const gapY = readVar(vars, '--selected-surface-wedge-gap-y', '0px');
    const top = readVar(vars, '--selected-surface-wedge-top', '-20px');
    const left = readVar(vars, '--selected-surface-wedge-left', '-24px');

    const z = cam?.zoom || 1;
    const wedgeScale = z < 1 ? Math.max(0.3, z) : 1;

    return {
        width: `${targetRect.width}px`,
        height: `${targetRect.height}px`,
        opacity: 1,
        transform: `translate(${targetRect.left - appRect.left}px, ${targetRect.top - appRect.top}px) scale(${wedgeScale})`,
        transformOrigin: 'top left',
        transition: 'none',
        '--selection-wedge-z': readVar(vars, '--selected-surface-wedge-z', 'var(--z-selection-wedge)'),
        '--selection-wedge-size': readVar(vars, '--selected-surface-wedge-size', '22px'),
        '--selection-wedge-top': `calc(${restY} + ${gapY} + ${top})`,
        '--selection-wedge-left': `calc(${restX} + ${gapX} + ${left})`,
    };
}

function ctrlPreviewStyle(app) {
    const appRect = app.getBoundingClientRect();
    const vp = document.getElementById('viewport');
    if (!vp) return hiddenStyle();
    const vr = vp.getBoundingClientRect();
    const cx = vr.left + vr.width / 2 - appRect.left;
    const cy = vr.top + vr.height / 2 - appRect.top;
    return {
        width: '0px',
        height: '0px',
        opacity: 1,
        transform: `translate(${cx}px, ${cy}px)`,
        transition: 'none',
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

function hasFocusedSurface() {
    const focused = document.activeElement;
    if (!(focused instanceof HTMLElement) || focused === document.body) return false;
    return focused.closest('[data-selected-surface]') !== null;
}


