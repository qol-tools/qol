import { html } from '../html.js';
import { useRef, useEffect } from 'preact/hooks';
import { shuffle, resolveColor } from '../canvas.js';

const FADE_IN_RATE = 0.12;
const INITIAL_ALPHA = 0.35;
const ALPHA_STEP = 0.07;
const BORDER_HEIGHT = 3;
const GLOW_ROWS = [0.3, 1, 0.3];

export function NoiseBorder({ active }) {
    const canvasRef = useRef(null);

    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const parent = canvas.parentElement;

        if (!active) {
            canvas.getContext('2d').clearRect(0, 0, canvas.width, canvas.height);
            return;
        }

        if (parent.offsetWidth > 0) return runAnimation(canvas);

        let animCleanup = null;
        let started = false;
        const observer = new ResizeObserver((entries) => {
            const w = entries[0].contentRect.width;
            if (w === 0 || started) return;
            started = true;
            animCleanup = runAnimation(canvas);
        });
        observer.observe(parent);
        return () => { observer.disconnect(); animCleanup?.(); };
    }, [active]);

    return html`<canvas ref=${canvasRef} class="noise-border" />`;
}

function initBorderState(canvas) {
    const w = canvas.parentElement.offsetWidth;
    canvas.width = w;
    canvas.height = BORDER_HEIGHT;
    const ctx = canvas.getContext('2d');
    ctx.clearRect(0, 0, w, BORDER_HEIGHT);
    const [r, g, b] = resolveColor('var(--accent)');
    return { ctx, w, r, g, b, indices: shuffle(w), alpha: new Float32Array(w), cursor: 0 };
}

function borderFrame(s) {
    const batch = Math.max(1, Math.ceil((s.w - s.cursor) * FADE_IN_RATE));
    const end = Math.min(s.cursor + batch, s.w);
    for (let i = s.cursor; i < end; i++) s.alpha[s.indices[i]] = INITIAL_ALPHA;
    s.cursor = end;
    let allDone = true;
    const imgData = s.ctx.createImageData(s.w, BORDER_HEIGHT);
    const d = imgData.data;
    for (let i = 0; i < s.w; i++) {
        if (s.alpha[i] > 0 && s.alpha[i] < 1) {
            s.alpha[i] = Math.min(1, s.alpha[i] + ALPHA_STEP);
            allDone = false;
        }
        if (s.alpha[i] <= 0) continue;
        for (let row = 0; row < BORDER_HEIGHT; row++) {
            const off = (row * s.w + i) * 4;
            d[off] = s.r; d[off + 1] = s.g; d[off + 2] = s.b;
            d[off + 3] = Math.round(GLOW_ROWS[row] * s.alpha[i] * 255);
        }
    }
    s.ctx.putImageData(imgData, 0, 0);
    return allDone && s.cursor >= s.w;
}

function runAnimation(canvas) {
    const s = initBorderState(canvas);
    let frame;
    const tick = () => {
        if (borderFrame(s)) return;
        frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => { if (frame) cancelAnimationFrame(frame); };
}
