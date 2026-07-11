import { html } from '../../lib/html.js';
import { useEffect, useLayoutEffect, useRef } from 'preact/hooks';
import { createWorldCanvasBg } from '../../fx/world-canvas-bg.js';
import { createDebug, elLabel } from '../../lib/debug.js';
import { isCtrlHeld } from '../../lib/modifier-state.js';
import { findActiveSelectedSurface } from '../../lib/selected-surface.js';
import { KEYBOARD_FOLLOW_DURATION_MS, edgeFollowDelta, normalizedZoom, surfaceCenterDelta } from '../../lib/viewport-follow.js';
import { nearestSurfaceToCenter } from '../../lib/viewport-spatial.js';
import { cameraTargetForSurface, screenRectToWorld } from '../../lib/world-geometry.js';
import { getWorldSettings } from '../../lib/world-settings.js';
import { selectorFor } from '../../lib/world-navigation.js';
import { contains } from '../../lib/world-registry.js';
import { PeripheralPreview } from './PeripheralPreview.js';
import { AtmosphereLayer } from './AtmosphereLayer.js';

const log = createDebug('qol:world');
const WHEEL_ZOOM_FACTOR = 0.002;
const INTERACTIVE_SELECTOR = 'button, input, select, textarea, [data-selected-surface], a, [role="tab"], [tabindex]';

export function WorldViewport({ camera, onViewChange, navigation, registry, renderPage, children }) {
    const viewportRef = useRef(null);
    const worldRef = useRef(null);
    const dragRef = useRef({ active: false, startX: 0, startY: 0, camX: 0, camY: 0, moved: false });
    const keysRef = useRef(new Set());
    const ctrlPannedRef = useRef(false);
    const bgCanvasRef = useRef(null);

    useEffect(() => {
        if (!bgCanvasRef.current) return;
        const bg = createWorldCanvasBg(bgCanvasRef.current, camera);
        return () => bg.destroy();
    }, [camera]);

    useLayoutEffect(() => {
        if (worldRef.current) camera.setWorldElement(worldRef.current);
    }, [camera]);

    useEffect(() => {
        const vp = viewportRef.current;
        if (!vp) return;

        function onPointerDown(e) {
            const isMiddleClick = e.button === 1;
            const isAltLeftClick = e.button === 0 && e.altKey;
            const isForcePan = isMiddleClick || isAltLeftClick;
            if (e.button !== 0 && !isMiddleClick) return;
            const isPeripheral = !!e.target.closest('.peripheral-preview');
            if (!isForcePan && !isPeripheral && e.target.closest(INTERACTIVE_SELECTOR)) {
                vp.classList.add('interactive');
                return;
            }
            if (isForcePan) e.preventDefault();
            const d = dragRef.current;
            d.pending = true;
            d.active = false;
            d.moved = false;
            d.startX = e.clientX;
            d.startY = e.clientY;
            d.camX = camera.x;
            d.camY = camera.y;
            d.pointerId = e.pointerId;
        }

        function onPointerMove(e) {
            const d = dragRef.current;
            if (!d.pending && !d.active) {
                const target = document.elementFromPoint(e.clientX, e.clientY);
                vp.classList.toggle('interactive', !!(target && target.closest(INTERACTIVE_SELECTOR)));
                return;
            }
            const dx = e.clientX - d.startX;
            const dy = e.clientY - d.startY;
            if (d.pending && (Math.abs(dx) > 3 || Math.abs(dy) > 3)) {
                d.pending = false;
                d.active = true;
                d.moved = true;
                camera.cancelSmooth();
                window.getSelection?.()?.removeAllRanges?.();
                vp.classList.add('grabbing');
                vp.setPointerCapture(d.pointerId);
            }
            if (d.active) {
                camera.panTo(d.camX - dx / camera.zoom, d.camY - dy / camera.zoom);
            }
        }

        function onPointerUp(e) {
            const d = dragRef.current;
            if (d.active) {
                vp.classList.remove('grabbing');
                vp.classList.remove('interactive');
                vp.releasePointerCapture(e.pointerId);
            }
            d.pending = false;
            d.active = false;
        }

        function onWheel(e) {
            e.preventDefault();
            if (e.deltaY) {
                const rect = vp.getBoundingClientRect();
                camera.zoomAround(
                    e.clientX - rect.left,
                    e.clientY - rect.top,
                    camera.zoom * Math.exp(-e.deltaY * WHEEL_ZOOM_FACTOR),
                );
            }
            if (e.deltaX) camera.nudge(e.deltaX / camera.zoom, 0);
        }

        function onKeyDown(e) {
            if (e.key === 'Control') ctrlPannedRef.current = false;
            keysRef.current.add(e.key);
            if (isCtrlHeld() && (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
                e.stopPropagation();
            }
        }

        function onKeyUp(e) {
            if (e.key === 'Control') {
                const panned = ctrlPannedRef.current;
                log('ctrl-release panned:', panned);
                if (panned) {
                    requestAnimationFrame(() => {
                        const isInput = document.activeElement?.matches('input, textarea, select, [contenteditable]');
                        if (!isInput) snapToCenter(vp, onViewChange, navigation, registry);
                    });
                }
            }
            keysRef.current.delete(e.key);
        }

        let rafId = 0;
        function ctrlPanLoop() {
            if (isCtrlHeld()) {
                const keys = keysRef.current;
                let dx = 0, dy = 0;
                const speed = getWorldSettings().panSpeed / camera.zoom;
                if (keys.has('ArrowLeft')) dx = -speed;
                if (keys.has('ArrowRight')) dx = speed;
                if (keys.has('ArrowUp')) dy = -speed;
                if (keys.has('ArrowDown')) dy = speed;
                if (dx || dy) { camera.nudge(dx, dy); ctrlPannedRef.current = true; }
            }
            rafId = requestAnimationFrame(ctrlPanLoop);
        }

        function onFocusIn(e) {
            const surface = e.target?.closest?.('[data-selected-surface]');
            if (!surface) return;
            if (navigation) {
                const pageId = surface.closest('[data-view-id]')?.dataset?.viewId;
                const selector = selectorFor(surface);
                if (pageId && selector) navigation.setFocus(pageId, selector);
            }
            followSurface(activeFollowTarget(surface));
        }

        const VIEWPORT_ANIM_CLASSES = ['dive-out', 'ascend-out', 'fade-out', 'layer-in'];
        let pendingFollow = 0;

        function followSurface(surface) {
            cancelAnimationFrame(pendingFollow);
            pendingFollow = 0;
            if (!(surface instanceof HTMLElement)) return;
            if (isCtrlHeld()) return;
            if (VIEWPORT_ANIM_CLASSES.some((cls) => vp.classList.contains(cls))) {
                pendingFollow = requestAnimationFrame(() => followSurface(surface));
                return;
            }
            const vr = vp.getBoundingClientRect();
            const fr = surface.getBoundingClientRect();
            const inputMode = document.querySelector('.app-container')?.dataset?.inputMode || 'keyboard';
            if (inputMode === 'keyboard' && followPageFirst(vr, fr, surface)) return;
            const { dx, dy, mode, duration } = inputMode === 'keyboard'
                ? surfaceCenterDelta(vr, fr)
                : edgeFollowDelta(vr, fr);
            if (dx || dy) {
                const zoom = normalizedZoom(camera.zoom);
                log('cam follow', mode, 'Δ', Math.round(dx), Math.round(dy), elLabel(surface));
                camera.panSmooth(camera.x + dx / zoom, camera.y + dy / zoom, duration);
            }
        }

        function followPageFirst(vr, fr, surface) {
            const pageId = surface.closest('[data-view-id]')?.dataset?.viewId;
            const entry = pageId ? registry?.getEntry?.(pageId) : null;
            if (!entry || !(fr.width > 0) || !(fr.height > 0)) return false;
            const zoom = normalizedZoom(camera.zoom);
            const surfaceWorld = screenRectToWorld(fr, vr, camera);
            const target = cameraTargetForSurface(entry, surfaceWorld, vr.width, vr.height, zoom);
            const dx = (target.x - camera.x) * zoom;
            const dy = (target.y - camera.y) * zoom;
            if (Math.abs(dx) > 0.5 || Math.abs(dy) > 0.5) {
                log('cam follow page-first', 'Δ', Math.round(dx), Math.round(dy), elLabel(surface));
                camera.panSmooth(target.x, target.y, KEYBOARD_FOLLOW_DURATION_MS);
            }
            return true;
        }

        vp.addEventListener('pointerdown', onPointerDown);
        vp.addEventListener('pointermove', onPointerMove);
        vp.addEventListener('pointerup', onPointerUp);
        vp.addEventListener('wheel', onWheel, { passive: false });
        document.addEventListener('keydown', onKeyDown, true);
        document.addEventListener('keyup', onKeyUp, true);
        document.addEventListener('focusin', onFocusIn, true);
        rafId = requestAnimationFrame(ctrlPanLoop);

        return () => {
            vp.removeEventListener('pointerdown', onPointerDown);
            vp.removeEventListener('pointermove', onPointerMove);
            vp.removeEventListener('pointerup', onPointerUp);
            vp.removeEventListener('wheel', onWheel);
            document.removeEventListener('keydown', onKeyDown, true);
            document.removeEventListener('keyup', onKeyUp, true);
            document.removeEventListener('focusin', onFocusIn, true);
            cancelAnimationFrame(rafId);
            cancelAnimationFrame(pendingFollow);
        };
    }, [camera, navigation, registry]);

    return html`
        <div id="viewport" ref=${viewportRef}>
            <canvas id="world-bg" ref=${bgCanvasRef}></canvas>
            <${AtmosphereLayer} navigation=${navigation} />
            <div id="world" ref=${worldRef}>
                ${children}
            </div>
            <${PeripheralPreview} camera=${camera} navigation=${navigation} registry=${registry} renderPage=${renderPage} />
        </div>
    `;
}

function snapToCenter(viewport, onViewChange, navigation, registry) {
    const { surface, viewId, dist, count } = nearestSurfaceToCenter(viewport);
    const confinement = navigation?.getCurrentConfinement?.() || null;
    const viewIdInsideConfinement = (() => {
        if (!viewId) return null;
        if (!confinement) return viewId;
        const entry = registry?.getEntry?.(viewId);
        if (!entry) return null;
        return contains(confinement, entry) ? viewId : null;
    })();
    const fallbackAnchor = navigation?.getCurrentAnchor?.();
    const effectiveViewId = viewIdInsideConfinement || fallbackAnchor?.pageId || null;

    if (viewId && onViewChange) onViewChange(viewId);

    if (surface && navigation && viewId) {
        const selector = selectorFor(surface);
        if (selector) navigation.setFocus(viewId, selector);
    }

    if (!surface) {
        log('snap: no surfaces in', viewId || '(no-view)', '→ recover to', effectiveViewId || '(none)');
        if (navigation && effectiveViewId) {
            navigation.setCurrentAnchor({ pageId: effectiveViewId });
            navigation.gotoAnchor({ pageId: effectiveViewId }, { respectKnob: false });
        }
        return;
    }

    log('snap →', elLabel(surface), 'in', viewId, 'dist:', Math.round(dist), 'of', count, 'candidates');

    if (navigation) {
        navigation.setCurrentAnchor({ pageId: viewId });
        navigation.gotoAnchor({ pageId: viewId }, { respectKnob: true });
    } else {
        surface.focus({ preventScroll: true });
    }
}

function activeFollowTarget(fallback) {
    return findActiveSelectedSurface({ currentTarget: fallback }) || fallback;
}
