import { html } from '../../lib/html.js';
import { useEffect, useLayoutEffect, useRef } from 'preact/hooks';
import { createWorldCanvasBg } from '../../lib/world-canvas-bg.js';
import { createDebug, elLabel } from '../../lib/debug.js';
import { isCtrlHeld } from '../../lib/ctrl-state.js';
import { nearestSurfaceToCenter } from '../../lib/viewport-spatial.js';
import { getWorldSettings } from '../../lib/world-settings.js';
import { selectorFor } from '../../lib/world-navigation.js';

const log = createDebug('qol:world');
const CAMERA_FOLLOW_PAD = 40;
const INTERACTIVE_SELECTOR = 'button, input, select, textarea, [data-selected-surface], a, [role="tab"], [tabindex]';

export function WorldViewport({ camera, onViewChange, navigation, children }) {
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
            if (e.button !== 0) return;
            if (e.target.closest(INTERACTIVE_SELECTOR)) { vp.classList.add('interactive'); return; }
            const d = dragRef.current;
            d.active = true;
            d.moved = false;
            d.startX = e.clientX;
            d.startY = e.clientY;
            d.camX = camera.x;
            d.camY = camera.y;
            camera.cancelSmooth();
            vp.classList.add('grabbing');
            vp.setPointerCapture(e.pointerId);
        }

        function onPointerMove(e) {
            const d = dragRef.current;
            if (!d.active) {
                const target = document.elementFromPoint(e.clientX, e.clientY);
                vp.classList.toggle('interactive', !!(target && target.closest(INTERACTIVE_SELECTOR)));
                return;
            }
            const dx = e.clientX - d.startX;
            const dy = e.clientY - d.startY;
            if (Math.abs(dx) > 3 || Math.abs(dy) > 3) d.moved = true;
            camera.panTo(d.camX - dx / camera.zoom, d.camY - dy / camera.zoom);
        }

        function onPointerUp(e) {
            const d = dragRef.current;
            if (!d.active) return;
            d.active = false;
            vp.classList.remove('grabbing');
            vp.classList.remove('interactive');
            vp.releasePointerCapture(e.pointerId);
        }

        function onWheel(e) {
            e.preventDefault();
            if (isCtrlHeld()) return;
            camera.nudge(e.deltaX / camera.zoom, e.deltaY / camera.zoom);
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
                        if (!isInput) snapToCenter(vp, onViewChange);
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
            if (isCtrlHeld()) return;
            const vr = vp.getBoundingClientRect();
            const fr = surface.getBoundingClientRect();
            let dx = 0, dy = 0;
            if (fr.bottom > vr.bottom - CAMERA_FOLLOW_PAD) dy = fr.bottom - (vr.bottom - CAMERA_FOLLOW_PAD);
            else if (fr.top < vr.top + CAMERA_FOLLOW_PAD) dy = fr.top - (vr.top + CAMERA_FOLLOW_PAD);
            if (fr.right > vr.right - CAMERA_FOLLOW_PAD) dx = fr.right - (vr.right - CAMERA_FOLLOW_PAD);
            else if (fr.left < vr.left + CAMERA_FOLLOW_PAD) dx = fr.left - (vr.left + CAMERA_FOLLOW_PAD);
            if (dx || dy) {
                log('cam follow Δ', Math.round(dx), Math.round(dy), elLabel(surface));
                camera.panSmooth(camera.x + dx / camera.zoom, camera.y + dy / camera.zoom, 200);
            }
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
        };
    }, [camera, navigation]);

    return html`
        <div id="viewport" ref=${viewportRef}>
            <canvas id="world-bg" ref=${bgCanvasRef}></canvas>
            <div id="world" ref=${worldRef}>
                ${children}
            </div>
        </div>
    `;
}

function snapToCenter(viewport, onViewChange) {
    const { surface, viewId, dist, count } = nearestSurfaceToCenter(viewport);

    if (viewId && onViewChange) onViewChange(viewId);

    if (!surface) {
        log('snap: no surfaces in', viewId);
        return;
    }

    log('snap →', elLabel(surface), 'in', viewId, 'dist:', Math.round(dist), 'of', count, 'candidates');
    surface.focus({ preventScroll: true });
}
