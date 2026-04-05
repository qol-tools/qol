import { html } from '../../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import { directSurfaces } from '../../lib/surface-traits.js';
import { nearestSurfaceInDirection } from '../../lib/spatial-nav.js';
import { createDebug } from '../../lib/debug.js';

const log = createDebug('qol:world');

const PAN_SPEED = 12;
const INTERACTIVE_SELECTOR = 'button, input, select, textarea, [data-selected-surface], a, [role="tab"], [tabindex]';

export function WorldViewport({ camera, onViewChange, children }) {
    const viewportRef = useRef(null);
    const worldRef = useRef(null);
    const dragRef = useRef({ active: false, startX: 0, startY: 0, camX: 0, camY: 0, moved: false });
    const ctrlRef = useRef(false);
    const keysRef = useRef(new Set());
    const ctrlPannedRef = useRef(false);

    useEffect(() => {
        if (worldRef.current) camera.setWorldElement(worldRef.current);
    }, [camera]);

    useEffect(() => {
        const vp = viewportRef.current;
        if (!vp) return;

        function onPointerDown(e) {
            if (e.button !== 0) return;
            const target = e.target;
            if (target.closest(INTERACTIVE_SELECTOR)) {
                vp.classList.add('interactive');
                return;
            }
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
            camera.panTo(d.camX - dx, d.camY - dy);
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
            if (ctrlRef.current) return;
            camera.nudge(e.deltaX, e.deltaY);
        }

        function onKeyDown(e) {
            if (e.key === 'Control') {
                ctrlRef.current = true;
                ctrlPannedRef.current = false;
                document.body.dataset.ctrlHeld = '';
            }
            keysRef.current.add(e.key);
            // When CTRL is held, consume arrow keys so they only pan the camera
            if (ctrlRef.current && (e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
                e.stopPropagation();
            }
        }

        function onKeyUp(e) {
            if (e.key === 'Control') {
                ctrlRef.current = false;
                delete document.body.dataset.ctrlHeld;
                const panned = ctrlPannedRef.current;
                log('ctrl-release panned:', panned);
                if (panned) {
                    requestAnimationFrame(() => {
                        const isInput = document.activeElement?.matches('input, textarea, select, [contenteditable]');
                        if (!isInput) focusNearestSurface(vp, onViewChange);
                    });
                }
            }
            keysRef.current.delete(e.key);
        }

        let rafId = 0;
        function ctrlPanLoop() {
            if (ctrlRef.current) {
                const keys = keysRef.current;
                let dx = 0, dy = 0;
                if (keys.has('ArrowLeft')) dx = -PAN_SPEED;
                if (keys.has('ArrowRight')) dx = PAN_SPEED;
                if (keys.has('ArrowUp')) dy = -PAN_SPEED;
                if (keys.has('ArrowDown')) dy = PAN_SPEED;
                if (dx || dy) { camera.nudge(dx, dy); ctrlPannedRef.current = true; }
            }
            rafId = requestAnimationFrame(ctrlPanLoop);
        }

        // Camera follow: any surface focus outside viewport triggers a pan
        function onFocusIn(e) {
            if (ctrlRef.current) return;
            const surface = e.target?.closest?.('[data-selected-surface]');
            if (!surface) return;
            const vr = vp.getBoundingClientRect();
            const fr = surface.getBoundingClientRect();
            const pad = 40;
            let dx = 0, dy = 0;
            if (fr.bottom > vr.bottom - pad) dy = fr.bottom - (vr.bottom - pad);
            else if (fr.top < vr.top + pad) dy = fr.top - (vr.top + pad);
            if (fr.right > vr.right - pad) dx = fr.right - (vr.right - pad);
            else if (fr.left < vr.left + pad) dx = fr.left - (vr.left + pad);
            if (dx || dy) {
                log('cam follow Δ', Math.round(dx), Math.round(dy), elLabel(surface));
                camera.panSmooth(camera.x + dx, camera.y + dy, 200);
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
    }, [camera]);

    return html`
        <div id="viewport" ref=${viewportRef}>
            <div id="world" ref=${worldRef}>
                <div id="world-bg"></div>
                ${children}
            </div>
        </div>
    `;
}

function isInViewport(el, viewport) {
    if (!el || !viewport) return false;
    const vr = viewport.getBoundingClientRect();
    const er = el.getBoundingClientRect();
    return er.width > 0 && er.height > 0 &&
        er.bottom > vr.top && er.top < vr.bottom &&
        er.right > vr.left && er.left < vr.right;
}

function slotAtCenter(viewport) {
    const vr = viewport.getBoundingClientRect();
    const cx = vr.left + vr.width / 2;
    const cy = vr.top + vr.height / 2;

    const el = document.elementFromPoint(cx, cy);
    let slot = el?.closest('.world-view-slot');
    let method = 'elementFromPoint';

    if (!slot) {
        method = 'overlap';
        let bestOverlap = 0;
        for (const s of viewport.querySelectorAll('.world-view-slot')) {
            const sr = s.getBoundingClientRect();
            const ox = Math.max(0, Math.min(sr.right, vr.right) - Math.max(sr.left, vr.left));
            const oy = Math.max(0, Math.min(sr.bottom, vr.bottom) - Math.max(sr.top, vr.top));
            const overlap = ox * oy;
            if (overlap > bestOverlap) { bestOverlap = overlap; slot = s; }
        }
    }

    log('slotAtCenter:', slot?.dataset?.viewId || 'NONE', 'via', method,
        'center:', Math.round(cx), Math.round(cy));
    return slot;
}

function focusNearestSurface(viewport, onViewChange) {
    if (!viewport) return;
    const vr = viewport.getBoundingClientRect();
    const cx = vr.left + vr.width / 2;
    const cy = vr.top + vr.height / 2;

    const slot = slotAtCenter(viewport);
    const searchRoot = slot || viewport;

    const viewId = slot?.dataset?.viewId;
    if (viewId && onViewChange) onViewChange(viewId);

    const surfaces = Array.from(searchRoot.querySelectorAll('[data-selected-surface]'))
        .filter(el => {
            const r = el.getBoundingClientRect();
            return r.width > 0 && r.height > 0 &&
                r.bottom > vr.top && r.top < vr.bottom &&
                r.right > vr.left && r.left < vr.right;
        });

    if (surfaces.length === 0) {
        log('snap: no surfaces in', viewId);
        return;
    }

    let best = surfaces[0];
    let bestDist = Infinity;
    for (const el of surfaces) {
        const r = el.getBoundingClientRect();
        const d = Math.hypot(r.left + r.width / 2 - cx, r.top + r.height / 2 - cy);
        if (d < bestDist) { best = el; bestDist = d; }
    }

    const br = best.getBoundingClientRect();
    log('snap →', elLabel(best), 'in', viewId,
        `(${Math.round(br.left)},${Math.round(br.top)})`, 'dist:', Math.round(bestDist),
        'of', surfaces.length, 'candidates');
    best.focus({ preventScroll: true });
}

function elLabel(el) {
    if (!el) return 'null';
    const tag = el.tagName?.toLowerCase() || '?';
    const cls = el.className ? '.' + String(el.className).split(/\s+/).slice(0, 2).join('.') : '';
    return tag + cls;
}
