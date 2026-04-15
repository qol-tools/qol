import { html } from '../html.js';
import { useRef, useCallback, useLayoutEffect } from 'preact/hooks';
import { runDissolve } from '../dissolve.js';

const DISSOLVE_PENDING_KEY = 'qol:dissolve-pending';

function playDissolve(canvas, cancelRef, reload) {
    cancelRef.current = runDissolve(canvas, 'var(--bg-base)', () => {
        cancelRef.current = null;
        if (!reload) {
            const ctx = canvas.getContext('2d');
            ctx.clearRect(0, 0, canvas.width, canvas.height);
        }
    }, 'var(--accent)');
}

export function RecompileDissolve({ triggerRef }) {
    const canvasRef = useRef(null);
    const cancelRef = useRef(null);

    const trigger = useCallback((reload = true) => {
        if (!canvasRef.current || cancelRef.current) return;
        if (reload) {
            sessionStorage.setItem(DISSOLVE_PENDING_KEY, '1');
            window.location.reload();
            return;
        }
        playDissolve(canvasRef.current, cancelRef, false);
    }, []);

    useLayoutEffect(() => {
        if (triggerRef) triggerRef.current = trigger;
        if (!sessionStorage.getItem(DISSOLVE_PENDING_KEY)) return;
        sessionStorage.removeItem(DISSOLVE_PENDING_KEY);
        if (!canvasRef.current || cancelRef.current) return;
        document.body.classList.remove('dissolve-pending');
        playDissolve(canvasRef.current, cancelRef, false);
    }, [trigger, triggerRef]);

    return html`<canvas ref=${canvasRef} class="dissolve-canvas" />`;
}
