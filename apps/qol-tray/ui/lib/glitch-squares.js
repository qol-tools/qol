const SHAPES = {
    normal:       { freq: 86, draw: drawNormal },
    quadrantLoss: { freq: 4,  draw: drawQuadrantLoss, minSize: 4 },
    lineToSquare: { freq: 1,  draw: drawLineToSquare, minSize: 4 },
    fadeOut:       { freq: 7,  draw: drawFadeOut, onlyEpochEnd: true },
};

const MOVEMENTS = {
    static:  { freq: 194 },
    snap:    { freq: 5,  move: moveSnap },
    jitter:  { freq: 1,  move: moveJitter },
};

const DECORATIONS = {
    none:      { freq: 72 },
    lineStub:  { freq: 6,  render: renderLineStub },
    faintGlow: { freq: 22, render: renderFaintGlow },
};

const LONGEVITY = [
    { freq: 3, shift: 4 },
    { freq: 2, shift: 5 },
    { freq: 2, shift: 6 },
    { freq: 1, shift: 8 },
];

const SIZES = [1, 1, 2, 2, 3, 4, 6, 9, 14, 20, 28, 36];

const OCC_CELL = 5;
const _mv = [0, 0];

export function createField(W, H, accentFill, bgFill) {
    const gw = Math.ceil(W / OCC_CELL);
    const gh = Math.ceil(H / OCC_CELL);
    return {
        W, H, accentFill, bgFill,
        salt: (Math.random() * 0xffffffff) >>> 0,
        frame: 0,
        _occupied: { grid: new Uint8Array(gw * gh), gw, gh },
    };
}

export function renderField(ctx, glowCtx, field, progress) {
    const count = Math.max(3, ((1 - progress) * 24000) | 0);
    field._occupied.grid.fill(0);
    for (let q = 0; q < count; q++) {
        renderSquare(ctx, glowCtx, field, q, progress);
    }
    field.frame++;
}

function renderSquare(ctx, glowCtx, f, q, progress) {
    const lifeRoll = (Math.imul(q, 48271) >>> 0) & 0x7;
    const longevity = pickLongevity(lifeRoll);
    const epochLen = 1 << longevity.shift;
    const offset = (Math.imul(q, 65537) >>> 0) & (epochLen - 1);
    const epoch = ((f.frame + offset) >> longevity.shift);
    const epochAge = (f.frame + offset) & (epochLen - 1);
    const epochEnd = epochLen - epochAge;
    const seed = hash(q * 7919 ^ (epoch * 2654435761) ^ f.salt, 1103515245 + epoch);

    if ((seed ^ f.frame) % 150 === 0) return;

    const sz = SIZES[(seed >> 8) % SIZES.length];
    const rectRoll = (seed >> 24) & 0xf;
    const sw = rectRoll < 2 ? Math.round(sz * 1.5) : rectRoll < 4 ? Math.max(2, Math.round(sz * 0.7)) : sz;
    const sh = rectRoll < 2 ? Math.max(2, Math.round(sz * 0.7)) : rectRoll < 4 ? Math.round(sz * 1.5) : sz;
    const jx = ((seed >> 22) & 0xf) - 8;
    const jy = ((seed >> 26) & 0xf) - 8;
    const baseX = (((seed >> 4) + jx) % f.W + f.W) % f.W;
    const baseY = ((((seed >> 14) ^ (seed >> 2)) + jy) % f.H + f.H) % f.H;
    const accent = ((seed >> 20) & 0x7) < 3;
    const alphaSeed = ((seed >> 18) & 0xff) / 255;
    const sizeAlpha = 1 - Math.min(1, Math.max(sw, sh) / 36) * 0.7;
    const baseAlpha = (0.3 + alphaSeed * alphaSeed * 0.7) * sizeAlpha;
    const fadeSpeed = 0.3 + ((seed >> 3) & 0xf) / 15 * 2.0;
    const fadePhase = ((seed >> 11) & 0xff) / 255 * Math.PI * 2;
    const fade = 0.5 + 0.5 * Math.sin(epochAge * fadeSpeed * 0.1 + fadePhase);
    const alpha = baseAlpha * fade;

    const movement = pickMovement(seed, f.frame);
    let dx = 0, dy = 0;
    if (movement.move) movement.move(f.frame, seed, _mv);
    dx = _mv[0]; dy = _mv[1];
    const fx = baseX + dx;
    const fy = baseY + dy;

    {
        const occ = f._occupied;
        const cx0 = Math.max(0, (fx / OCC_CELL) | 0);
        const cy0 = Math.max(0, (fy / OCC_CELL) | 0);
        const cx1 = Math.min(occ.gw - 1, ((fx + sw) / OCC_CELL) | 0);
        const cy1 = Math.min(occ.gh - 1, ((fy + sh) / OCC_CELL) | 0);
        let blocked = false;
        for (let cy = cy0; cy <= cy1 && !blocked; cy++) {
            for (let cx = cx0; cx <= cx1; cx++) {
                if (occ.grid[cy * occ.gw + cx]) { blocked = true; break; }
            }
        }
        if (blocked) return;
        for (let cy = cy0; cy <= cy1; cy++) {
            for (let cx = cx0; cx <= cx1; cx++) {
                occ.grid[cy * occ.gw + cx] = 1;
            }
        }
    }

    const shape = pickShape(seed, sz, epochEnd);
    const fillStyle = accent ? f.accentFill : f.bgFill;

    const sq = { fx, fy, sz, sw, sh, seed, epochAge, epochLen, epochEnd, alpha, accent, fillStyle };
    ctx.globalAlpha = shape.onlyEpochEnd ? (epochEnd / 4) * alpha : alpha;
    ctx.fillStyle = fillStyle;
    shape.draw(ctx, sq);
    ctx.globalAlpha = 1;

    renderSpawnBuzz(ctx, glowCtx, f, sq);
    renderGlow(glowCtx, f, sq);

    const decoration = pickDecoration(seed);
    if (decoration.render) decoration.render(ctx, glowCtx, f, sq);
}

function pickLongevity(roll) {
    if (roll < 3) return LONGEVITY[0];
    if (roll < 5) return LONGEVITY[1];
    if (roll < 7) return LONGEVITY[2];
    return LONGEVITY[3];
}

function pickShape(seed, sz, epochEnd) {
    if (epochEnd <= 4 && ((seed >> 6) & 0x7) < 2) return SHAPES.fadeOut;
    const roll = (Math.imul(seed, 7727) >>> 0) % 100;
    if (roll < SHAPES.quadrantLoss.freq && sz >= SHAPES.quadrantLoss.minSize) return SHAPES.quadrantLoss;
    if (roll < SHAPES.quadrantLoss.freq + SHAPES.lineToSquare.freq && sz >= SHAPES.lineToSquare.minSize) return SHAPES.lineToSquare;
    return SHAPES.normal;
}

function pickMovement(seed, frame) {
    const roll = (Math.imul(seed ^ (frame >> 3), 48271) >>> 0) % 200;
    if (roll < MOVEMENTS.snap.freq) return MOVEMENTS.snap;
    if (roll < MOVEMENTS.snap.freq + MOVEMENTS.jitter.freq) return MOVEMENTS.jitter;
    return MOVEMENTS.static;
}

function pickDecoration(seed) {
    const roll = (seed >> 12) & 0xff;
    if (roll < 2) return DECORATIONS.lineStub;
    return DECORATIONS.none;
}

function drawNormal(ctx, sq) {
    ctx.fillRect(sq.fx, sq.fy, sq.sw, sq.sh);
}

function drawQuadrantLoss(ctx, sq) {
    const t = sq.epochAge / sq.epochLen;
    ctx.fillRect(sq.fx, sq.fy, sq.sw, sq.sh);
    if (t > 0.4) {
        const hw = sq.sw >> 1;
        const hh = sq.sh >> 1;
        const quad = (sq.seed >> 7) & 0x3;
        ctx.clearRect(
            sq.fx + (quad & 1 ? hw : 0),
            sq.fy + (quad & 2 ? hh : 0),
            hw, hh
        );
    }
}

function drawLineToSquare(ctx, sq) {
    const t = Math.min(1, sq.epochAge / (sq.epochLen * 0.3));
    const dir = (sq.seed >> 5) & 1 ? -1 : 1;
    const lineW = sq.sw * 3 + 4;
    const lineH = Math.max(1, sq.sh >> 2);
    const curW = lineW + (sq.sw - lineW) * t;
    const curH = lineH + (sq.sh - lineH) * t;
    ctx.fillRect(sq.fx, sq.fy + t * dir * 6, curW, curH);
}

function drawFadeOut(ctx, sq) {
    const grow = ((1 - sq.epochEnd / 4) * 3) | 0;
    ctx.fillRect(sq.fx - grow, sq.fy - grow, sq.sw + grow * 2, sq.sh + grow * 2);
}

function moveSnap(frame, seed, out) {
    const dir = (seed >> 5) & 1 ? 1 : -1;
    out[0] = ((frame >> 2) & 1) ? dir * 6 : 0;
    out[1] = 0;
}

function moveJitter(frame, seed, out) {
    const axis = (seed >> 3) & 1;
    const v = (frame & 1) ? 3 : -3;
    out[0] = axis ? v : 0;
    out[1] = axis ? 0 : v;
}

function renderSpawnBuzz(ctx, glowCtx, f, sq) {
    if (sq.epochAge >= 2 || ((sq.seed >> 10) & 0x3f) !== 0) return;
    ctx.fillStyle = f.accentFill;
    glowCtx.fillStyle = f.accentFill;
    const n = 3 + (sq.seed % 5);
    for (let b = 0; b < n; b++) {
        const bs = hash(b * 3571 + sq.seed, 1664525);
        const bsz = 1 + (bs & 0x3);
        const bx = sq.fx + ((bs >> 4) & 0x1f) - 16;
        const by = sq.fy + ((bs >> 9) & 0x1f) - 16;
        ctx.fillRect(bx, by, bsz, bsz);
        glowCtx.fillRect(bx, by, bsz, bsz);
        if ((bs & 0x8) === 0) {
            const blen = 3 + (bs >> 14) % 12;
            if (bs & 0x10) ctx.fillRect(bx + bsz, by, blen, 1);
            else ctx.fillRect(bx, by + bsz, 1, blen);
        }
    }
    ctx.fillStyle = sq.fillStyle;
}

function renderGlow(glowCtx, f, sq) {
    if (!glowCtx) return;
    const superGlow = ((sq.seed >> 2) & 0xff) < 6;
    if (sq.accent || superGlow) {
        glowCtx.fillStyle = f.accentFill;
        glowCtx.globalAlpha = superGlow ? 1 : sq.alpha;
        const pad = superGlow ? 3 : 0;
        glowCtx.fillRect(sq.fx - pad, sq.fy - pad, sq.sw + pad * 2, sq.sh + pad * 2);
        glowCtx.globalAlpha = 1;
        return;
    }
    if (((sq.seed >> 24) & 0x7) >= 2) return;
    glowCtx.globalAlpha = 0.3;
    glowCtx.fillStyle = f.accentFill;
    glowCtx.fillRect(sq.fx, sq.fy, sq.sw, sq.sh);
    glowCtx.globalAlpha = 1;
}

function renderLineStub(ctx, glowCtx, f, sq) {
    const longLine = ((sq.seed >> 24) & 0x7) < 1;
    const lineLen = longLine ? sq.sz + 15 + ((sq.seed >> 18) % 25) : sq.sz + ((sq.seed >> 18) % 8);
    const h = Math.max(1, sq.sz >> 2);
    const horizontal = (sq.seed >> 15) & 1;
    ctx.fillStyle = sq.fillStyle;
    const dir = (sq.seed >> 17) & 1 ? 1 : -1;
    if (horizontal) {
        const lx = sq.fx + (dir > 0 ? sq.sz : -lineLen);
        const ly = sq.fy + ((sq.sz - h) >> 1);
        ctx.fillRect(lx, ly, lineLen, h);
        if (sq.accent) glowCtx.fillRect(lx, ly, lineLen, h);
    } else {
        const lx = sq.fx + ((sq.sz - h) >> 1);
        const ly = sq.fy + (dir > 0 ? sq.sz : -lineLen);
        ctx.fillRect(lx, ly, h, lineLen);
        if (sq.accent) glowCtx.fillRect(lx, ly, h, lineLen);
    }
}

function renderFaintGlow(ctx, glowCtx, f, sq) {
    if (sq.accent || !glowCtx) return;
    glowCtx.globalAlpha = 0.3;
    glowCtx.fillStyle = f.accentFill;
    glowCtx.fillRect(sq.fx, sq.fy, sq.sw, sq.sh);
    glowCtx.globalAlpha = 1;
}

function hash(a, b) {
    return (Math.imul(a, b) >>> 0);
}
