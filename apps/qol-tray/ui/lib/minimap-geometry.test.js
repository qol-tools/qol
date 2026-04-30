import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    computeMinimapFocalLayout,
    computeMinimapFocalRect,
    computeSlotCoverage,
    FOCAL_GAP_PX,
} from './minimap-geometry.js';
import { clampRectForDraw, drawMinimap, drawViewportRect, VIEWPORT_MIN_WIDTH } from './minimap-draw.js';

const PAGE = (id, x, width = 1280, height = 900) => ({ id, x, y: 0, width, height });

test('focal layout: empty input returns null', () => {
    assert.equal(computeMinimapFocalLayout({ entries: [], activePosF: 0, minimapWidth: 200 }), null);
});

test('focal layout: invalid minimap width returns null', () => {
    assert.equal(computeMinimapFocalLayout({
        entries: [PAGE('a', 0)], activePosF: 0, minimapWidth: 0,
    }), null);
});

test('focal layout: single entry takes the full width minus gaps (gap is 0 for one entry)', () => {
    const layout = computeMinimapFocalLayout({
        entries: [PAGE('a', 0)], activePosF: 0, minimapWidth: 200,
    });
    assert.ok(Math.abs(layout.slots[0].w - 200) < 1e-6);
});

test('focal layout: active is centred at minimapWidth/2 with neighbours on both sides', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 1, minimapWidth: 380 });
    const activeCentre = layout.slots[1].x + layout.slots[1].w / 2;
    assert.ok(Math.abs(activeCentre - 190) < 1e-6, `active centre ${activeCentre} != 190`);
});

test('focal layout: at left edge active is still centred — left side has empty space', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 0, minimapWidth: 380 });
    const activeCentre = layout.slots[0].x + layout.slots[0].w / 2;
    assert.ok(Math.abs(activeCentre - 190) < 1e-6);
    assert.ok(layout.slots[0].x > 0);
});

test('focal layout: neighbour widths decay geometrically from active within focus radius', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000), PAGE('d', 30000), PAGE('e', 40000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 2, focusRadius: 5, minimapWidth: 600 });
    const w = layout.slots.map(s => s.w);
    const decay = Math.pow(0.3, 1 / 5);
    assert.ok(Math.abs(w[1] / w[2] - decay) < 1e-6);
    assert.ok(Math.abs(w[3] / w[2] - decay) < 1e-6);
    assert.ok(Math.abs(w[0] / w[2] - decay * decay) < 1e-6);
    assert.ok(Math.abs(w[4] / w[2] - decay * decay) < 1e-6);
});

test('focal layout: slot at distance > focusRadius+1 gets zero width', () => {
    const entries = Array.from({ length: 9 }, (_, i) => PAGE(`e${i}`, i * 10000));
    const layout = computeMinimapFocalLayout({
        entries, activePosF: 4, focusRadius: 1, minimapWidth: 600,
    });
    assert.equal(layout.slots[0].w, 0);
    assert.equal(layout.slots[1].w, 0);
    assert.equal(layout.slots[7].w, 0);
    assert.equal(layout.slots[8].w, 0);
    assert.ok(layout.slots[3].w > 0);
    assert.ok(layout.slots[5].w > 0);
});

test('focal layout: as activePosF slides past an integer, slot fades smoothly from 0 to full', () => {
    const entries = Array.from({ length: 9 }, (_, i) => PAGE(`e${i}`, i * 10000));
    const samples = [];
    for (let p = 1.0; p <= 2.01; p += 0.1) {
        const layout = computeMinimapFocalLayout({
            entries, activePosF: p, focusRadius: 1, minimapWidth: 600,
        });
        samples.push(layout.slots[3].w);
    }
    for (let i = 1; i < samples.length; i++) {
        assert.ok(samples[i] >= samples[i - 1] - 1e-6, `slot[3] should grow monotonically from p=1 to p=2`);
    }
});

test('focal layout: total width + gaps == minimapWidth when no floor kicks in and all slots visible', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000), PAGE('d', 30000), PAGE('e', 40000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 2, focusRadius: 10, minimapWidth: 600 });
    const totalSlots = layout.slots.reduce((a, s) => a + s.w, 0);
    const totalGaps = (entries.length - 1) * FOCAL_GAP_PX;
    assert.ok(Math.abs(totalSlots + totalGaps - 600) < 1e-6);
});

test('focal layout: gaps between adjacent slots equal FOCAL_GAP_PX', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 1, minimapWidth: 380 });
    const gap01 = layout.slots[1].x - (layout.slots[0].x + layout.slots[0].w);
    const gap12 = layout.slots[2].x - (layout.slots[1].x + layout.slots[1].w);
    assert.ok(Math.abs(gap01 - FOCAL_GAP_PX) < 1e-6);
    assert.ok(Math.abs(gap12 - FOCAL_GAP_PX) < 1e-6);
});

test('focal layout: slot aspect is FOCAL_SLOT_ASPECT regardless of entry aspect', () => {
    const tall = computeMinimapFocalLayout({
        entries: [PAGE('a', 0, 1280, 4000)], activePosF: 0, minimapWidth: 200, canvasHeight: 1000,
    });
    const wide = computeMinimapFocalLayout({
        entries: [PAGE('a', 0, 1280, 200)], activePosF: 0, minimapWidth: 200, canvasHeight: 1000,
    });
    assert.ok(Math.abs(tall.slots[0].h / tall.slots[0].w - wide.slots[0].h / wide.slots[0].w) < 1e-6);
});

test('focal layout: row shrunk when natural height exceeds canvasHeight', () => {
    const layout = computeMinimapFocalLayout({
        entries: [PAGE('a', 0, 100, 1000)], activePosF: 0, minimapWidth: 200, canvasHeight: 50,
    });
    assert.ok(layout.rowHeight <= 50 + 1e-6);
});

test('focal rect: camera fully inside active page → rect is camera span × active scale', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 1, minimapWidth: 380 });
    const rect = computeMinimapFocalRect({
        entries, slots: layout.slots, cameraX: 10000, viewportRange: 640,
    });
    const slot = layout.slots[1];
    assert.ok(Math.abs(rect.width - (640 / 1280) * slot.w) < 1e-6);
    assert.ok(Math.abs(rect.x - slot.x) < 1e-6);
});

test('focal rect: camera covering exactly one entry → rect equals that slot', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 0, minimapWidth: 200 });
    const rect = computeMinimapFocalRect({
        entries, slots: layout.slots, cameraX: 0, viewportRange: 1280,
    });
    assert.ok(Math.abs(rect.x - layout.slots[0].x) < 1e-6);
    assert.ok(Math.abs(rect.width - layout.slots[0].w) < 1e-6);
});

test('focal rect: camera spanning multiple entries unions their projected pieces', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 1, minimapWidth: 380 });
    const rect = computeMinimapFocalRect({
        entries, slots: layout.slots, cameraX: 0, viewportRange: 25000,
    });
    assert.ok(rect.x <= layout.slots[0].x + 1e-6);
    const lastEnd = layout.slots[2].x + layout.slots[2].w;
    assert.ok(rect.x + rect.width >= lastEnd - 1e-6);
});

test('focal rect: empty intersections yield empty rect', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 0, minimapWidth: 200 });
    const rect = computeMinimapFocalRect({
        entries, slots: layout.slots, cameraX: 5000, viewportRange: 100,
    });
    assert.equal(rect.x, 0);
    assert.equal(rect.width, 0);
});

test('focal layout: continuous activePosF — slot widths interpolate smoothly between integer positions', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000), PAGE('d', 30000), PAGE('e', 40000)];
    const a = computeMinimapFocalLayout({ entries, activePosF: 2, minimapWidth: 600 });
    const b = computeMinimapFocalLayout({ entries, activePosF: 2.5, minimapWidth: 600 });
    const c = computeMinimapFocalLayout({ entries, activePosF: 3, minimapWidth: 600 });
    assert.ok(b.slots[2].w < a.slots[2].w);
    assert.ok(b.slots[2].w > c.slots[2].w);
    assert.ok(b.slots[3].w > a.slots[3].w);
    assert.ok(b.slots[3].w < c.slots[3].w);
});

test('focal layout: at activePosF = 2.5, the centre point is between slots 2 and 3 at minimapWidth/2', () => {
    const entries = [PAGE('a', 0), PAGE('b', 10000), PAGE('c', 20000), PAGE('d', 30000), PAGE('e', 40000)];
    const layout = computeMinimapFocalLayout({ entries, activePosF: 2.5, minimapWidth: 600 });
    const between = (layout.slots[2].x + layout.slots[2].w + FOCAL_GAP_PX / 2);
    assert.ok(Math.abs(between - 300) < 1e-6, `between ${between} != 300`);
});

test('focal rect: zero or negative viewportRange returns null', () => {
    assert.equal(computeMinimapFocalRect({
        entries: [PAGE('a', 0)], slots: [{ x: 0, w: 100 }], cameraX: 0, viewportRange: 0,
    }), null);
});

test('coverage: slot fully inside rect returns 1', () => {
    assert.equal(computeSlotCoverage({ x: 50, w: 20 }, { x: 0, width: 100 }), 1);
});

test('coverage: slot fully outside rect returns 0', () => {
    assert.equal(computeSlotCoverage({ x: 200, w: 20 }, { x: 0, width: 100 }), 0);
});

test('coverage: half overlap returns 0.5', () => {
    assert.equal(computeSlotCoverage({ x: 0, w: 100 }, { x: 50, width: 60 }), 0.5);
});

test('coverage: missing slot/rect returns 0', () => {
    assert.equal(computeSlotCoverage(null, { x: 0, width: 10 }), 0);
    assert.equal(computeSlotCoverage({ x: 0, w: 10 }, null), 0);
    assert.equal(computeSlotCoverage({ x: 0, w: 0 }, { x: 0, width: 10 }), 0);
    assert.equal(computeSlotCoverage({ x: 0, w: 10 }, { x: 0, width: 0 }), 0);
});

test('clampRectForDraw: rect wider than min width passes through unchanged', () => {
    const c = clampRectForDraw({ x: 10, width: 40 }, 220);
    assert.equal(c.x, 10);
    assert.equal(c.width, 40);
});

test('clampRectForDraw: rect narrower than min width widens to min, centred on original centre', () => {
    const c = clampRectForDraw({ x: 50, width: 2 }, 220, 10);
    assert.equal(c.width, 10);
    assert.equal(c.x, 46);
});

test('clampRectForDraw: widened rect clamps to left edge', () => {
    const c = clampRectForDraw({ x: 1, width: 2 }, 220, 10);
    assert.equal(c.x, 0);
    assert.equal(c.width, 10);
});

test('clampRectForDraw: widened rect clamps to right edge', () => {
    const c = clampRectForDraw({ x: 218, width: 2 }, 220, 10);
    assert.equal(c.x, 210);
    assert.equal(c.width, 10);
});

test('clampRectForDraw: zero or negative rect width yields zero width', () => {
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

function makeRng(seed) {
    let s = seed >>> 0;
    return () => {
        s = (s * 1664525 + 1013904223) >>> 0;
        return s / 2 ** 32;
    };
}

test('property: active slot width is invariant under activePosF for a given layer + focusRadius (200 cases)', () => {
    const rng = makeRng(0xAA00FF00);
    for (let i = 0; i < 200; i++) {
        const N = 3 + Math.floor(rng() * 7);
        const entries = Array.from({ length: N }, (_, k) => PAGE(`e${k}`, k * 10000));
        const minimapWidth = 100 + Math.floor(rng() * 400);
        const focusRadius = 1 + rng() * 6;
        const widths = [];
        for (let k = 0; k < N; k++) {
            const layout = computeMinimapFocalLayout({
                entries, activePosF: k, focusRadius, minimapWidth, canvasHeight: 10000,
            });
            widths.push(layout.slots[k].w);
        }
        const max = Math.max(...widths);
        const min = Math.min(...widths);
        assert.ok(Math.abs(max - min) < 1e-3,
            `case ${i}: active width varies ${min}..${max} across positions (N=${N}, R=${focusRadius})`);
    }
});

test('property: active centre stays at minimapWidth/2 (200 cases)', () => {
    const rng = makeRng(0xBEEFCAFE);
    for (let i = 0; i < 200; i++) {
        const N = 2 + Math.floor(rng() * 8);
        const entries = Array.from({ length: N }, (_, k) => PAGE(`e${k}`, k * 10000));
        const minimapWidth = 100 + Math.floor(rng() * 400);
        const aIdx = Math.floor(rng() * N);
        const layout = computeMinimapFocalLayout({
            entries, activePosF: aIdx, minimapWidth, canvasHeight: 10000,
        });
        const aSlot = layout.slots[aIdx];
        const aCentre = aSlot.x + aSlot.w / 2;
        assert.ok(Math.abs(aCentre - minimapWidth / 2) < 1e-3, `case ${i}`);
    }
});

test('property: clampRectForDraw output rect inside [0, cw] (200 cases)', () => {
    const rng = makeRng(0x77777777);
    for (let i = 0; i < 200; i++) {
        const cw = 40 + rng() * 460;
        const width = rng() * cw * 1.2;
        const x = -cw * 0.2 + rng() * cw * 1.4;
        const minWidth = 1 + rng() * 20;
        const c = clampRectForDraw({ x, width }, cw, minWidth);
        if (c.width === 0) continue;
        assert.ok(c.x >= -1e-9);
        assert.ok(c.x + c.width <= cw + 1e-9);
        assert.ok(c.width > 0);
    }
});

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

test('property: drawMinimap leaves canvas transform as identity (200 cases)', () => {
    const rng = makeRng(0x0F00BA12);
    for (let i = 0; i < 200; i++) {
        const N = 1 + Math.floor(rng() * 8);
        const entries = Array.from({ length: N }, (_, k) => PAGE(`e${k}`, k * 10000));
        const minimapWidth = 100 + rng() * 400;
        const ch = 40 + rng() * 80;
        const layout = computeMinimapFocalLayout({
            entries, activePosF: Math.floor(rng() * N), minimapWidth,
            canvasHeight: ch,
        });
        if (!layout) continue;
        const ctx = makeMockCtx();
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        drawMinimap(ctx, minimapWidth, ch, entries, layout.slots, entries[0].id, null);
        assert.ok(isIdentity(ctx.getTransform()), `case ${i}`);
        const saves = ctx._state.calls.filter(c => c === 'save').length;
        const restores = ctx._state.calls.filter(c => c === 'restore').length;
        assert.equal(saves, restores, `case ${i}`);
    }
});

test('drawViewportRect: zero rect width emits no strokes', () => {
    const ctx = makeMockCtx();
    ctx._state.moves = [];
    ctx.moveTo = (x, y) => { ctx._state.moves.push({ x, y }); };
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    drawViewportRect(ctx, 200, 60, { x: 0, width: 0, y: 0, height: 60 });
    assert.equal(ctx._state.calls.filter(c => c === 'save').length, 0);
    assert.equal(ctx._state.moves.length, 0);
});

test('VIEWPORT_MIN_WIDTH > 0', () => {
    assert.ok(VIEWPORT_MIN_WIDTH > 0);
});
