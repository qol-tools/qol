let resolveColor;
if (typeof document !== 'undefined') {
    resolveColor = (cssValue) => {
        const el = document.createElement('div');
        el.style.cssText = `display:none;color:${cssValue}`;
        document.body.appendChild(el);
        const rgb = getComputedStyle(el).color;
        document.body.removeChild(el);
        const m = rgb.match(/\d+/g);
        return m ? [+m[0], +m[1], +m[2]] : [26, 30, 38];
    };
}

const DISSOLVE_RATE = 0.14;
const BUBBLE_FADE = 0.05;
const BUBBLE_SPEED_MIN = 0.35;
const BUBBLE_SPEED_RANGE = 0.9;
const BUBBLE_WOBBLE_AMP = 1.5;
const BUBBLE_WOBBLE_FREQ = 0.28;
const WIND_STRENGTH = 0.12;
const VORTEX_COUNT = 2;
const VORTEX_RADIUS = 120;
const VORTEX_STRENGTH = 0.4;
const RENDER_SCALE = 2;
const MAX_PIXELS = 600000;
const TILE_SIZE = 32;
const TILE_JITTER = 0.2;
const SPARK_RATIO = 0.25;
const DEFAULT_DENSITY = 1.0;
const SPARK_THRESHOLD = (SPARK_RATIO * 4294967296) >>> 0;
const SIN_TABLE_SIZE = 4096;
const SIN_TABLE = new Float32Array(SIN_TABLE_SIZE);
for (let i = 0; i < SIN_TABLE_SIZE; i++) SIN_TABLE[i] = Math.sin((i / SIN_TABLE_SIZE) * Math.PI * 2);

function packRGBA(r, g, b, a) {
    return (a << 24) | (b << 16) | (g << 8) | r;
}

function fastSin(x) {
    const idx = ((x % (Math.PI * 2) + Math.PI * 2) / (Math.PI * 2) * SIN_TABLE_SIZE) & (SIN_TABLE_SIZE - 1);
    return SIN_TABLE[idx];
}

function spatialShuffle(total, W, tileSize = TILE_SIZE, origin = 'random') {
    const H = Math.ceil(total / W);
    const gw = Math.ceil(W / tileSize) + 1;
    const gh = Math.ceil(H / tileSize) + 1;
    const grid = new Float32Array(gw * gh);
    for (let i = 0; i < grid.length; i++) grid[i] = Math.random();
    const keys = new Float32Array(total);
    const cx = W / 2;
    const cy = H / 2;
    const maxDist = Math.sqrt(cx * cx + cy * cy);
    for (let i = 0; i < total; i++) {
        const px = i % W;
        const py = (i / W) | 0;
        const gx = px / tileSize;
        const gy = py / tileSize;
        const ix = gx | 0;
        const iy = gy | 0;
        const fx = gx - ix;
        const fy = gy - iy;
        const v0 = grid[iy * gw + ix] + (grid[iy * gw + ix + 1] - grid[iy * gw + ix]) * fx;
        const v1 = grid[(iy + 1) * gw + ix] + (grid[(iy + 1) * gw + ix + 1] - grid[(iy + 1) * gw + ix]) * fx;
        let key = v0 + (v1 - v0) * fy + Math.random() * TILE_JITTER;
        if (origin === 'center') {
            const dx = px - cx;
            const dy = py - cy;
            key = Math.sqrt(dx * dx + dy * dy) / maxDist + key * 0.3;
        }
        if (origin === 'edges') {
            const dx = px - cx;
            const dy = py - cy;
            key = 1 - Math.sqrt(dx * dx + dy * dy) / maxDist + key * 0.3;
        }
        keys[i] = key;
    }
    const indices = Array.from({ length: total }, (_, i) => i);
    indices.sort((a, b) => keys[a] - keys[b]);
    return indices;
}

export function computeCanvasSize(opts = {}) {
    const bleed = opts.bleed ?? 0;
    const edgeFade = opts.edgeFade ?? 0;
    const sourceW = opts.width ?? (typeof self !== 'undefined' && self.innerWidth || 1920);
    const sourceH = opts.height ?? (typeof self !== 'undefined' && self.innerHeight || 1080);
    const requestedScale = opts.renderScale ?? RENDER_SCALE;
    const totalPx = ((sourceW + bleed * 2) / requestedScale) * ((sourceH + bleed * 2) / requestedScale);
    const scale = totalPx > MAX_PIXELS
        ? Math.ceil(Math.sqrt((sourceW + bleed * 2) * (sourceH + bleed * 2) / MAX_PIXELS))
        : requestedScale;
    return {
        W: Math.ceil((sourceW + bleed * 2) / scale),
        H: Math.ceil((sourceH + bleed * 2) / scale),
        scale,
        edgeFade,
    };
}

export function createEvaporateState(canvas, cssColor, targetCssColor, opts = {}) {
    const { W, H, edgeFade, scale } = computeCanvasSize(opts);
    canvas.width = W;
    canvas.height = H;
    const ctx = canvas.getContext('2d');
    const [r, g, b] = typeof cssColor === 'string' ? resolveColor(cssColor) : cssColor;
    const [tr, tg, tb] = targetCssColor
        ? (typeof targetCssColor === 'string' ? resolveColor(targetCssColor) : targetCssColor)
        : [r, g, b];
    const total = W * H;
    const imgData = ctx.createImageData(W, H);
    const d = imgData.data;
    const d32 = new Uint32Array(d.buffer);
    const bgPacked = packRGBA(r, g, b, 255);
    const fadePx = edgeFade / scale;
    d32.fill(bgPacked);
    ctx.putImageData(imgData, 0, 0);
    const speeds = new Float32Array(total);
    const phases = new Float32Array(total);
    const speedMin = (opts.speedMin ?? BUBBLE_SPEED_MIN) / scale;
    const speedRange = (opts.speedRange ?? BUBBLE_SPEED_RANGE) / scale;
    for (let i = 0; i < total; i++) {
        speeds[i] = speedMin + ((Math.imul(i, 1234567891) >>> 0) / 4294967296) * speedRange;
        phases[i] = ((Math.imul(i, 2654435761) >>> 0) / 4294967296) * Math.PI * 2;
    }
    const windAngle = Math.random() * Math.PI * 2;
    const wind = opts.wind ?? WIND_STRENGTH;
    const windX = Math.cos(windAngle) * wind;
    const windY = Math.sin(windAngle) * wind;
    const fieldX = new Float32Array(total);
    const fieldY = new Float32Array(total);
    buildVortexField(fieldX, fieldY, W, H, windX, windY, opts);
    const density = opts.density ?? DEFAULT_DENSITY;
    const isParticle = new Uint8Array(total);
    for (let i = 0; i < total; i++) {
        isParticle[i] = ((Math.imul(i, 2654435761) >>> 0) / 4294967296) < density ? 1 : 0;
    }
    const edgeMap = new Float32Array(total);
    if (fadePx > 0) {
        for (let i = 0; i < total; i++) {
            edgeMap[i] = pixelEdgeFade(i % W, (i / W) | 0, W, H, fadePx);
        }
    } else {
        edgeMap.fill(1);
    }
    return {
        ctx, W, H, r, g, b, tr, tg, tb, total, imgData, d, d32, speeds, phases,
        edgeMap, isParticle, indices: spatialShuffle(total, W, opts.tileSize, opts.origin), birthFrame: new Int32Array(total).fill(-1),
        live: new Int32Array(total), liveCount: 0,
        cursor: 0, frame: 0, fieldX, fieldY,
        dissolveRate: opts.dissolveRate ?? DISSOLVE_RATE,
        bubbleFade: opts.bubbleFade ?? BUBBLE_FADE,
        maxBatchRate: opts.maxBatchRate ?? 0.04,
        invert: opts.invert ?? false,
        zoomRate: opts.zoomRate ?? 1,
        wobbleAmp: opts.wobbleAmp ?? BUBBLE_WOBBLE_AMP,
        wobbleFreq: opts.wobbleFreq ?? BUBBLE_WOBBLE_FREQ,
        colorDrift: opts.colorDrift ?? 0,
        echoes: opts.echoes ?? 0,
        sizeGrowth: opts.sizeGrowth ?? 0,
        spread: opts.spread ?? 1,
        dissolveAccel: opts.dissolveAccel ?? 0,
        ticksPerFrame: opts.ticksPerFrame ?? 1,
        convergence: opts.convergence ?? 0,
        vortexAt: opts.vortexAt ?? 0.42,
        phiAccelAt: opts.phiAccelAt ?? 0,
        materialize: opts.materialize ?? 0,
        particleGrids: opts.particleGrids ?? null,
        swirlVortices: opts.convergence > 0 ? [{
            x: W / 2, y: H / 2,
            dir: Math.random() > 0.5 ? 1 : -1,
            strength: opts.swirlStrength ?? 1.4,
        }] : [],
    };
}

function buildVortexField(fieldX, fieldY, W, H, windX, windY, opts = {}) {
    const vortexCount = opts.vortexCount ?? VORTEX_COUNT;
    const vortexRadius = opts.vortexRadius ?? VORTEX_RADIUS;
    const vortexStrength = opts.vortexStrength ?? VORTEX_STRENGTH;
    if (vortexCount <= 0 || vortexRadius <= 0 || vortexStrength <= 0) {
        fieldX.fill(windX);
        fieldY.fill(windY);
        return;
    }
    const vortices = [];
    for (let i = 0; i < vortexCount; i++) {
        vortices.push({
            x: Math.random() * W,
            y: Math.random() * H,
            dir: Math.random() > 0.5 ? 1 : -1,
            strength: vortexStrength * (0.6 + Math.random() * 0.8),
        });
    }
    const rSq = vortexRadius * vortexRadius;
    for (let idx = 0; idx < W * H; idx++) {
        const px = idx % W;
        const py = (idx / W) | 0;
        let dx = windX;
        let dy = windY;
        for (let v = 0; v < vortices.length; v++) {
            const vx = vortices[v];
            const rx = px - vx.x;
            const ry = py - vx.y;
            const distSq = rx * rx + ry * ry;
            if (distSq > rSq || distSq < 1) continue;
            const dist = Math.sqrt(distSq);
            const falloff = 1 - dist / vortexRadius;
            const perpX = -ry / dist * vx.dir;
            const perpY = rx / dist * vx.dir;
            dx += perpX * falloff * vx.strength;
            dy += perpY * falloff * vx.strength;
        }
        fieldX[idx] = dx;
        fieldY[idx] = dy;
    }
}

function pixelEdgeFade(x, y, W, H, fade) {
    const dist = Math.min(x, W - 1 - x, y, H - 1 - y);
    if (dist >= fade) return 1;
    return dist / fade;
}

export function activateBatch(s) {
    const progress = s.total > 0 ? s.cursor / s.total : 0;
    let accelMul = 1;
    if (s.dissolveAccel < 0) {
        accelMul = Math.max(0.05, 1 - Math.pow(progress, 12) * Math.abs(s.dissolveAccel));
    } else if (s.dissolveAccel > 0) {
        accelMul = 1 + s.dissolveAccel * progress;
    }
    if (s.phiAccelAt > 0 && progress > s.phiAccelAt) {
        const phiRamp = (progress - s.phiAccelAt) / (1 - s.phiAccelAt);
        accelMul *= Math.pow(1.618, phiRamp * 4);
    }
    const maxBatch = Math.ceil(s.total * s.maxBatchRate * accelMul);
    const batch = Math.min(maxBatch, Math.max(1, Math.ceil((s.total - s.cursor) * s.dissolveRate * accelMul)));
    const end = Math.min(s.cursor + batch, s.total);
    for (let i = s.cursor; i < end; i++) {
        const idx = s.indices[i];
        s.birthFrame[idx] = s.frame;
        if (s.isParticle[idx]) {
            s.live[s.liveCount++] = idx;
        }
    }
    s.cursor = end;
}

function fillUndissolved(s) {
    const { d32, indices, cursor, total, r, g, b, edgeMap } = s;
    for (let i = cursor; i < total; i++) {
        const idx = indices[i];
        d32[idx] = packRGBA(r, g, b, (edgeMap[idx] * 255 + 0.5) | 0);
    }
}

const SCATTER_SIZES = [2, 4, 6, 8, 11, 15, 20];
const MATERIALIZE_SIZES = [3, 6, 10, 15];

export function drawSolidPixels(s) {
    s._accentRects = [];
    const progress = s.total > 0 ? s.cursor / s.total : 0;
    if (s.materialize <= 0 || progress > 0.95) {
        fillUndissolved(s);
        return;
    }
    const remaining = s.total - s.cursor;
    const step = Math.max(1, (remaining / (s.W * 8)) | 0);
    for (let i = s.cursor; i < s.total; i += step) {
        const idx = s.indices[i];
        const bx = idx % s.W;
        const by = (idx / s.W) | 0;
        const seed = (Math.imul(idx, 2654435761) >>> 0);
        if ((seed ^ s.frame) % 180 === 0) continue;
        const sz = SCATTER_SIZES[(seed >> 8) % SCATTER_SIZES.length];
        const drifter = (seed & 0xf) < 3;
        const glitchJump = ((seed ^ s.frame) % 25 === 0) ? (((seed >> 4) & 0x7) - 4) : 0;
        const dx = glitchJump + (drifter ? ((Math.sin(s.frame * 0.008 + (seed & 0xff)) * 2) | 0) : 0);
        const dy = drifter ? ((Math.cos(s.frame * 0.006 + ((seed >> 8) & 0xff)) * 2) | 0) : 0;
        const ox = bx + dx - (sz >> 1);
        const oy = by + dy - (sz >> 1);
        s._accentRects.push(ox, oy, sz, sz);
        const tPacked = packRGBA(s.tr, s.tg, s.tb, 255);
        const yStart = Math.max(0, oy);
        const yEnd = Math.min(oy + sz, s.H);
        const xStart = Math.max(0, ox);
        const xEnd = Math.min(ox + sz, s.W);
        for (let py = yStart; py < yEnd; py++) {
            const rowOff = py * s.W;
            for (let px = xStart; px < xEnd; px++) {
                s.d32[rowOff + px] = tPacked;
            }
        }
    }
}

function particlePos(origX, origY, age, i, s, cx, cy, dir, wobbleAmp, jitter) {
    const dx = s.fieldX[i] * age + fastSin(s.phases[i] + age * s.wobbleFreq) * wobbleAmp;
    const dy = s.fieldY[i] * age + dir * age * s.speeds[i];
    let x = Math.round(origX + dx);
    let y = Math.round(origY + dy);
    if (s.zoomRate !== 1) {
        const zoom = Math.pow(s.zoomRate, age * jitter);
        x = Math.round(cx + (x - cx) * zoom);
        y = Math.round(cy + (y - cy) * zoom);
    }
    return [x, y];
}

export function processBubbles(s) {
    const wobbleAmp = s.wobbleAmp / RENDER_SCALE;
    const cx = s.W / 2;
    const cy = s.H / 2;
    const progress = s.total > 0 ? s.cursor / s.total : 0;
    const p = Math.pow(progress, 8);
    let driftR, driftG, driftB;
    if (s.colorDrift > 0) {
        driftR = Math.min(255, (s.tr + (180 - s.tr) * p * 0.3) | 0);
        driftG = Math.max(0, (s.tg - s.tg * p * 0.25) | 0);
        driftB = Math.max(0, (s.tb - s.tb * p * 0.3) | 0);
    } else if (s.colorDrift < 0) {
        driftR = Math.min(255, (s.tr + (255 - s.tr) * p * 0.25) | 0);
        driftG = Math.min(255, (s.tg + (255 - s.tg) * p * 0.25) | 0);
        driftB = Math.min(255, (s.tb + (255 - s.tb) * p * 0.25) | 0);
    } else {
        driftR = s.tr; driftG = s.tg; driftB = s.tb;
    }
    let writeIdx = 0;
    for (let j = 0; j < s.liveCount; j++) {
        const i = s.live[j];
        const age = s.frame - s.birthFrame[i];
        const isSpark = (Math.imul(i, 1103515245) >>> 0) < SPARK_THRESHOLD;
        const alpha = 1 - age * s.bubbleFade / s.spread;
        const origX = i % s.W;
        const origY = (i / s.W) | 0;
        const dir = s.invert ? 1 : -1;
        const jitter = 0.4 + ((Math.imul(i, 2654435761) >>> 0) / 4294967296) * 1.2;
        const pos = particlePos(origX, origY, age, i, s, cx, cy, dir, wobbleAmp, jitter);
        if (s.materialize > 0) {
            const sizeSeed = (Math.imul(i, 1664525) >>> 0) / 4294967296;
            const sIdx = (sizeSeed * MATERIALIZE_SIZES.length) | 0;
            pos[2] = MATERIALIZE_SIZES[Math.min(MATERIALIZE_SIZES.length - 1, sIdx)];
        }
        const vortexAt = s.swirlVortices.length > 0 ? (s.vortexAt ?? 0.42) : 2;
        if (s.convergence > 0 && progress > vortexAt) {
            const t0 = age * s.bubbleFade / s.spread;
            const ramp = Math.pow((progress - vortexAt) / (1 - vortexAt), 2);
            const pull = Math.min(1, t0 * t0 * ramp * s.convergence);
            const dx = pos[0] - cx;
            const dy = pos[1] - cy;
            const dist = Math.sqrt(dx * dx + dy * dy);
            const angle = Math.atan2(dy, dx);
            const v = s.swirlVortices[0];
            const depth = (Math.imul(i, 1664525) >>> 0) / 4294967296;
            const spin = v ? v.dir * v.strength * ramp * (0.5 + depth * 1.5) * 2 / Math.max(1, dist * 0.05) : 0;
            const newAngle = angle + spin;
            const newDist = dist * (1 - pull);
            const sway = Math.sin(s.phases[i] + age * 0.15) * ramp * dist * 0.2;
            const tubeWidth = Math.max(0, dist * 0.3 * (1 - pull * 0.7));
            const depthOffset = (depth - 0.5) * tubeWidth + sway;
            const perpX = -Math.sin(newAngle) * depthOffset;
            const perpY = Math.cos(newAngle) * depthOffset;
            pos[0] = Math.round(cx + Math.cos(newAngle) * newDist + perpX);
            pos[1] = Math.round(cy + Math.sin(newAngle) * newDist + perpY);
        }
        if (alpha <= 0 || pos[0] < 0 || pos[0] >= s.W || pos[1] < 0 || pos[1] >= s.H) continue;
        s.live[writeIdx++] = i;
        const cr = s.materialize > 0 ? s.tr : (isSpark ? driftR : ((s.r + driftR) >> 1));
        const cg = s.materialize > 0 ? s.tg : (isSpark ? driftG : ((s.g + driftG) >> 1));
        const cb = s.materialize > 0 ? s.tb : (isSpark ? driftB : ((s.b + driftB) >> 1));
        const sizeSeed = (Math.imul(i, 1664525) >>> 0) / 4294967296;
        const ss = sizeSeed * sizeSeed;
        const sizeT = s.sizeGrowth > 0 ? progress : (1 - progress);
        const echoCount = s.echoes + (s.convergence > 0 && progress > 0.7 ? (Math.pow((progress - 0.7) * 3.3, 3) * 20) | 0 : 0);
        for (let e = echoCount; e >= 0; e--) {
            const echoAge = age - e * 3;
            if (echoAge <= 0) continue;
            const echoAlpha = e === 0 ? alpha : alpha * 0.6 / (e * 0.5 + 0.5);
            let ep;
            if (e === 0) {
                ep = pos;
            } else {
                ep = particlePos(origX, origY, echoAge, i, s, cx, cy, dir, wobbleAmp, jitter);
                if (s.convergence > 0) {
                    const et = echoAge * s.bubbleFade / s.spread;
                    const ePull = et * et * progress * progress * progress * s.convergence;
                    ep[0] = Math.round(ep[0] + (cx - ep[0]) * ePull);
                    ep[1] = Math.round(ep[1] + (cy - ep[1]) * ePull);
                }
            }
            if (ep[0] < 0 || ep[0] >= s.W || ep[1] < 0 || ep[1] >= s.H) continue;
            const a = (echoAlpha * s.edgeMap[ep[1] * s.W + ep[0]] * 255) | 0;
            const pxSize = e > 0 ? 1 : s.materialize > 0
                ? Math.max(1, (pos[2] ?? 1) - 1)
                : s.sizeGrowth !== 0
                    ? Math.max(1, Math.min(4, 1 + (ss * sizeT * Math.abs(s.sizeGrowth) | 0)))
                    : 1;
            const packed = packRGBA(cr, cg, cb, a);
            for (let py = 0; py < pxSize; py++) {
                const ny = ep[1] + py;
                if (ny >= s.H) break;
                const rowOff = ny * s.W;
                for (let px = 0; px < pxSize; px++) {
                    const nx = ep[0] + px;
                    if (nx >= s.W) break;
                    const ni = rowOff + nx;
                    if (a <= s.d[(ni << 2) + 3]) continue;
                    s.d32[ni] = packed;
                }
            }
        }
    }
    s.liveCount = writeIdx;
}

export function evaporateFrame(s) {
    const progress = s.total > 0 ? s.cursor / s.total : 0;
    const ticks = Math.max(1, Math.ceil(s.ticksPerFrame * progress * 2));
    for (let t = 0; t < ticks; t++) activateBatch(s);
    s.d32.fill(0);
    drawSolidPixels(s);
    processBubbles(s);
    s.ctx.putImageData(s.imgData, 0, 0);
    s.frame++;
    return s.cursor >= s.total && s.liveCount === 0;
}

function buildParticleBuffer(s, buf) {
    const wobbleAmp = s.wobbleAmp / RENDER_SCALE;
    const cx = s.W / 2;
    const cy = s.H / 2;
    const progress = s.total > 0 ? s.cursor / s.total : 0;
    const p = Math.pow(progress, 8);
    const dir = s.invert ? 1 : -1;
    let driftR, driftG, driftB;
    if (s.colorDrift > 0) {
        driftR = (s.tr + (180 - s.tr) * p * 0.3) / 255;
        driftG = Math.max(0, (s.tg - s.tg * p * 0.25)) / 255;
        driftB = Math.max(0, (s.tb - s.tb * p * 0.3)) / 255;
    } else if (s.colorDrift < 0) {
        driftR = Math.min(1, (s.tr + (255 - s.tr) * p * 0.25) / 255);
        driftG = Math.min(1, (s.tg + (255 - s.tg) * p * 0.25) / 255);
        driftB = Math.min(1, (s.tb + (255 - s.tb) * p * 0.25) / 255);
    } else {
        driftR = s.tr / 255; driftG = s.tg / 255; driftB = s.tb / 255;
    }
    const bgR = s.r / 255, bgG = s.g / 255, bgB = s.b / 255;
    let count = 0;
    let writeIdx = 0;
    for (let j = 0; j < s.liveCount; j++) {
        const i = s.live[j];
        const age = s.frame - s.birthFrame[i];
        const alpha = 1 - age * s.bubbleFade / s.spread;
        if (alpha <= 0) continue;
        const origX = i % s.W;
        const origY = (i / s.W) | 0;
        const jitter = 0.4 + ((Math.imul(i, 2654435761) >>> 0) / 4294967296) * 1.2;
        const dx = s.fieldX[i] * age + fastSin(s.phases[i] + age * s.wobbleFreq) * wobbleAmp;
        const dy = s.fieldY[i] * age + dir * age * s.speeds[i];
        let x = origX + dx;
        let y = origY + dy;
        if (s.zoomRate !== 1) {
            const zoom = Math.pow(s.zoomRate, age * jitter);
            x = cx + (x - cx) * zoom;
            y = cy + (y - cy) * zoom;
        }
        if (x < 0 || x >= s.W || y < 0 || y >= s.H) continue;
        s.live[writeIdx++] = i;
        const isSpark = (Math.imul(i, 1103515245) >>> 0) < SPARK_THRESHOLD;
        const cr = isSpark ? driftR : (bgR + driftR) * 0.5;
        const cg = isSpark ? driftG : (bgG + driftG) * 0.5;
        const cb = isSpark ? driftB : (bgB + driftB) * 0.5;
        const off = count * 6;
        buf[off] = x;
        buf[off + 1] = y;
        buf[off + 2] = cr;
        buf[off + 3] = cg;
        buf[off + 4] = cb;
        buf[off + 5] = alpha;
        count++;
    }
    s.liveCount = writeIdx;
    return count;
}

let gpuModule = null;
try {
    if (typeof document !== 'undefined') {
        gpuModule = await import('./dissolve-gpu.js');
    }
} catch {}

function tryInitGPU(canvas, s) {
    if (!gpuModule) return null;
    try {
        return gpuModule.initGPU(canvas, s.W, s.H, s.indices, s.total, [s.r, s.g, s.b]);
    } catch { return null; }
}

function runDissolveGPU(canvas, bgColor, targetColor, onComplete, opts) {
    const s = createEvaporateState(canvas, bgColor, targetColor, opts);
    const gpu = tryInitGPU(canvas, s);
    if (!gpu) return null;
    const buf = gpu.particleData;
    let rafId = null;
    function cancel() {
        if (rafId) { cancelAnimationFrame(rafId); rafId = null; }
        canvas._dissolveCancel = null;
    }
    function tick() {
        const progress = s.total > 0 ? s.cursor / s.total : 0;
        const ticks = Math.max(1, Math.ceil(s.ticksPerFrame * progress * 2));
        for (let t = 0; t < ticks; t++) activateBatch(s);
        const particleCount = buildParticleBuffer(s, buf);
        gpuModule.renderFrame(gpu, progress, particleCount, buf);
        s.frame++;
        if (s.cursor >= s.total && s.liveCount === 0) {
            rafId = null;
            canvas._dissolveCancel = null;
            if (onComplete) onComplete();
            return;
        }
        rafId = requestAnimationFrame(tick);
    }
    canvas._dissolveCancel = cancel;
    rafId = requestAnimationFrame(tick);
    return cancel;
}

function runDissolveMainThread(canvas, bgColor, targetColor, onComplete, opts) {
    const s = createEvaporateState(canvas, bgColor, targetColor, opts);
    let rafId = null;
    function cancel() {
        if (rafId) { cancelAnimationFrame(rafId); rafId = null; }
        canvas._dissolveCancel = null;
    }
    function tick() {
        if (evaporateFrame(s)) {
            rafId = null;
            canvas._dissolveCancel = null;
            if (onComplete) onComplete();
            return;
        }
        rafId = requestAnimationFrame(tick);
    }
    canvas._dissolveCancel = cancel;
    rafId = requestAnimationFrame(tick);
    return cancel;
}

function runDissolveWorker(canvas, bgColor, targetColor, onComplete, opts) {
    const mergedOpts = {
        width: opts?.width ?? window.innerWidth,
        height: opts?.height ?? window.innerHeight,
        ...opts,
    };
    const { W, H } = computeCanvasSize(mergedOpts);
    canvas.width = W;
    canvas.height = H;
    const ctx = canvas.getContext('2d');
    const bufferSize = W * H * 4;
    let pixelBuffer = new ArrayBuffer(bufferSize);
    const imgData = ctx.createImageData(W, H);
    const worker = new Worker('./lib/dissolve-worker.js', { type: 'module' });
    let rafId = null;
    function cancel() {
        if (rafId) { cancelAnimationFrame(rafId); rafId = null; }
        worker.terminate();
        canvas._dissolveCancel = null;
    }
    worker.onmessage = (e) => {
        const { type, buffer } = e.data;
        pixelBuffer = buffer;
        if (type === 'done') {
            new Uint8Array(imgData.data.buffer).set(new Uint8Array(buffer));
            ctx.putImageData(imgData, 0, 0);
            canvas._dissolveCancel = null;
            worker.terminate();
            if (onComplete) onComplete();
            return;
        }
        rafId = requestAnimationFrame(() => {
            rafId = null;
            new Uint8Array(imgData.data.buffer).set(new Uint8Array(pixelBuffer));
            ctx.putImageData(imgData, 0, 0);
            worker.postMessage({ type: 'buffer', buffer: pixelBuffer }, [pixelBuffer]);
            pixelBuffer = null;
        });
    };
    worker.postMessage({
        type: 'start', bgColor, targetColor,
        opts: mergedOpts,
        pixelBuffer,
    }, [pixelBuffer]);
    canvas._dissolveCancel = cancel;
    return cancel;
}

const supportsWorker = typeof Worker !== 'undefined';

export function runDissolve(canvas, cssColor, onComplete, targetCssColor, opts) {
    if (canvas._dissolveCancel) canvas._dissolveCancel();
    const bgColor = typeof cssColor === 'string' && resolveColor ? resolveColor(cssColor) : cssColor;
    const targetColor = targetCssColor
        ? (typeof targetCssColor === 'string' && resolveColor ? resolveColor(targetCssColor) : targetCssColor)
        : bgColor;
    const gpuCancel = runDissolveGPU(canvas, bgColor, targetColor, onComplete, opts);
    if (gpuCancel) return gpuCancel;
    return runDissolveMainThread(canvas, bgColor, targetColor, onComplete, opts);
}

export function cancelExistingDissolve(container) {
    const old = container.querySelector('.dissolve-canvas');
    if (!old) return;
    if (old._dissolveCancel) old._dissolveCancel();
    old.remove();
}

export function createDissolveCanvas(container, opts) {
    const bleed = opts.bleed ?? 0;
    const canvas = document.createElement('canvas');
    canvas.className = 'dissolve-canvas';
    canvas.style.position = 'absolute';
    canvas.style.inset = `${-bleed}px`;
    canvas.style.width = bleed ? `calc(100% + ${bleed * 2}px)` : '100%';
    canvas.style.height = bleed ? `calc(100% + ${bleed * 2}px)` : '100%';
    canvas.style.pointerEvents = 'none';
    canvas.style.zIndex = '2';
    canvas.style.imageRendering = 'pixelated';
    if (opts.filter) canvas.style.filter = opts.filter;
    container.appendChild(canvas);
    return canvas;
}

export function sampleCompositeBg(el) {
    const c = document.createElement('canvas');
    c.width = 1;
    c.height = 1;
    const ctx = c.getContext('2d');
    const layers = [];
    let node = el;
    while (node) {
        const bg = getComputedStyle(node).backgroundColor;
        if (bg && bg !== 'transparent' && bg !== 'rgba(0, 0, 0, 0)') layers.push(bg);
        node = node.parentElement;
    }
    layers.reverse();
    for (const bg of layers) {
        ctx.fillStyle = bg;
        ctx.fillRect(0, 0, 1, 1);
    }
    const [r, g, b] = ctx.getImageData(0, 0, 1, 1).data;
    return `rgb(${r},${g},${b})`;
}
