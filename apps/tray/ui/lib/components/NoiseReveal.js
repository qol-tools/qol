import { html } from '../html.js';
import { useRef, useEffect } from 'preact/hooks';
import { shuffle, resolveColor, sizeToParent, filledImageData } from '../../fx/canvas.js';

const DISSOLVE_RATE = 0.06;
const FADE_STEP = 0.05;
const CELL = 4;
const BUBBLE_SPEED_MIN = 0.35;
const BUBBLE_SPEED_RANGE = 0.9;
const BUBBLE_WOBBLE_AMP = 1.5;
const BUBBLE_WOBBLE_FREQ = 0.28;
const BUBBLE_FADE = 0.025;

export function NoiseReveal({ variant = 'dissolve' }) {
    const canvasRef = useRef(null);
    const frameRef = useRef(null);
    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        let lastW = 0;
        const animate = variant === 'bubble' ? runBubble : runDissolve;
        const observer = new ResizeObserver((entries) => {
            const w = entries[0].contentRect.width;
            if (w > 0 && lastW === 0) animate(canvas, frameRef);
            lastW = w;
        });
        observer.observe(canvas);
        return () => {
            observer.disconnect();
            if (frameRef.current) cancelAnimationFrame(frameRef.current);
        };
    }, []);
    return html`<canvas ref=${canvasRef} class="noise-reveal" />`;
}

function createDissolveState(canvas) {
    const [W, H] = sizeToParent(canvas);
    const ctx = canvas.getContext('2d');
    const [r, g, b] = resolveColor('var(--bg-base)');
    const cols = Math.ceil(W / CELL);
    const total = cols * Math.ceil(H / CELL);
    const imgData = filledImageData(ctx, W, H, r, g, b);
    ctx.putImageData(imgData, 0, 0);
    return { ctx, W, H, cols, total, imgData, d: imgData.data, indices: shuffle(total), cellAlpha: new Float32Array(total).fill(1), cursor: 0 };
}

function dissolveFrame(s) {
    const batch = Math.max(1, Math.ceil((s.total - s.cursor) * DISSOLVE_RATE));
    const end = Math.min(s.cursor + batch, s.total);
    for (let i = s.cursor; i < end; i++) s.cellAlpha[s.indices[i]] = 1 - FADE_STEP;
    s.cursor = end;
    let done = s.cursor >= s.total;
    for (let i = 0; i < s.total; i++) {
        if (s.cellAlpha[i] <= 0 || s.cellAlpha[i] >= 1) continue;
        s.cellAlpha[i] = Math.max(0, s.cellAlpha[i] - FADE_STEP);
        if (s.cellAlpha[i] > 0) done = false;
        const a = Math.round(s.cellAlpha[i] * 255);
        const col = i % s.cols, row = Math.floor(i / s.cols);
        const x0 = col * CELL, y0 = row * CELL;
        const x1 = Math.min(x0 + CELL, s.W), y1 = Math.min(y0 + CELL, s.H);
        for (let y = y0; y < y1; y++)
            for (let x = x0; x < x1; x++)
                s.d[(y * s.W + x) * 4 + 3] = a;
    }
    s.ctx.putImageData(s.imgData, 0, 0);
    return done;
}

function runDissolve(canvas, frameRef) {
    if (frameRef.current) cancelAnimationFrame(frameRef.current);
    const s = createDissolveState(canvas);
    const tick = () => {
        if (dissolveFrame(s)) { frameRef.current = null; s.ctx.clearRect(0, 0, s.W, s.H); return; }
        frameRef.current = requestAnimationFrame(tick);
    };
    frameRef.current = requestAnimationFrame(tick);
}

function createBubbleState(canvas) {
    const [W, H] = sizeToParent(canvas);
    const ctx = canvas.getContext('2d');
    const [r, g, b] = resolveColor('var(--bg-base)');
    const total = W * H;
    const imgData = filledImageData(ctx, W, H, r, g, b);
    ctx.putImageData(imgData, 0, 0);
    return { ctx, W, H, r, g, b, total, imgData, d: imgData.data, indices: shuffle(total), activated: new Int32Array(total).fill(-1), cursor: 0, frame: 0 };
}

function bubbleFrame(s) {
    const batch = Math.max(1, Math.ceil((s.total - s.cursor) * DISSOLVE_RATE));
    const end = Math.min(s.cursor + batch, s.total);
    for (let i = s.cursor; i < end; i++) s.activated[s.indices[i]] = s.frame;
    s.cursor = end;
    s.d.fill(0);
    let anyMoving = false;
    for (let i = 0; i < s.total; i++) {
        if (s.activated[i] < 0) {
            const off = i * 4;
            s.d[off] = s.r; s.d[off + 1] = s.g; s.d[off + 2] = s.b; s.d[off + 3] = 255;
            continue;
        }
        const age = s.frame - s.activated[i];
        const alpha = 1 - age * BUBBLE_FADE;
        if (alpha <= 0) continue;
        anyMoving = true;
        const spd = BUBBLE_SPEED_MIN + ((Math.imul(i, 1234567891) >>> 0) / 4294967296) * BUBBLE_SPEED_RANGE;
        const ph = ((Math.imul(i, 2654435761) >>> 0) / 4294967296) * Math.PI * 2;
        const newX = Math.min(s.W - 1, Math.max(0, Math.round(i % s.W + Math.sin(ph + age * BUBBLE_WOBBLE_FREQ) * BUBBLE_WOBBLE_AMP)));
        const newY = Math.round((i / s.W | 0) - age * spd);
        if (newY < 0 || newY >= s.H) continue;
        const noff = (newY * s.W + newX) * 4;
        const a = (alpha * 255) | 0;
        if (a <= s.d[noff + 3]) continue;
        s.d[noff] = s.r; s.d[noff + 1] = s.g; s.d[noff + 2] = s.b; s.d[noff + 3] = a;
    }
    s.ctx.putImageData(s.imgData, 0, 0);
    s.frame++;
    return s.cursor >= s.total && !anyMoving;
}

function runBubble(canvas, frameRef) {
    if (frameRef.current) cancelAnimationFrame(frameRef.current);
    const s = createBubbleState(canvas);
    const tick = () => {
        if (bubbleFrame(s)) { frameRef.current = null; s.ctx.clearRect(0, 0, s.W, s.H); return; }
        frameRef.current = requestAnimationFrame(tick);
    };
    frameRef.current = requestAnimationFrame(tick);
}
