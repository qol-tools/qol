import { html } from '../../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';

const PAN_SPEED = 12;
const INTERACTIVE_SELECTOR = 'button, input, select, textarea, [data-selected-surface], a, [role="tab"], [tabindex]';

export function WorldViewport({ camera, children }) {
    const viewportRef = useRef(null);
    const worldRef = useRef(null);
    const dragRef = useRef({ active: false, startX: 0, startY: 0, camX: 0, camY: 0, moved: false });
    const ctrlRef = useRef(false);
    const keysRef = useRef(new Set());

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
            if (e.key === 'Control') ctrlRef.current = true;
            keysRef.current.add(e.key);
        }

        function onKeyUp(e) {
            if (e.key === 'Control') ctrlRef.current = false;
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
                if (dx || dy) camera.nudge(dx, dy);
            }
            rafId = requestAnimationFrame(ctrlPanLoop);
        }

        vp.addEventListener('pointerdown', onPointerDown);
        vp.addEventListener('pointermove', onPointerMove);
        vp.addEventListener('pointerup', onPointerUp);
        vp.addEventListener('wheel', onWheel, { passive: false });
        document.addEventListener('keydown', onKeyDown, true);
        document.addEventListener('keyup', onKeyUp, true);
        rafId = requestAnimationFrame(ctrlPanLoop);

        return () => {
            vp.removeEventListener('pointerdown', onPointerDown);
            vp.removeEventListener('pointermove', onPointerMove);
            vp.removeEventListener('pointerup', onPointerUp);
            vp.removeEventListener('wheel', onWheel);
            document.removeEventListener('keydown', onKeyDown, true);
            document.removeEventListener('keyup', onKeyUp, true);
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
