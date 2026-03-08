import { html } from '../lib/html.js';
import { useState, useRef, useEffect } from 'preact/hooks';
import { shuffle, resolveColor, filledImageData } from '../lib/canvas.js';

const HOLD_MS = 2500;
const RED_AT_MS = 1000;
const BUBBLE_AT_MS = 1300;
const DISSOLVE_RATE = 0.09;
const BUBBLE_FADE = 0.035;
const BUBBLE_SPEED_MIN = 0.35;
const BUBBLE_SPEED_RANGE = 0.9;
const BUBBLE_WOBBLE_AMP = 1.5;
const BUBBLE_WOBBLE_FREQ = 0.28;

function createEvaporateState(canvas) {
    canvas.width = window.innerWidth;
    canvas.height = window.innerHeight;
    const ctx = canvas.getContext('2d');
    const [r, g, b] = resolveColor('var(--bg-base)');
    const W = canvas.width;
    const H = canvas.height;
    const total = W * H;
    const imgData = filledImageData(ctx, W, H, r, g, b);
    ctx.putImageData(imgData, 0, 0);
    return { ctx, W, H, r, g, b, total, imgData, d: imgData.data, indices: shuffle(total), activated: new Int32Array(total).fill(-1), cursor: 0, frame: 0 };
}

function activateBatch(s) {
    const batch = Math.max(1, Math.ceil((s.total - s.cursor) * DISSOLVE_RATE));
    const end = Math.min(s.cursor + batch, s.total);
    for (let i = s.cursor; i < end; i++) s.activated[s.indices[i]] = s.frame;
    s.cursor = end;
}

function bubblePosition(i, W, age) {
    const spd = BUBBLE_SPEED_MIN + ((Math.imul(i, 1234567891) >>> 0) / 4294967296) * BUBBLE_SPEED_RANGE;
    const ph = ((Math.imul(i, 2654435761) >>> 0) / 4294967296) * Math.PI * 2;
    const newX = Math.min(W - 1, Math.max(0, Math.round(i % W + Math.sin(ph + age * BUBBLE_WOBBLE_FREQ) * BUBBLE_WOBBLE_AMP)));
    const newY = Math.round((i / W | 0) - age * spd);
    return [newX, newY];
}

function renderBubblePixel(s, i) {
    if (s.activated[i] < 0) {
        const off = i * 4;
        s.d[off] = s.r; s.d[off + 1] = s.g; s.d[off + 2] = s.b; s.d[off + 3] = 255;
        return false;
    }
    const age = s.frame - s.activated[i];
    const alpha = 1 - age * BUBBLE_FADE;
    if (alpha <= 0) return false;
    const [newX, newY] = bubblePosition(i, s.W, age);
    if (newY < 0 || newY >= s.H) return true;
    const noff = (newY * s.W + newX) * 4;
    const a = (alpha * 255) | 0;
    if (a <= s.d[noff + 3]) return true;
    s.d[noff] = s.r; s.d[noff + 1] = s.g; s.d[noff + 2] = s.b; s.d[noff + 3] = a;
    return true;
}

function evaporateFrame(s) {
    activateBatch(s);
    s.d.fill(0);
    let anyMoving = false;
    for (let i = 0; i < s.total; i++) {
        if (renderBubblePixel(s, i)) anyMoving = true;
    }
    s.ctx.putImageData(s.imgData, 0, 0);
    s.frame++;
    return s.cursor >= s.total && !anyMoving;
}

export function CloseGuard() {
    const [visible, setVisible] = useState(false);
    const [remaining, setRemaining] = useState(HOLD_MS);
    const [danger, setDanger] = useState(false);
    const phaseRef = useRef('idle');
    const startRef = useRef(null);
    const rafRef = useRef(null);
    const bubbleRafRef = useRef(null);
    const canvasRef = useRef(null);

    useEffect(() => {
        function startEvaporation() {
            if (!canvasRef.current) return;
            const s = createEvaporateState(canvasRef.current);
            function loop() {
                if (evaporateFrame(s)) return;
                bubbleRafRef.current = requestAnimationFrame(loop);
            }
            bubbleRafRef.current = requestAnimationFrame(loop);
        }

        function tick() {
            const elapsed = performance.now() - startRef.current;
            const rem = Math.max(0, HOLD_MS - elapsed);
            setRemaining(rem);
            setDanger(rem <= RED_AT_MS);
            if (rem <= 0) { window.close(); return; }
            if (rem <= BUBBLE_AT_MS && phaseRef.current === 'holding') {
                phaseRef.current = 'dissolving';
                startEvaporation();
            }
            rafRef.current = requestAnimationFrame(tick);
        }

        function cancel() {
            if (phaseRef.current === 'idle') return;
            phaseRef.current = 'idle';
            startRef.current = null;
            if (rafRef.current) { cancelAnimationFrame(rafRef.current); rafRef.current = null; }
            if (bubbleRafRef.current) { cancelAnimationFrame(bubbleRafRef.current); bubbleRafRef.current = null; }
            const canvas = canvasRef.current;
            if (canvas) canvas.getContext('2d').clearRect(0, 0, canvas.width, canvas.height);
            setVisible(false);
            setDanger(false);
            setRemaining(HOLD_MS);
        }

        function onKeyDown(e) {
            if (!(e.ctrlKey && e.key === 'w')) return;
            e.preventDefault();
            if (phaseRef.current !== 'idle') return;
            phaseRef.current = 'holding';
            startRef.current = performance.now();
            setVisible(true);
            rafRef.current = requestAnimationFrame(tick);
        }

        function onKeyUp(e) {
            if (e.key !== 'w' && e.key !== 'Control') return;
            cancel();
        }

        document.addEventListener('keydown', onKeyDown);
        document.addEventListener('keyup', onKeyUp);
        return () => {
            cancel();
            document.removeEventListener('keydown', onKeyDown);
            document.removeEventListener('keyup', onKeyUp);
        };
    }, []);

    return html`
        <div class=${'close-guard-backdrop' + (visible ? ' visible' : '')}>
            <span class=${'close-guard-timer' + (danger ? ' danger' : '')}>
                ${(remaining / 1000).toFixed(1)}
            </span>
        </div>
        <canvas ref=${canvasRef} class="close-guard-canvas" />
    `;
}
