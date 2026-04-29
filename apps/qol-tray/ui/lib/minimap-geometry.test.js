import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    computeMinimapLinearLayout,
    computeMinimapLinearRect,
    computeSlotCoverage,
} from './minimap-geometry.js';
import { clampRectForDraw, drawMinimap, drawViewportRect, VIEWPORT_MIN_WIDTH } from './minimap-draw.js';

const THREE_ENTRIES = [
    { id: 'a', x: 0,     y: 0, width: 1280, height: 900 },
    { id: 'b', x: 10000, y: 0, width: 1280, height: 900 },
    { id: 'c', x: 20000, y: 0, width: 1280, height: 900 },
];

// ---------------------------------------------------------------------------
// computeMinimapLinearLayout — linear projection of entries into pixel space.
// The contract that matters most: slot widths depend ONLY on entry.width
// and the projection scale (= minimapWidth / range). They are invariant to
// how many entries fall inside the projected window. That is the whole
// point of switching to linear projection — the previous packing made
// slots rescale as the user navigated between pages.
// ---------------------------------------------------------------------------

test('layout: empty input returns null', () => {
    assert.equal(computeMinimapLinearLayout({
        entries: [], worldStart: 0, worldEnd: 100, minimapWidth: 200,
    }), null);
});

test('layout: invalid minimap width returns null', () => {
    assert.equal(computeMinimapLinearLayout({
        entries: THREE_ENTRIES, worldStart: 0, worldEnd: 100, minimapWidth: 0,
    }), null);
});

test('layout: invalid range (end <= start) returns null', () => {
    assert.equal(computeMinimapLinearLayout({
        entries: THREE_ENTRIES, worldStart: 100, worldEnd: 100, minimapWidth: 200,
    }), null);
});

test('layout: slot.x is the linear projection of entry.x', () => {
    const layout = computeMinimapLinearLayout({
        entries: THREE_ENTRIES, worldStart: 0, worldEnd: 21280, minimapWidth: 380,
    });
    const scale = 380 / 21280;
    assert.ok(Math.abs(layout.slots[0].x - 0) < 1e-9);
    assert.ok(Math.abs(layout.slots[1].x - 10000 * scale) < 1e-9);
    assert.ok(Math.abs(layout.slots[2].x - 20000 * scale) < 1e-9);
});

test('layout: slot.w = entry.width * scale (gaps preserved)', () => {
    const layout = computeMinimapLinearLayout({
        entries: THREE_ENTRIES, worldStart: 0, worldEnd: 21280, minimapWidth: 380,
    });
    const scale = 380 / 21280;
    for (const s of layout.slots) {
        assert.ok(Math.abs(s.w - 1280 * scale) < 1e-9);
    }
});

test('layout: slot widths are invariant to how many entries fall in the range', () => {
    // Range 1: just entry "b" near the centre.
    const tight = computeMinimapLinearLayout({
        entries: THREE_ENTRIES, worldStart: 9000, worldEnd: 12000, minimapWidth: 380,
    });
    // Range 2: same range width, all three entries (entries far apart so only one
    // falls in either case, but slot.w depends only on scale, not visible count).
    const range = 12000 - 9000;
    const expectedSlotW = 1280 * (380 / range);

    // tight: slot for b is centred in the strip (entry.x maps to (10000 - 9000)*scale)
    const scale = 380 / range;
    assert.ok(Math.abs(tight.slots[1].w - expectedSlotW) < 1e-9);
    assert.ok(Math.abs(tight.slots[1].x - (10000 - 9000) * scale) < 1e-9);

    // Slots a and c are off-screen; their w still equals expectedSlotW.
    assert.ok(Math.abs(tight.slots[0].w - expectedSlotW) < 1e-9);
    assert.ok(Math.abs(tight.slots[2].w - expectedSlotW) < 1e-9);
    // a is to the left of strip, c is to the right.
    assert.ok(tight.slots[0].x < 0);
    assert.ok(tight.slots[2].x > 380);
});

test('layout: slot widths depend on factor — same entries, different range, different size', () => {
    const tight = computeMinimapLinearLayout({
        entries: THREE_ENTRIES, worldStart: 9000, worldEnd: 11000, minimapWidth: 380,
    });
    const wide = computeMinimapLinearLayout({
        entries: THREE_ENTRIES, worldStart: 0, worldEnd: 21280, minimapWidth: 380,
    });
    assert.ok(tight.slots[1].w > wide.slots[1].w,
        `tight (range 2000) should give bigger slot than wide (range 21280)`);
});

test('layout: row centred vertically and shrunk to canvasHeight when natural h overflows', () => {
    // tall entry: aspect 1:10. With minimapWidth 200 and range matching entry.width,
    // slot.w = 200, natural slot.h = 200 * (10/1) = 2000. Cap at canvasHeight 50.
    const layout = computeMinimapLinearLayout({
        entries: [{ id: 'tall', x: 0, y: 0, width: 100, height: 1000 }],
        worldStart: 0, worldEnd: 100, minimapWidth: 200, canvasHeight: 50,
    });
    assert.ok(Math.abs(layout.rowHeight - 50) < 1e-9, `rowHeight=${layout.rowHeight}`);
    assert.ok(Math.abs(layout.rowY - 0) < 1e-9, `rowY=${layout.rowY}`);
    // Aspect preserved: row scale shrunk both w and h.
    const expectedW = 200 * (50 / 2000);
    assert.ok(Math.abs(layout.slots[0].w - expectedW) < 1e-9);
    assert.ok(Math.abs(layout.slots[0].h - 50) < 1e-9);
});

test('layout: row centred when natural row height is shorter than canvas', () => {
    const layout = computeMinimapLinearLayout({
        entries: [{ id: 'a', x: 0, y: 0, width: 100, height: 50 }],
        worldStart: 0, worldEnd: 100, minimapWidth: 200, canvasHeight: 200,
    });
    // Natural slot.w = 200, slot.h = 100. canvasHeight 200 → rowY = 50.
    assert.ok(Math.abs(layout.rowHeight - 100) < 1e-9);
    assert.ok(Math.abs(layout.rowY - 50) < 1e-9);
});

test('layout: degenerate entry (width or height 0) gets zero-size slot but is still indexed', () => {
    const layout = computeMinimapLinearLayout({
        entries: [{ id: 'a', x: 0, y: 0, width: 0, height: 0 }],
        worldStart: 0, worldEnd: 100, minimapWidth: 200,
    });
    assert.equal(layout.slots.length, 1);
    assert.equal(layout.slots[0].w, 0);
    assert.equal(layout.slots[0].h, 0);
});

// ---------------------------------------------------------------------------
// computeMinimapLinearRect — camera viewport projected to minimap pixels.
// ---------------------------------------------------------------------------

test('rect: full viewport range matches the minimap width', () => {
    const rect = computeMinimapLinearRect({
        cameraX: 0, viewportRange: 100, worldStart: 0, worldEnd: 100, minimapWidth: 200,
    });
    assert.equal(rect.x, 0);
    assert.equal(rect.width, 200);
});

test('rect: at factor 4 (range = 4 × viewportRange), rect is exactly 1/4 of the strip', () => {
    const viewportRange = 100;
    const factor = 4;
    const range = viewportRange * factor;
    // Camera centred on the projected range.
    const cameraX = (range - viewportRange) / 2; // worldStart=0, so cameraX = halfRange - vr/2
    const rect = computeMinimapLinearRect({
        cameraX,
        viewportRange,
        worldStart: 0,
        worldEnd: range,
        minimapWidth: 400,
    });
    assert.ok(Math.abs(rect.width - 100) < 1e-9, `expected 100, got ${rect.width}`);
    // Centred horizontally: rect.x = (400 - 100) / 2 = 150
    assert.ok(Math.abs(rect.x - 150) < 1e-9, `expected x=150, got ${rect.x}`);
});

test('rect: panning the camera by dx moves the rect by dx * scale', () => {
    const before = computeMinimapLinearRect({
        cameraX: 5000, viewportRange: 1280, worldStart: 4000, worldEnd: 6560, minimapWidth: 256,
    });
    const after = computeMinimapLinearRect({
        cameraX: 5500, viewportRange: 1280, worldStart: 4000, worldEnd: 6560, minimapWidth: 256,
    });
    const scale = 256 / 2560;
    assert.ok(Math.abs((after.x - before.x) - 500 * scale) < 1e-9);
    assert.ok(Math.abs(after.width - before.width) < 1e-9);
});

test('rect: zero or invalid inputs return zero-width rect', () => {
    const z1 = computeMinimapLinearRect({
        cameraX: 0, viewportRange: 0, worldStart: 0, worldEnd: 100, minimapWidth: 200,
    });
    assert.equal(z1.width, 0);

    const z2 = computeMinimapLinearRect({
        cameraX: 0, viewportRange: 100, worldStart: 100, worldEnd: 100, minimapWidth: 200,
    });
    assert.equal(z2.width, 0);
});

test('rect: rowY/rowHeight pass through', () => {
    const r = computeMinimapLinearRect({
        cameraX: 0, viewportRange: 100, worldStart: 0, worldEnd: 200, minimapWidth: 100,
        rowY: 5, rowHeight: 30,
    });
    assert.equal(r.y, 5);
    assert.equal(r.height, 30);
});

// ---------------------------------------------------------------------------
// computeSlotCoverage — overlap fraction of slot vs camera rect, in pixels.
// ---------------------------------------------------------------------------

test('coverage: slot fully inside rect returns 1', () => {
    const slot = { x: 50, w: 20 };
    const rect = { x: 0, width: 100 };
    assert.equal(computeSlotCoverage(slot, rect), 1);
});

test('coverage: slot fully outside rect returns 0', () => {
    const slot = { x: 200, w: 20 };
    const rect = { x: 0, width: 100 };
    assert.equal(computeSlotCoverage(slot, rect), 0);
});

test('coverage: half overlap returns 0.5', () => {
    const slot = { x: 0, w: 100 };
    const rect = { x: 50, width: 60 };
    // overlap = 50, slot.w = 100 → 0.5
    assert.equal(computeSlotCoverage(slot, rect), 0.5);
});

test('coverage: missing slot/rect returns 0', () => {
    assert.equal(computeSlotCoverage(null, { x: 0, width: 10 }), 0);
    assert.equal(computeSlotCoverage({ x: 0, w: 10 }, null), 0);
    assert.equal(computeSlotCoverage({ x: 0, w: 0 }, { x: 0, width: 10 }), 0);
    assert.equal(computeSlotCoverage({ x: 0, w: 10 }, { x: 0, width: 0 }), 0);
});

// ---------------------------------------------------------------------------
// clampRectForDraw — viewport-rect draw-layer clamp. Independent of the
// projection, but lives in minimap-draw.js, which Minimap.js reaches via
// drawViewportRect. These tests guard the clamp's pixel math.
// ---------------------------------------------------------------------------

test('clampRectForDraw: rect wider than min width passes through unchanged', () => {
    const c = clampRectForDraw({ x: 10, width: 40 }, 220);
    assert.equal(c.x, 10);
    assert.equal(c.width, 40);
});

test('clampRectForDraw: rect narrower than min width widens to min, centred on original centre', () => {
    const c = clampRectForDraw({ x: 50, width: 2 }, 220, 10);
    // centre was 51, widened to 10 should place x at 46.
    assert.equal(c.width, 10);
    assert.equal(c.x, 46);
});

test('clampRectForDraw: widened rect clamps to left edge when centre is too close to 0', () => {
    const c = clampRectForDraw({ x: 1, width: 2 }, 220, 10);
    assert.equal(c.x, 0);
    assert.equal(c.width, 10);
});

test('clampRectForDraw: widened rect clamps to right edge when centre is too close to cw', () => {
    const c = clampRectForDraw({ x: 218, width: 2 }, 220, 10);
    assert.equal(c.x, 210);
    assert.equal(c.width, 10);
});

test('clampRectForDraw: zero or negative rect width yields zero width (no draw)', () => {
    assert.deepEqual(clampRectForDraw({ x: 10, width: 0 }, 220), { x: 0, width: 0 });
    assert.deepEqual(clampRectForDraw({ x: 10, width: -5 }, 220), { x: 0, width: 0 });
});

test('clampRectForDraw: zero or negative canvas width yields zero width', () => {
    assert.deepEqual(clampRectForDraw({ x: 10, width: 20 }, 0), { x: 0, width: 0 });
    assert.deepEqual(clampRectForDraw({ x: 10, width: 20 }, -1), { x: 0, width: 0 });
});

test('clampRectForDraw: min-width capped at canvas width when canvas is tiny', () => {
    const c = clampRectForDraw({ x: 0, width: 1 }, 4, 10);
    assert.equal(c.width, 4);
    assert.equal(c.x, 0);
});

// ---------------------------------------------------------------------------
// Property tests — seeded RNG, 200 cases each.
// ---------------------------------------------------------------------------

function makeRng(seed) {
    let s = seed >>> 0;
    return () => {
        s = (s * 1664525 + 1013904223) >>> 0;
        return s / 2 ** 32;
    };
}

test('property: clampRectForDraw output rect always inside [0, cw] (200 cases)', () => {
    const rng = makeRng(0x77777777);
    for (let i = 0; i < 200; i++) {
        const cw = 40 + rng() * 460;
        const width = rng() * cw * 1.2;
        const x = -cw * 0.2 + rng() * cw * 1.4;
        const minWidth = 1 + rng() * 20;
        const c = clampRectForDraw({ x, width }, cw, minWidth);
        if (c.width === 0) continue;
        assert.ok(c.x >= -1e-9, `case ${i}: x=${c.x} < 0`);
        assert.ok(c.x + c.width <= cw + 1e-9, `case ${i}: x+w=${c.x + c.width} > cw=${cw}`);
        assert.ok(c.width > 0, `case ${i}: width collapsed to ${c.width}`);
    }
});

test('property: clampRectForDraw width never drops below min (when canvas permits) (200 cases)', () => {
    const rng = makeRng(0x88888888);
    for (let i = 0; i < 200; i++) {
        const minWidth = 2 + rng() * 30;
        const cw = minWidth + rng() * 400;
        const width = rng() * 50;
        const x = rng() * cw;
        const c = clampRectForDraw({ x, width }, cw, minWidth);
        if (c.width === 0) continue;
        assert.ok(
            c.width >= minWidth - 1e-9,
            `case ${i}: clamped width ${c.width} < minWidth ${minWidth}`,
        );
    }
});

test('property: when original rect.width already >= minWidth, clamp does not widen (200 cases)', () => {
    const rng = makeRng(0x99999999);
    for (let i = 0; i < 200; i++) {
        const minWidth = 2 + rng() * 20;
        const cw = 100 + rng() * 400;
        const width = minWidth + rng() * (cw - minWidth);
        const x = rng() * (cw - width);
        const c = clampRectForDraw({ x, width }, cw, minWidth);
        assert.ok(
            Math.abs(c.width - width) < 1e-9,
            `case ${i}: width drifted from ${width} to ${c.width}`,
        );
        assert.ok(
            Math.abs(c.x - x) < 1e-9,
            `case ${i}: x drifted from ${x} to ${c.x}`,
        );
    }
});

test('property: layout slot widths are invariant to how many entries fall in the range (200 cases)', () => {
    const rng = makeRng(0xAA00FF00);
    for (let i = 0; i < 200; i++) {
        const entryCount = 3 + Math.floor(rng() * 6);
        const stride = 5000 + rng() * 8000;
        const width = 800 + rng() * 1000;
        const entries = Array.from({ length: entryCount }, (_, k) => ({
            id: `e${k}`, x: k * stride, y: 0, width, height: 600,
        }));
        const minimapWidth = 200 + rng() * 200;
        const range = 1000 + rng() * 30000;
        const expectedSlotW = width * (minimapWidth / range);
        // Cycle the projection across each entry's centre — different entries fall
        // in/out of [start, end], but every slot's w should equal expectedSlotW.
        for (let k = 0; k < entryCount; k++) {
            const center = entries[k].x + width / 2;
            const layout = computeMinimapLinearLayout({
                entries,
                worldStart: center - range / 2,
                worldEnd: center + range / 2,
                minimapWidth,
            });
            for (const s of layout.slots) {
                assert.ok(
                    Math.abs(s.w - expectedSlotW) < 1e-6,
                    `case ${i} k=${k}: slot.w=${s.w} expected ${expectedSlotW}`,
                );
            }
        }
    }
});

test('property: rect width = viewportRange * scale, regardless of camera position (200 cases)', () => {
    const rng = makeRng(0xBEEFCAFE);
    for (let i = 0; i < 200; i++) {
        const range = 100 + rng() * 50000;
        const minimapWidth = 50 + rng() * 500;
        const scale = minimapWidth / range;
        const viewportRange = 10 + rng() * range; // can exceed strip
        const worldStart = -10000 + rng() * 20000;
        const worldEnd = worldStart + range;
        const cameraX = worldStart + rng() * range;
        const r = computeMinimapLinearRect({
            cameraX, viewportRange, worldStart, worldEnd, minimapWidth,
        });
        const expected = viewportRange * scale;
        assert.ok(
            Math.abs(r.width - expected) < 1e-6,
            `case ${i}: width=${r.width} expected ${expected}`,
        );
    }
});

// ---------------------------------------------------------------------------
// Canvas transform-state guard. Regression test: drawMinimap must not leak
// any ctx.scale/translate. Adapted to the linear layout — it just feeds the
// layout's slots into drawMinimap and asserts the CTM stays identity.
// ---------------------------------------------------------------------------

function makeMockCtx() {
    const state = { transforms: [[1, 0, 0, 1, 0, 0]], calls: [] };
    const ctx = {
        _state: state,
        save() { state.transforms.push([...state.transforms.at(-1)]); state.calls.push('save'); },
        restore() { if (state.transforms.length > 1) state.transforms.pop(); state.calls.push('restore'); },
        setTransform(a, b, c, d, e, f) { state.transforms[state.transforms.length - 1] = [a, b, c, d, e, f]; },
        getTransform() { const t = state.transforms.at(-1); return { a: t[0], b: t[1], c: t[2], d: t[3], e: t[4], f: t[5] }; },
        scale(sx, sy) { const t = state.transforms.at(-1); t[0] *= sx; t[3] *= sy; },
        translate(tx, ty) { const t = state.transforms.at(-1); t[4] += tx; t[5] += ty; },
        clearRect() {}, fillRect() {}, fill() {}, stroke() {}, fillText() {},
        beginPath() {}, closePath() {},
        moveTo() {}, lineTo() {}, quadraticCurveTo() {},
        set fillStyle(v) {}, set strokeStyle(v) {}, set lineWidth(v) {},
        set shadowColor(v) {}, set shadowBlur(v) {},
        set font(v) {}, set textAlign(v) {}, set textBaseline(v) {},
        set globalAlpha(v) {},
    };
    return ctx;
}

function isIdentity(t) {
    return Math.abs(t.a - 1) < 1e-9 && Math.abs(t.b) < 1e-9 && Math.abs(t.c) < 1e-9
        && Math.abs(t.d - 1) < 1e-9 && Math.abs(t.e) < 1e-9 && Math.abs(t.f) < 1e-9;
}

test('property: draw leaves the canvas transform as identity — no leaked scale/translate (200 cases)', () => {
    const rng = makeRng(0x0F00BA12);
    for (let i = 0; i < 200; i++) {
        const count = 1 + Math.floor(rng() * 8);
        const entries = Array.from({ length: count }, (_, k) => ({
            id: `e${k}`, x: k * (4000 + rng() * 6000), y: 0,
            width: 600 + rng() * 1200, height: 400 + rng() * 800,
        }));
        const minimapWidth = 100 + rng() * 400;
        const ch = 40 + rng() * 80;
        const last = entries.at(-1);
        const layout = computeMinimapLinearLayout({
            entries, worldStart: 0, worldEnd: last.x + last.width,
            minimapWidth, canvasHeight: ch,
        });
        if (!layout) continue;
        const activeIdx = Math.floor(rng() * entries.length);
        const activeId = entries[activeIdx].id;

        const ctx = makeMockCtx();
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        drawMinimap(ctx, minimapWidth, ch, entries, layout.slots, activeId, null);

        assert.ok(
            isIdentity(ctx.getTransform()),
            `case ${i}: non-identity transform after draw: ${JSON.stringify(ctx.getTransform())}`,
        );
        const saves = ctx._state.calls.filter(c => c === 'save').length;
        const restores = ctx._state.calls.filter(c => c === 'restore').length;
        assert.equal(saves, restores, `case ${i}: unbalanced save/restore (saves=${saves}, restores=${restores})`);
    }
});

test('drawViewportRect: at zero rect width it emits no strokes (early return)', () => {
    const ctx = makeMockCtx();
    ctx._state.moves = [];
    ctx.moveTo = (x, y) => { ctx._state.moves.push({ x, y }); };
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    drawViewportRect(ctx, 200, 60, { x: 0, width: 0, y: 0, height: 60 });
    assert.equal(ctx._state.calls.filter(c => c === 'save').length, 0);
    assert.equal(ctx._state.moves.length, 0);
});

test('VIEWPORT_MIN_WIDTH is exported as a positive number', () => {
    assert.ok(VIEWPORT_MIN_WIDTH > 0);
});
