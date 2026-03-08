import { resolveColor } from './canvas.js';

const DISSOLVE_RATE = 0.09;
const BUBBLE_FADE = 0.035;
const BUBBLE_SPEED_MIN = 0.35;
const BUBBLE_SPEED_RANGE = 0.9;
const BUBBLE_WOBBLE_AMP = 1.5;
const BUBBLE_WOBBLE_FREQ = 0.28;
const RENDER_SCALE = 2;
const TILE_SIZE = 32;
const TILE_JITTER = 0.2;
const SPARK_RATIO = 0.15;
const SPARK_THRESHOLD = (SPARK_RATIO * 4294967296) >>> 0;

function spatialShuffle(total, W) {
    const H = Math.ceil(total / W);
    const gw = Math.ceil(W / TILE_SIZE) + 1;
    const gh = Math.ceil(H / TILE_SIZE) + 1;
    const grid = new Float32Array(gw * gh);
    for (let i = 0; i < grid.length; i++) grid[i] = Math.random();
    const keys = new Float32Array(total);
    for (let i = 0; i < total; i++) {
        const gx = (i % W) / TILE_SIZE;
        const gy = ((i / W) | 0) / TILE_SIZE;
        const ix = gx | 0;
        const iy = gy | 0;
        const fx = gx - ix;
        const fy = gy - iy;
        const v0 = grid[iy * gw + ix] + (grid[iy * gw + ix + 1] - grid[iy * gw + ix]) * fx;
        const v1 = grid[(iy + 1) * gw + ix] + (grid[(iy + 1) * gw + ix + 1] - grid[(iy + 1) * gw + ix]) * fx;
        keys[i] = v0 + (v1 - v0) * fy + Math.random() * TILE_JITTER;
    }
    const indices = Array.from({ length: total }, (_, i) => i);
    indices.sort((a, b) => keys[a] - keys[b]);
    return indices;
}

export function createEvaporateState(canvas, cssColor, targetCssColor) {
    const W = Math.ceil(window.innerWidth / RENDER_SCALE);
    const H = Math.ceil(window.innerHeight / RENDER_SCALE);
    canvas.width = W;
    canvas.height = H;
    const ctx = canvas.getContext('2d');
    const [r, g, b] = resolveColor(cssColor);
    const [tr, tg, tb] = targetCssColor ? resolveColor(targetCssColor) : [r, g, b];
    const total = W * H;
    const imgData = ctx.createImageData(W, H);
    const d = imgData.data;
    for (let i = 0; i < total; i++) {
        const off = i * 4;
        d[off] = r; d[off + 1] = g; d[off + 2] = b; d[off + 3] = 255;
    }
    ctx.putImageData(imgData, 0, 0);
    const speeds = new Float32Array(total);
    const phases = new Float32Array(total);
    for (let i = 0; i < total; i++) {
        speeds[i] = (BUBBLE_SPEED_MIN + ((Math.imul(i, 1234567891) >>> 0) / 4294967296) * BUBBLE_SPEED_RANGE) / RENDER_SCALE;
        phases[i] = ((Math.imul(i, 2654435761) >>> 0) / 4294967296) * Math.PI * 2;
    }
    return {
        ctx, W, H, r, g, b, tr, tg, tb, total, imgData, d, speeds, phases,
        indices: spatialShuffle(total, W), birthFrame: new Int32Array(total).fill(-1),
        live: new Int32Array(total), liveCount: 0,
        cursor: 0, frame: 0
    };
}

function activateBatch(s) {
    const batch = Math.max(1, Math.ceil((s.total - s.cursor) * DISSOLVE_RATE));
    const end = Math.min(s.cursor + batch, s.total);
    for (let i = s.cursor; i < end; i++) {
        const idx = s.indices[i];
        s.birthFrame[idx] = s.frame;
        s.live[s.liveCount++] = idx;
    }
    s.cursor = end;
}

function drawSolidPixels(s) {
    for (let i = s.cursor; i < s.total; i++) {
        const off = s.indices[i] * 4;
        s.d[off] = s.r; s.d[off + 1] = s.g; s.d[off + 2] = s.b; s.d[off + 3] = 255;
    }
}

function processBubbles(s) {
    const wobbleAmp = BUBBLE_WOBBLE_AMP / RENDER_SCALE;
    let writeIdx = 0;
    for (let j = 0; j < s.liveCount; j++) {
        const i = s.live[j];
        const age = s.frame - s.birthFrame[i];
        const alpha = 1 - age * BUBBLE_FADE;
        const newY = Math.round((i / s.W | 0) - age * s.speeds[i]);
        if (alpha <= 0 || newY < 0) continue;
        s.live[writeIdx++] = i;
        const newX = Math.min(s.W - 1, Math.max(0, Math.round(
            i % s.W + Math.sin(s.phases[i] + age * BUBBLE_WOBBLE_FREQ) * wobbleAmp
        )));
        const noff = (newY * s.W + newX) * 4;
        const a = (alpha * 255) | 0;
        if (a <= s.d[noff + 3]) continue;
        const isSpark = (Math.imul(i, 1103515245) >>> 0) < SPARK_THRESHOLD;
        s.d[noff] = isSpark ? s.tr : s.r;
        s.d[noff + 1] = isSpark ? s.tg : s.g;
        s.d[noff + 2] = isSpark ? s.tb : s.b;
        s.d[noff + 3] = a;
    }
    s.liveCount = writeIdx;
}

export function evaporateFrame(s) {
    activateBatch(s);
    s.d.fill(0);
    drawSolidPixels(s);
    processBubbles(s);
    s.ctx.putImageData(s.imgData, 0, 0);
    s.frame++;
    return s.cursor >= s.total && s.liveCount === 0;
}

export function runDissolve(canvas, cssColor, onComplete, targetCssColor) {
    const s = createEvaporateState(canvas, cssColor, targetCssColor);
    let rafId = null;
    function tick() {
        if (evaporateFrame(s)) {
            rafId = null;
            if (onComplete) onComplete();
            return;
        }
        rafId = requestAnimationFrame(tick);
    }
    rafId = requestAnimationFrame(tick);
    return () => { if (rafId) cancelAnimationFrame(rafId); };
}
