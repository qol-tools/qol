import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeMinimapSlots, computeMinimapViewportRect, computeSlotCoverage } from './minimap-geometry.js';
import { cameraTargetFor } from './world-geometry.js';

const THREE_ENTRIES = [
    { id: 'a', x: 0,     y: 0, width: 1280, height: 900 },
    { id: 'b', x: 10000, y: 0, width: 1280, height: 900 },
    { id: 'c', x: 20000, y: 0, width: 1280, height: 900 },
];

test('rect spans full minimap width when camera view covers every entry', () => {
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        cameraX: 0,
        cameraZoom: 0.02,
        viewportWidthPx: 800,
        minimapWidth: 220,
    });
    assert.equal(rect.x, 0);
    assert.equal(rect.width, 220);
});

test('rect is empty when camera view is entirely before the world span', () => {
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        cameraX: -5000,
        cameraZoom: 1,
        viewportWidthPx: 800,
        minimapWidth: 220,
    });
    assert.equal(rect.width, 0);
});

test('rect clamps to the right edge when camera view extends past the last entry', () => {
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        cameraX: 10000,
        cameraZoom: 0.02,
        viewportWidthPx: 800,
        minimapWidth: 220,
    });
    assert.ok(rect.x + rect.width <= 220 + 1e-9);
    // Covers entry b (10000..11280) entirely and all of entry c
    // (20000..21280) because the camera window extends to 50000.
    assert.ok(rect.width > 100);
});

test('rect ignores the gap between entries — panning across a gap does not widen the rect', () => {
    // Camera window sits in the gap between entry a (0..1280) and entry b (10000..11280).
    // Expected: rect is empty (no entry overlap).
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        cameraX: 5000,
        cameraZoom: 1,
        viewportWidthPx: 800,
        minimapWidth: 220,
    });
    assert.equal(rect.width, 0);
});

test('rect is empty when inputs are missing or invalid', () => {
    const emptyEntries = computeMinimapViewportRect({ sortedEntries: [], cameraX: 0, cameraZoom: 1, viewportWidthPx: 800, minimapWidth: 220 });
    assert.equal(emptyEntries.x, 0);
    assert.equal(emptyEntries.width, 0);
    // cameraZoom: 0 falls back to 1 to preserve the previous "safe default"
    // semantics — an intentional choice so early frames before zoom settles
    // still draw a meaningful rect rather than collapsing to zero.
    const zoomZero = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES, cameraX: 0, cameraZoom: 0, viewportWidthPx: 800, minimapWidth: 220,
    });
    assert.ok(zoomZero.width > 0);
    const noViewport = computeMinimapViewportRect({ sortedEntries: THREE_ENTRIES, cameraX: 0, cameraZoom: 1, viewportWidthPx: 0, minimapWidth: 220 });
    assert.equal(noViewport.x, 0);
    assert.equal(noViewport.width, 0);
    const noMinimap = computeMinimapViewportRect({ sortedEntries: THREE_ENTRIES, cameraX: 0, cameraZoom: 1, viewportWidthPx: 800, minimapWidth: 0 });
    assert.equal(noMinimap.x, 0);
    assert.equal(noMinimap.width, 0);
});

// --- Property tests: seeded RNG, 200 cases each ---
// Mulberry32 — small deterministic PRNG so failures are reproducible.
function makeRng(seed) {
    let s = seed >>> 0;
    return () => {
        s = (s + 0x6D2B79F5) >>> 0;
        let t = s;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

function makeRandomEntries(rng, count) {
    const entries = [];
    let cursor = rng() * 1000;
    for (let i = 0; i < count; i++) {
        const w = 400 + rng() * 2000;
        entries.push({ id: `e${i}`, x: cursor, y: 0, width: w, height: 900 });
        cursor += w + rng() * 5000;
    }
    return entries;
}

test('property: rect is always contained within the minimap drawable area (200 cases)', () => {
    const rng = makeRng(0xCAFEBABE);
    for (let i = 0; i < 200; i++) {
        const count = 1 + Math.floor(rng() * 5);
        const entries = makeRandomEntries(rng, count);
        const worldMin = entries[0].x;
        const worldMax = entries.at(-1).x + entries.at(-1).width;
        const worldSpan = worldMax - worldMin;

        // Sample camera anywhere from 2 worldSpans before to 2 after.
        const cameraX = worldMin - worldSpan * 2 + rng() * worldSpan * 5;
        const cameraZoom = 0.001 + rng() * 4;
        const viewportWidthPx = 400 + rng() * 1600;
        const minimapWidth = 100 + rng() * 400;

        const rect = computeMinimapViewportRect({
            sortedEntries: entries,
            cameraX, cameraZoom, viewportWidthPx, minimapWidth,
        });

        assert.ok(
            rect.x >= -1e-9 && rect.x + rect.width <= minimapWidth + 1e-9 && rect.width >= -1e-9,
            `containment failed at case ${i}: rect=${JSON.stringify(rect)} minimapWidth=${minimapWidth}`,
        );
    }
});

test('property: within a single entry, doubling camera zoom halves rect width (200 cases)', () => {
    const rng = makeRng(0xFEEDBEEF);
    for (let i = 0; i < 200; i++) {
        const count = 2 + Math.floor(rng() * 4);
        const entries = makeRandomEntries(rng, count);

        // Pick one entry and stay inside its span at both zoom levels so the
        // piecewise-linear projection is a single scale factor.
        const pickIdx = Math.floor(rng() * entries.length);
        const entry = entries[pickIdx];
        const viewportWidthPx = 400 + rng() * 1600;
        const minimapWidth = 100 + rng() * 400;
        // Keep visible-world width well inside the entry at the lower zoom
        // (zoomOut), so both r1 and r2 stay within that entry.
        const zoomIn = 10 + rng() * 20;
        const zoomOut = zoomIn / 2;
        const visibleAtOut = viewportWidthPx / zoomOut;
        if (visibleAtOut * 1.1 > entry.width) continue; // need headroom
        const minCamX = entry.x;
        const maxCamX = entry.x + entry.width - visibleAtOut * 1.05;
        if (maxCamX <= minCamX) continue;
        const cameraX = minCamX + rng() * (maxCamX - minCamX);

        const r1 = computeMinimapViewportRect({
            sortedEntries: entries, cameraX, cameraZoom: zoomOut, viewportWidthPx, minimapWidth,
        });
        const r2 = computeMinimapViewportRect({
            sortedEntries: entries, cameraX, cameraZoom: zoomIn, viewportWidthPx, minimapWidth,
        });

        assert.ok(r1.width > 1e-6, `case ${i}: r1 collapsed`);
        const ratio = r2.width / r1.width;
        assert.ok(
            Math.abs(ratio - 0.5) < 1e-6,
            `halving failed case ${i}: r1.w=${r1.width} r2.w=${r2.width} ratio=${ratio}`,
        );
    }
});

test('property: within a single entry, panning camera by dx translates rect by dx/e.width * slot.w (200 cases)', () => {
    const rng = makeRng(0x12345678);
    for (let i = 0; i < 200; i++) {
        const count = 2 + Math.floor(rng() * 4);
        const entries = makeRandomEntries(rng, count);

        const pickIdx = Math.floor(rng() * entries.length);
        const entry = entries[pickIdx];
        const viewportWidthPx = 400 + rng() * 1600;
        const minimapWidth = 100 + rng() * 400;
        const cameraZoom = 5 + rng() * 20;
        const visibleWorld = viewportWidthPx / cameraZoom;
        if (visibleWorld * 2 > entry.width) continue; // need room for pan

        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });
        const slot = slots[pickIdx];

        const safeMin = entry.x;
        const safeMax = entry.x + entry.width - visibleWorld;
        if (safeMax <= safeMin) continue;
        const cameraX = safeMin + rng() * (safeMax - safeMin) * 0.4;
        const remaining = safeMax - cameraX;
        if (remaining < 1) continue;
        const dx = rng() * remaining * 0.4;

        const r1 = computeMinimapViewportRect({
            sortedEntries: entries, cameraX, cameraZoom, viewportWidthPx, minimapWidth,
        });
        const r2 = computeMinimapViewportRect({
            sortedEntries: entries, cameraX: cameraX + dx, cameraZoom, viewportWidthPx, minimapWidth,
        });

        const scale = slot.w / entry.width;
        const expectedDeltaX = dx * scale;
        const actualDeltaX = r2.x - r1.x;
        assert.ok(
            Math.abs(actualDeltaX - expectedDeltaX) < 1e-6,
            `translation failed case ${i}: dx=${dx} scale=${scale} expectedDelta=${expectedDeltaX} actualDelta=${actualDeltaX}`,
        );
        assert.ok(
            Math.abs(r2.width - r1.width) < 1e-6,
            `width changed on pan case ${i}: r1.w=${r1.width} r2.w=${r2.width}`,
        );
    }
});

// --- computeMinimapSlots: geometric slot layout ---

test('slots: empty input yields empty slots', () => {
    assert.deepEqual(computeMinimapSlots({ sortedEntries: [], minimapWidth: 220 }), []);
});

test('slots: invalid minimap width yields empty slots', () => {
    assert.deepEqual(computeMinimapSlots({ sortedEntries: THREE_ENTRIES, minimapWidth: 0 }), []);
});

test('slots: equal-width entries each get equal fraction of minimap (gaps collapsed)', () => {
    const slots = computeMinimapSlots({ sortedEntries: THREE_ENTRIES, minimapWidth: 220 });
    assert.equal(slots.length, 3);
    // Each entry is 1280 wide, total = 3840, scale = 220/3840, slot.w ≈ 73.33
    const expectedW = 220 / 3;
    for (let i = 0; i < 3; i++) {
        assert.ok(Math.abs(slots[i].w - expectedW) < 1e-6, `slot ${i} width: ${slots[i].w} vs ${expectedW}`);
    }
    assert.ok(Math.abs(slots[0].x) < 1e-9);
    assert.ok(Math.abs((slots[2].x + slots[2].w) - 220) < 1e-9);
});

test('slots: variable entry widths produce proportional slots', () => {
    const entries = [
        { id: 'a', x: 0, y: 0, width: 1000, height: 800 },
        { id: 'b', x: 3000, y: 0, width: 500, height: 800 },
        { id: 'c', x: 8000, y: 0, width: 2000, height: 800 },
    ];
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth: 350 });
    // Total width = 3500, scale = 0.1
    assert.ok(Math.abs(slots[0].w - 100) < 1e-6);
    assert.ok(Math.abs(slots[1].w - 50) < 1e-6);
    assert.ok(Math.abs(slots[2].w - 200) < 1e-6);
    // Packed left-to-right
    assert.ok(Math.abs(slots[0].x - 0) < 1e-6);
    assert.ok(Math.abs(slots[1].x - 100) < 1e-6);
    assert.ok(Math.abs(slots[2].x - 150) < 1e-6);
});

test('slots: preserve order matching sortedEntries', () => {
    const entries = [
        { id: 'a', x: 0, y: 0, width: 1000, height: 800 },
        { id: 'b', x: 3000, y: 0, width: 500, height: 800 },
        { id: 'c', x: 8000, y: 0, width: 2000, height: 800 },
    ];
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth: 400 });
    for (let i = 0; i < entries.length - 1; i++) {
        assert.ok(slots[i].x <= slots[i + 1].x, `out of order at ${i}`);
    }
});

test('property: when camera covers exactly one entry (zoom=1, viewport=entry.width) at cameraTargetFor, rect range equals that entry slot range (200 cases)', () => {
    const rng = makeRng(0xAB1EC0FE);
    for (let i = 0; i < 200; i++) {
        const count = 1 + Math.floor(rng() * 5);
        const entries = makeRandomEntries(rng, count);
        const minimapWidth = 100 + rng() * 400;
        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });

        for (let j = 0; j < entries.length; j++) {
            const entry = entries[j];
            const viewportWidthPx = entry.width;
            const zoom = 1;
            // camera snug on the entry: camera window = [entry.x, entry.x + entry.width]
            const cam = cameraTargetFor(entry, viewportWidthPx, 0, zoom);
            const rect = computeMinimapViewportRect({
                sortedEntries: entries,
                cameraX: cam.x,
                cameraZoom: zoom,
                viewportWidthPx,
                minimapWidth,
            });
            const slot = slots[j];
            assert.ok(
                Math.abs(rect.x - slot.x) < 1e-6,
                `case ${i}, entry ${j}: rect.x=${rect.x} slot.x=${slot.x}`,
            );
            assert.ok(
                Math.abs(rect.width - slot.w) < 1e-6,
                `case ${i}, entry ${j}: rect.width=${rect.width} slot.w=${slot.w}`,
            );
        }
    }
});

test('slots: layout is independent of any "active" concern — same input always yields same slots', () => {
    // Active-slot emphasis lives in Minimap.js draw code (visual scale),
    // not here. If this ever changes — e.g. someone adds an activeId param
    // to computeMinimapSlots and uses it to resize — the viewport rect
    // alignment contract breaks. This guards against that.
    const slotsA = computeMinimapSlots({ sortedEntries: THREE_ENTRIES, minimapWidth: 220 });
    const slotsB = computeMinimapSlots({ sortedEntries: THREE_ENTRIES, minimapWidth: 220 });
    assert.deepEqual(slotsA, slotsB);
    // computeMinimapSlots takes no activeId/active/selected parameter.
    assert.equal(computeMinimapSlots.length, 1);
});

test('property: slots always cover the full minimap width with no gaps (200 cases)', () => {
    const rng = makeRng(0xDEADF00D);
    for (let i = 0; i < 200; i++) {
        const count = 2 + Math.floor(rng() * 6);
        const entries = makeRandomEntries(rng, count);
        const minimapWidth = 100 + rng() * 400;
        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });

        // Every slot butts up exactly against the next (gaps collapsed).
        for (let j = 0; j < slots.length - 1; j++) {
            assert.ok(
                Math.abs((slots[j].x + slots[j].w) - slots[j + 1].x) < 1e-6,
                `gap/overlap at ${j}: end=${slots[j].x + slots[j].w} nextStart=${slots[j + 1].x}`,
            );
        }
        assert.ok(Math.abs(slots[0].x) < 1e-6, `first slot not at 0: ${slots[0].x}`);
        const last = slots[slots.length - 1];
        assert.ok(Math.abs((last.x + last.w) - minimapWidth) < 1e-6, `last slot not at end: ${last.x + last.w} vs ${minimapWidth}`);
    }
});

test('property: slot aspect matches entry aspect when entries share aspect (200 cases)', () => {
    const rng = makeRng(0xA5F3C10D);
    for (let i = 0; i < 200; i++) {
        const count = 1 + Math.floor(rng() * 6);
        // All entries share the same aspect ratio.
        const pageW = 400 + rng() * 2000;
        const pageH = 200 + rng() * 1800;
        const entries = [];
        let cursor = rng() * 1000;
        for (let j = 0; j < count; j++) {
            const w = pageW * (0.9 + rng() * 0.2);
            const h = w * (pageH / pageW);
            entries.push({ id: `e${j}`, x: cursor, y: 0, width: w, height: h });
            cursor += w + rng() * 5000;
        }
        const minimapWidth = 100 + rng() * 400;
        const canvasHeight = 40 + rng() * 200;
        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth, canvasHeight });
        const expectedAspect = entries[0].width / entries[0].height;
        for (let j = 0; j < slots.length; j++) {
            if (!(slots[j].h > 0)) continue;
            const slotAspect = slots[j].w / slots[j].h;
            assert.ok(
                Math.abs(slotAspect - expectedAspect) < 1e-6,
                `case ${i}, slot ${j}: aspect ${slotAspect} vs expected ${expectedAspect}`,
            );
        }
    }
});

test('property: slot row stays inside [0, canvasHeight] and is vertically centred (200 cases)', () => {
    const rng = makeRng(0xB0CAF00D);
    for (let i = 0; i < 200; i++) {
        const count = 1 + Math.floor(rng() * 6);
        const entries = makeRandomEntries(rng, count);
        const minimapWidth = 100 + rng() * 400;
        const canvasHeight = 40 + rng() * 300;
        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth, canvasHeight });
        if (slots.length === 0) continue;
        const rowY = slots[0].y;
        const rowH = slots[0].h;
        // All slots share the same row y.
        for (let j = 0; j < slots.length; j++) {
            assert.ok(Math.abs(slots[j].y - rowY) < 1e-9, `case ${i} slot ${j} y mismatch`);
        }
        const maxH = slots.reduce((m, s) => Math.max(m, s.h), 0);
        assert.ok(rowY >= -1e-9, `case ${i}: rowY=${rowY} < 0`);
        assert.ok(rowY + maxH <= canvasHeight + 1e-6, `case ${i}: rowY+h=${rowY + maxH} > canvasHeight=${canvasHeight}`);
        // Centred: distance from top equals distance from bottom (within 1e-6).
        const topGap = rowY;
        const bottomGap = canvasHeight - (rowY + maxH);
        assert.ok(
            Math.abs(topGap - bottomGap) < 1e-6,
            `case ${i}: not centred top=${topGap} bottom=${bottomGap}`,
        );
    }
});

test('slots: canvasHeight caps row height and shrinks width proportionally to preserve aspect', () => {
    // Tall page (portrait) + wide minimap + small canvas — expect shrink.
    const entries = [
        { id: 'a', x: 0, y: 0, width: 100, height: 1000 },
    ];
    const minimapWidth = 400;
    const canvasHeight = 50;
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth, canvasHeight });
    assert.equal(slots.length, 1);
    // Unscaled slot.w would be 400 (fills minimap), slot.h would be 4000 (way > 50).
    // Shrink scale = 50/4000 = 0.0125. Shrunk w=5, h=50. Aspect preserved.
    assert.ok(Math.abs(slots[0].h - canvasHeight) < 1e-6, `capped h=${slots[0].h}`);
    const aspect = slots[0].w / slots[0].h;
    const expectedAspect = 100 / 1000;
    assert.ok(Math.abs(aspect - expectedAspect) < 1e-6, `aspect ${aspect} vs ${expectedAspect}`);
});

test('slots: with sparse world layout (stride >> width), slots still pack to fill the minimap', () => {
    // Regression guard: this is the production scenario — 8 pages at stride 10000 with
    // width 1280. A global linear projection collapses each slot to ~6px.
    // Piecewise projection gives each slot (minimapWidth / 8) pixels.
    const entries = Array.from({ length: 8 }, (_, i) => ({
        id: `p${i}`, x: i * 10000, y: 0, width: 1280, height: 900,
    }));
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth: 380 });
    const expectedW = 380 / 8;
    assert.ok(Math.abs(slots[0].w - expectedW) < 1e-6);
    assert.ok(slots[0].w > 40, `slot too narrow for labels: ${slots[0].w}`);
});

// --- Canvas transform state guard --------------------------------------
// Regression guard for future drift: if someone reintroduces a ctx.scale()
// or ctx.translate() to emphasise the active slot and forgets to wrap it
// in save/restore, subsequent draws will render against a mutated CTM.
// The draw code currently computes scaled coordinates manually and never
// touches the CTM; this test locks that in.

import {
    computeLayerPulse,
    drawMinimap,
    drawViewportRect,
    clampRectForDraw,
    LAYER_PULSE_MS,
    VIEWPORT_MIN_WIDTH,
} from './minimap-draw.js';

function makeMockCtx() {
    const state = { transforms: [[1, 0, 0, 1, 0, 0]], calls: [] };
    const ctx = {
        _state: state,
        save() { state.transforms.push([...state.transforms.at(-1)]); state.calls.push('save'); },
        restore() { if (state.transforms.length > 1) state.transforms.pop(); state.calls.push('restore'); },
        setTransform(a, b, c, d, e, f) { state.transforms[state.transforms.length - 1] = [a, b, c, d, e, f]; },
        getTransform() { const t = state.transforms.at(-1); return { a: t[0], b: t[1], c: t[2], d: t[3], e: t[4], f: t[5] }; },
        scale(sx, sy) {
            const t = state.transforms.at(-1);
            t[0] *= sx; t[3] *= sy;
        },
        translate(tx, ty) {
            const t = state.transforms.at(-1);
            t[4] += tx; t[5] += ty;
        },
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
        const entries = makeRandomEntries(rng, count);
        const minimapWidth = 100 + rng() * 400;
        const ch = 40 + rng() * 80;
        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });
        const activeIdx = Math.floor(rng() * entries.length);
        const activeId = entries[activeIdx].id;

        const ctx = makeMockCtx();
        // Simulate the one setTransform the renderer does before drawing.
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        drawMinimap(ctx, minimapWidth, ch, entries, slots, activeId, null);

        assert.ok(
            isIdentity(ctx.getTransform()),
            `case ${i}: non-identity transform after draw: ${JSON.stringify(ctx.getTransform())}`,
        );
        // Every save must be matched by a restore.
        const saves = ctx._state.calls.filter(c => c === 'save').length;
        const restores = ctx._state.calls.filter(c => c === 'restore').length;
        assert.equal(saves, restores, `case ${i}: unbalanced save/restore (saves=${saves}, restores=${restores})`);
    }
});

// --- clampRectForDraw: viewport-rect draw-layer clamp ----------------
// The viewport rect would otherwise collapse to a single-pixel sliver at high
// zoom (camera narrower than one slot). Clamp widens it to a minimum while
// preserving the centre, then keeps it inside the canvas.

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
    // centre = 2; naïve x = 2 - 5 = -3 → clamped to 0.
    assert.equal(c.x, 0);
    assert.equal(c.width, 10);
});

test('clampRectForDraw: widened rect clamps to right edge when centre is too close to cw', () => {
    const c = clampRectForDraw({ x: 218, width: 2 }, 220, 10);
    // centre = 219; naïve x = 214, x+width = 224 > cw=220 → x = 210.
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
    // Canvas is narrower than the default 8px minimum — target must fit inside.
    const c = clampRectForDraw({ x: 0, width: 1 }, 4, 10);
    assert.equal(c.width, 4);
    assert.equal(c.x, 0);
});

test('property: clampRectForDraw output rect always inside [0, cw] (200 cases)', () => {
    const rng = makeRng(0x77777777);
    for (let i = 0; i < 200; i++) {
        const cw = 40 + rng() * 460;
        // Sample rect with x possibly outside canvas, width spanning 0..cw.
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
        // Ensure canvas >= minWidth so the clamp can always satisfy the floor.
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
        // Pick x so the rect stays inside — no clamp-to-edge wobble here.
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

// --- End-to-end draw integration: rect at zoom=1 snug on entry -------
// Ties the clamp + draw layer back to the geometry invariant so we catch
// drift between computeMinimapViewportRect and clampRectForDraw.

function makeMockCtxWithPaths() {
    const ctx = makeMockCtx();
    ctx._state.moves = [];
    ctx._state.lines = [];
    ctx.moveTo = function (x, y) { ctx._state.moves.push({ x, y }); };
    ctx.lineTo = function (x, y) { ctx._state.lines.push({ x, y }); };
    return ctx;
}

// roundRect path layout: moveTo(x+r, y), then lineTo(x+w-r, y), ...
// So the first moveTo of each roundRect captures the rect's left edge + radius.
test('drawViewportRect: at zoom 1 snug on a single entry, rect width matches slot width (no clamp applied)', () => {
    const entries = [
        { id: 'a', x: 0, y: 0, width: 1280, height: 900 },
        { id: 'b', x: 10000, y: 0, width: 1280, height: 900 },
    ];
    const minimapWidth = 220;
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });
    const entry = entries[0];
    const cam = cameraTargetFor(entry, entry.width, 0, 1);
    const rect = computeMinimapViewportRect({
        sortedEntries: entries,
        cameraX: cam.x,
        cameraZoom: 1,
        viewportWidthPx: entry.width,
        minimapWidth,
    });
    // Pre-condition: geometry invariant holds.
    assert.ok(Math.abs(rect.x - slots[0].x) < 1e-6);
    assert.ok(Math.abs(rect.width - slots[0].w) < 1e-6);
    // Clamp must not widen past the slot here — slot is ~110 px >> 8 px min.
    const c = clampRectForDraw(rect, minimapWidth);
    assert.ok(Math.abs(c.x - rect.x) < 1e-9, `x drifted: rect.x=${rect.x} clamped.x=${c.x}`);
    assert.ok(Math.abs(c.width - rect.width) < 1e-9, `width drifted: rect.w=${rect.width} clamped.w=${c.width}`);
});

test('drawViewportRect: at zero rect width it emits no strokes (early return)', () => {
    const ctx = makeMockCtxWithPaths();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    drawViewportRect(ctx, 200, 60, { x: 0, width: 0, y: 0, height: 60 });
    assert.equal(ctx._state.calls.filter(c => c === 'save').length, 0);
    assert.equal(ctx._state.moves.length, 0);
});

test('drawViewportRect: at very high zoom (rect < 8px) the drawn path widens to minimum', () => {
    // Fabricate a rect narrower than the 8px minimum. Draw must widen it.
    const ctx = makeMockCtxWithPaths();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    drawViewportRect(ctx, 220, 60, { x: 110, width: 2, y: 10, height: 40 });
    // Two roundRect paths were drawn (fill + stroke). Each path's first moveTo
    // is at (x + RADIUS, y). So we can recover the drawn x from moveTo[0].
    assert.ok(ctx._state.moves.length >= 1);
    // Both paths share the same x/w; re-derive by reading line endpoints.
    // moveTo: (x+r, y). First lineTo: (x+w-r, y). Difference = w - 2r.
    const mv0 = ctx._state.moves[0];
    const ln0 = ctx._state.lines[0];
    const derivedWidth = (ln0.x - mv0.x) + 2 * 3; // RADIUS = 3
    assert.ok(
        derivedWidth >= VIEWPORT_MIN_WIDTH - 1e-6,
        `drawn width ${derivedWidth} should be >= ${VIEWPORT_MIN_WIDTH}`,
    );
});

// --- computeSlotCoverage: per-slot coverage for opacity fade -----------
// Coverage drives the minimap strip's opacity — slots inside the camera
// window draw at full alpha, slots outside fade to a floor. These tests
// lock the shape of that contract (and guard the zoom=1 snug regression).

test('computeSlotCoverage: rect fully containing slot → coverage = 1', () => {
    const slot = { x: 30, y: 0, w: 40, h: 10 };
    const rect = { x: 0, y: 0, width: 200, height: 10 };
    assert.equal(computeSlotCoverage(slot, rect), 1);
});

test('computeSlotCoverage: rect fully outside slot (left) → coverage = 0', () => {
    const slot = { x: 100, y: 0, w: 40, h: 10 };
    const rect = { x: 0, y: 0, width: 50, height: 10 };
    assert.equal(computeSlotCoverage(slot, rect), 0);
});

test('computeSlotCoverage: rect fully outside slot (right) → coverage = 0', () => {
    const slot = { x: 100, y: 0, w: 40, h: 10 };
    const rect = { x: 200, y: 0, width: 50, height: 10 };
    assert.equal(computeSlotCoverage(slot, rect), 0);
});

test('computeSlotCoverage: rect flush against slot edge (no overlap) → coverage = 0', () => {
    const slot = { x: 100, y: 0, w: 40, h: 10 };
    // rect ends exactly at slot.x — no overlap.
    assert.equal(computeSlotCoverage(slot, { x: 50, y: 0, width: 50, height: 10 }), 0);
    // rect starts exactly at slot.x + slot.w — no overlap.
    assert.equal(computeSlotCoverage(slot, { x: 140, y: 0, width: 50, height: 10 }), 0);
});

test('computeSlotCoverage: rect fully inside slot → coverage = rect.width / slot.w', () => {
    const slot = { x: 100, y: 0, w: 40, h: 10 };
    const rect = { x: 110, y: 0, width: 20, height: 10 };
    assert.ok(Math.abs(computeSlotCoverage(slot, rect) - 0.5) < 1e-9);
});

test('computeSlotCoverage: rect overlapping 50% of slot → coverage = 0.5', () => {
    const slot = { x: 100, y: 0, w: 40, h: 10 };
    // rect covers right half of slot (x: 120..140).
    const rect = { x: 120, y: 0, width: 1000, height: 10 };
    assert.ok(Math.abs(computeSlotCoverage(slot, rect) - 0.5) < 1e-9);
});

test('computeSlotCoverage: rect partial-overlap from the left → coverage reflects fraction', () => {
    const slot = { x: 100, y: 0, w: 40, h: 10 };
    // rect spans 80..110 — 10 px inside the slot out of 40 → 0.25.
    const rect = { x: 80, y: 0, width: 30, height: 10 };
    assert.ok(Math.abs(computeSlotCoverage(slot, rect) - 0.25) < 1e-9);
});

test('computeSlotCoverage: zero-width slot → 0 (guard against div by zero)', () => {
    assert.equal(computeSlotCoverage({ x: 10, y: 0, w: 0, h: 10 }, { x: 0, width: 100 }), 0);
});

test('computeSlotCoverage: zero-width rect → 0', () => {
    assert.equal(computeSlotCoverage({ x: 10, y: 0, w: 40, h: 10 }, { x: 20, width: 0 }), 0);
});

test('computeSlotCoverage: null/undefined inputs → 0', () => {
    assert.equal(computeSlotCoverage(null, { x: 0, width: 10 }), 0);
    assert.equal(computeSlotCoverage({ x: 0, y: 0, w: 10, h: 10 }, null), 0);
    assert.equal(computeSlotCoverage(undefined, undefined), 0);
});

test('property: computeSlotCoverage ∈ [0, 1] for any slot/rect combination (200 cases)', () => {
    const rng = makeRng(0xC0C0C0FE);
    for (let i = 0; i < 200; i++) {
        const slot = {
            x: -100 + rng() * 400,
            y: 0,
            w: rng() * 120, // includes zero-width slots
            h: 10,
        };
        const rect = {
            x: -100 + rng() * 400,
            y: 0,
            width: -20 + rng() * 240, // includes zero/negative widths
            height: 10,
        };
        const c = computeSlotCoverage(slot, rect);
        assert.ok(c >= 0 && c <= 1, `case ${i}: coverage=${c} out of [0,1], slot=${JSON.stringify(slot)} rect=${JSON.stringify(rect)}`);
    }
});

test('property: at zoom=1 snug on entry[i], coverage of slot[i] === 1 and all other slots === 0 (200 cases)', () => {
    // This is the load-bearing invariant for the opacity-fade feature: when
    // the user is snug on a single page, exactly that slot reads "fully lit"
    // and every other slot reads "fully dimmed". Any regression in the
    // projection (piecewise linearity, scale alignment) breaks this.
    const rng = makeRng(0x1507A611);
    for (let i = 0; i < 200; i++) {
        const count = 1 + Math.floor(rng() * 5);
        const entries = makeRandomEntries(rng, count);
        const minimapWidth = 100 + rng() * 400;
        const canvasHeight = 40 + rng() * 100;
        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth, canvasHeight });

        for (let j = 0; j < entries.length; j++) {
            const entry = entries[j];
            const viewportWidthPx = entry.width;
            const cam = cameraTargetFor(entry, viewportWidthPx, 0, 1);
            const rect = computeMinimapViewportRect({
                sortedEntries: entries,
                cameraX: cam.x,
                cameraZoom: 1,
                viewportWidthPx,
                minimapWidth,
                canvasHeight,
            });
            for (let k = 0; k < slots.length; k++) {
                const c = computeSlotCoverage(slots[k], rect);
                if (k === j) {
                    assert.ok(
                        Math.abs(c - 1) < 1e-6,
                        `case ${i} entry ${j}: own slot coverage=${c}, expected 1`,
                    );
                } else {
                    assert.ok(
                        c < 1e-6,
                        `case ${i} entry ${j} vs slot ${k}: coverage=${c}, expected 0`,
                    );
                }
            }
        }
    }
});

test('computeSlotCoverage: at zoom 0.5 spanning entries i and i+1, both slots register partial coverage', () => {
    // Camera window at zoom 0.5 spans 2× viewportWidthPx worth of world width.
    // With viewport = entry.width, this spans exactly 2 entries when positioned
    // at the first entry's start. Both slots should have coverage > 0.
    const entries = [
        { id: 'a', x: 0, y: 0, width: 1000, height: 800 },
        { id: 'b', x: 1000, y: 0, width: 1000, height: 800 },
        { id: 'c', x: 2000, y: 0, width: 1000, height: 800 },
    ];
    const minimapWidth = 300;
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });
    const rect = computeMinimapViewportRect({
        sortedEntries: entries,
        cameraX: 0,
        cameraZoom: 0.5,
        viewportWidthPx: 1000,
        minimapWidth,
    });
    // Entry a and b are both fully inside the camera window → coverage 1 each.
    assert.ok(Math.abs(computeSlotCoverage(slots[0], rect) - 1) < 1e-6);
    assert.ok(Math.abs(computeSlotCoverage(slots[1], rect) - 1) < 1e-6);
    // Entry c is fully outside → coverage 0.
    assert.ok(computeSlotCoverage(slots[2], rect) < 1e-6);
});

// --- computeLayerPulse: layer-change visual cue ----------------------------
// The viewport rect's geometric position can land on the same slot pre and
// post a dive (Plugins → plugin → Plugins: every plugin sub-page is anchored
// at the Plugins entry's x, so the rect always projects to the same Plugins
// slot at root). Without a non-geometric cue the user perceives the rect as
// "frozen". The pulse provides that cue. These tests lock the contract so
// future changes don't accidentally suppress the cue or leave it always on.

test('computeLayerPulse: returns null when no pulse is armed (start = null)', () => {
    assert.equal(computeLayerPulse(1000, null), null);
    assert.equal(computeLayerPulse(1000, undefined), null);
});

test('computeLayerPulse: returns null when pulse window has elapsed', () => {
    assert.equal(computeLayerPulse(1000 + LAYER_PULSE_MS, 1000), null);
    assert.equal(computeLayerPulse(1000 + LAYER_PULSE_MS + 1, 1000), null);
});

test('computeLayerPulse: returns null when nowMs is before start (clock skew guard)', () => {
    // Negative t should be treated as not-yet-pulsing; we hand back null so
    // the draw layer stays at baseline rather than fabricating a glow.
    assert.equal(computeLayerPulse(900, 1000), null);
});

test('computeLayerPulse: returns null when duration is zero or negative', () => {
    assert.equal(computeLayerPulse(1000, 990, 0), null);
    assert.equal(computeLayerPulse(1000, 990, -100), null);
});

test('computeLayerPulse: at t=0 the boost is at its peak', () => {
    const pulse = computeLayerPulse(1000, 1000);
    assert.ok(pulse, 'pulse is active at t=0');
    assert.ok(Math.abs(pulse.alphaBoost - 0.20) < 1e-9, `alphaBoost ${pulse.alphaBoost}`);
    assert.ok(Math.abs(pulse.blurExtra - 14) < 1e-9, `blurExtra ${pulse.blurExtra}`);
    assert.ok(Math.abs(pulse.widthBoost - 1.5) < 1e-9, `widthBoost ${pulse.widthBoost}`);
    assert.equal(pulse.progress, 0);
});

test('computeLayerPulse: monotonically decays toward zero across the window', () => {
    const start = 5000;
    const samples = [0, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99].map(f => start + f * LAYER_PULSE_MS);
    const pulses = samples.map(t => computeLayerPulse(t, start));
    for (let i = 1; i < pulses.length; i++) {
        assert.ok(
            pulses[i].alphaBoost <= pulses[i - 1].alphaBoost + 1e-9,
            `alphaBoost not monotone at i=${i}: ${pulses[i - 1].alphaBoost} → ${pulses[i].alphaBoost}`,
        );
        assert.ok(
            pulses[i].blurExtra <= pulses[i - 1].blurExtra + 1e-9,
            `blurExtra not monotone at i=${i}`,
        );
        assert.ok(
            pulses[i].widthBoost <= pulses[i - 1].widthBoost + 1e-9,
            `widthBoost not monotone at i=${i}`,
        );
    }
    // Last sample (just before window end) is close to zero.
    const tail = pulses.at(-1);
    assert.ok(tail.alphaBoost < 0.01, `tail alphaBoost ${tail.alphaBoost} should be near zero`);
});

test('property: pulse boosts are always non-negative inside the window (200 cases)', () => {
    const rng = makeRng(0xBADC0DE5);
    const start = 1000;
    for (let i = 0; i < 200; i++) {
        const t = start + rng() * LAYER_PULSE_MS;
        const pulse = computeLayerPulse(t, start);
        assert.ok(pulse, `case ${i}: expected pulse at t=${t}`);
        assert.ok(pulse.alphaBoost >= -1e-9, `case ${i}: alphaBoost=${pulse.alphaBoost} < 0`);
        assert.ok(pulse.blurExtra >= -1e-9, `case ${i}: blurExtra=${pulse.blurExtra} < 0`);
        assert.ok(pulse.widthBoost >= -1e-9, `case ${i}: widthBoost=${pulse.widthBoost} < 0`);
        assert.ok(pulse.progress >= 0 && pulse.progress < 1, `case ${i}: progress=${pulse.progress} outside [0,1)`);
    }
});

// --- Slot-scale inflation: rect tracks visible content, not raw camera window
// When the world camera zooms below ghostThreshold and uiScaleOnZoomOut is on,
// individual page slots are CSS-scaled around their centre so they stay
// readable. The scaled slot's VISUAL world-x bounds extend past the entry's
// declared `x..x+width`. The rect therefore accepts an optional `inflatedRanges`
// array; when supplied, overlap is checked against those bounds instead of raw
// entry bounds. Without this, the rect under-represents the camera framing —
// the user sees a neighbour page on screen, but the rect stays on the active
// slot only.

import {
    computeBaseScale,
    computeSlotScale,
    inflatedEntryRange,
} from './world-geometry.js';

test('rect with inflatedRanges null behaves identically to legacy (raw entry bounds)', () => {
    // Sanity guard that the additive parameter is back-compat: omitting it OR
    // passing `null` must not change anything. This protects every existing
    // caller (and existing test) from silent behaviour drift.
    const before = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        cameraX: 200,
        cameraZoom: 1,
        viewportWidthPx: 800,
        minimapWidth: 220,
    });
    const after = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        cameraX: 200,
        cameraZoom: 1,
        viewportWidthPx: 800,
        minimapWidth: 220,
        inflatedRanges: null,
    });
    assert.deepEqual(before, after);
});

test('rect uses inflatedRanges when supplied — neighbour visible via slot-scale shows up in rect', () => {
    // Camera world window covers Plugins only, but Plugin Store's slot is
    // CSS-scaled big enough that its visual range overlaps the camera window.
    // Expected: rect crosses into Store slot too (bug fix).
    const entries = [
        { id: 'plugins', x: 0,     y: 0, width: 1280, height: 900 },
        { id: 'store',   x: 10000, y: 0, width: 1280, height: 900 },
        { id: 'hotkeys', x: 20000, y: 0, width: 1280, height: 900 },
    ];
    // Plugins centre 640, Store centre 10640.
    // Inflated Store range at slotScale=4: 10640 ± 2560 = [8080, 13200].
    const inflatedRanges = [
        { x0: -2240, x1: 3520 },   // Plugins inflated to slotScale=4
        { x0: 8080,  x1: 13200 },  // Store inflated to slotScale=4
        { x0: 20000, x1: 21280 },  // Hotkeys at scale=1 (far from camera)
    ];
    // Camera window [-1500, 8500] at zoom=0.12. Camera window:
    //   - overlaps Plugins inflated [-2240, 3520] → [-1500, 3520]
    //   - overlaps Store   inflated [8080, 13200] → [8080, 8500]
    //   - misses Hotkeys
    const minimapWidth = 300;
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });
    const inflated = computeMinimapViewportRect({
        sortedEntries: entries,
        cameraX: -1500,
        cameraZoom: 0.12,
        viewportWidthPx: 1200,
        minimapWidth,
        inflatedRanges,
    });
    // Rect must reach into Store slot — that's the load-bearing assertion.
    assert.ok(
        inflated.x + inflated.width >= slots[1].x + 1e-6,
        `rect end ${inflated.x + inflated.width} should reach Store slot at ${slots[1].x}`,
    );
    // Rect must overlap Plugins slot (x falls inside Plugins slot range).
    assert.ok(
        inflated.x < slots[0].x + slots[0].w + 1e-6,
        `rect.x ${inflated.x} should be inside Plugins slot [${slots[0].x}, ${slots[0].x + slots[0].w}]`,
    );
    // For comparison, the legacy raw computation would only land on Plugins.
    const raw = computeMinimapViewportRect({
        sortedEntries: entries,
        cameraX: -1500,
        cameraZoom: 0.12,
        viewportWidthPx: 1200,
        minimapWidth,
    });
    assert.ok(
        raw.x + raw.width <= slots[1].x + 1e-6,
        `raw rect right edge ${raw.x + raw.width} should NOT reach Store slot at ${slots[1].x}`,
    );
});

test('rect at zoom=1 (no inflation) is identical with or without inflatedRanges supplied', () => {
    // Property guard: at zoom >= ghostThreshold, slotScale === 1 for every
    // entry, so inflatedRanges equals raw entry bounds. The rect must not
    // change. This prevents drift between the two code paths.
    const entries = THREE_ENTRIES;
    const ranges = entries.map(e => ({ x0: e.x, x1: e.x + e.width }));
    const a = computeMinimapViewportRect({
        sortedEntries: entries,
        cameraX: 200,
        cameraZoom: 1,
        viewportWidthPx: 800,
        minimapWidth: 220,
    });
    const b = computeMinimapViewportRect({
        sortedEntries: entries,
        cameraX: 200,
        cameraZoom: 1,
        viewportWidthPx: 800,
        minimapWidth: 220,
        inflatedRanges: ranges,
    });
    assert.deepEqual(a, b);
});

test('integration: Minimap.js scenario — at low zoom with slot-scale, rect spans visible neighbour', () => {
    // End-to-end scenario matching the qol-tray layout (8 layer-0 pages at
    // stride 10000 width 1280) at the zoom-pan combination that triggers
    // the bug — Plugins page on left half of screen, Plugin Store on right
    // half via CSS slot-scale inflation.
    const PAGE_STRIDE = 10000;
    const entries = ['plugins', 'store', 'hotkeys', 'shortcuts'].map((id, i) => ({
        id, x: i * PAGE_STRIDE, y: 0, width: 1280, height: 900,
    }));
    const minimapWidth = 380;
    const canvasHeight = 209;
    const viewportWidthPx = 1200;
    const viewportHeightPx = 720;
    const zoom = 0.12;
    const cameraX = 0;
    const cameraY = 0;
    const ghostThreshold = 0.55;
    const baseScale = computeBaseScale(zoom, ghostThreshold);
    assert.ok(baseScale > 1, `expected ghost mode at zoom ${zoom}, got baseScale ${baseScale}`);
    const inflatedRanges = entries.map(entry => inflatedEntryRange(entry, computeSlotScale({
        entry,
        cameraX,
        cameraY,
        viewportW: viewportWidthPx,
        viewportH: viewportHeightPx,
        zoom,
        baseScale,
    })));
    const rect = computeMinimapViewportRect({
        sortedEntries: entries,
        cameraX,
        cameraZoom: zoom,
        viewportWidthPx,
        minimapWidth,
        canvasHeight,
        inflatedRanges,
    });
    const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth, canvasHeight });
    // Rect covers Plugins slot...
    assert.ok(
        rect.x <= slots[0].x + slots[0].w * 0.5,
        `rect.x ${rect.x} should cover Plugins (slot.x=${slots[0].x})`,
    );
    // ...and reaches into Plugin Store slot.
    assert.ok(
        rect.x + rect.width >= slots[1].x + slots[1].w * 0.1,
        `rect end ${rect.x + rect.width} should reach into Store slot (starts at ${slots[1].x})`,
    );
});

// --- computeBaseScale + computeSlotScale: pure math used by both Minimap and App
test('computeBaseScale: above threshold zoom returns 1 (no inflation)', () => {
    assert.equal(computeBaseScale(0.6, 0.55), 1);
    assert.equal(computeBaseScale(1, 0.55), 1);
    assert.equal(computeBaseScale(8, 0.55), 1);
});

test('computeBaseScale: below threshold zoom returns >1 (inflation active)', () => {
    assert.ok(computeBaseScale(0.3, 0.55) > 1);
    assert.ok(computeBaseScale(0.1, 0.55) > 1);
    assert.ok(computeBaseScale(0.05, 0.55) > 1);
});

test('computeBaseScale: invalid inputs return 1 (no inflation)', () => {
    assert.equal(computeBaseScale(0, 0.55), 1);
    assert.equal(computeBaseScale(-1, 0.55), 1);
    assert.equal(computeBaseScale(0.3, 0), 1);
    assert.equal(computeBaseScale(0.3, -1), 1);
});

test('computeSlotScale: returns 1 when baseScale === 1', () => {
    const result = computeSlotScale({
        entry: { x: 0, y: 0, width: 1280, height: 900 },
        cameraX: 0, cameraY: 0, viewportW: 1200, viewportH: 720,
        zoom: 1, baseScale: 1,
    });
    assert.equal(result, 1);
});

test('computeSlotScale: returns 1 when entry has no width', () => {
    const result = computeSlotScale({
        entry: { x: 0, y: 0, width: 0, height: 900 },
        cameraX: 0, cameraY: 0, viewportW: 1200, viewportH: 720,
        zoom: 0.2, baseScale: 3,
    });
    assert.equal(result, 1);
});

test('computeSlotScale: closer to camera centre yields higher scale', () => {
    const opts = {
        cameraX: 0, cameraY: 0, viewportW: 1200, viewportH: 720,
        zoom: 0.2, baseScale: 3,
    };
    // Near-centre entry vs far-away entry. Camera centre = 0 + 1200/(2*0.2) = 3000.
    const near = computeSlotScale({
        ...opts,
        entry: { x: 2360, y: 0, width: 1280, height: 900 }, // centre ≈ 3000
    });
    const far = computeSlotScale({
        ...opts,
        entry: { x: 100000, y: 0, width: 1280, height: 900 }, // centre ≈ 100640
    });
    assert.ok(near > far, `near ${near} should exceed far ${far}`);
});

test('computeSlotScale: capped by maxSlotScale (PAGE_STRIDE * GAP_FACTOR / entry.width)', () => {
    // Tiny entry would otherwise inflate without bound. Clamp at the gap-size
    // so neighbouring slots don't visually collide.
    const result = computeSlotScale({
        entry: { x: 0, y: 0, width: 100, height: 900 },
        cameraX: 0, cameraY: 0, viewportW: 1200, viewportH: 720,
        zoom: 0.05, baseScale: 100,
    });
    const maxAllowed = (10000 * 0.85) / 100;
    assert.ok(result <= maxAllowed + 1e-9, `result ${result} should be capped at ${maxAllowed}`);
});

test('inflatedEntryRange: at slotScale=1 returns raw entry bounds', () => {
    const entry = { x: 100, y: 0, width: 1280, height: 900 };
    assert.deepEqual(inflatedEntryRange(entry, 1), { x0: 100, x1: 1380 });
});

test('inflatedEntryRange: at slotScale=k inflates around centre by k', () => {
    const entry = { x: 0, y: 0, width: 1000, height: 900 };
    // centre 500, half = 1000*2.5/2 = 1250.
    assert.deepEqual(inflatedEntryRange(entry, 2.5), { x0: -750, x1: 1750 });
});

test('inflatedEntryRange: zero-width entry passes through (no division by zero)', () => {
    const entry = { x: 100, y: 0, width: 0, height: 900 };
    assert.deepEqual(inflatedEntryRange(entry, 3), { x0: 100, x1: 100 });
});

test('property: at slotScale=1 for every entry, inflatedRanges equals raw bounds and rect matches legacy (200 cases)', () => {
    const rng = makeRng(0xDEAFBABE);
    for (let i = 0; i < 200; i++) {
        const count = 1 + Math.floor(rng() * 6);
        const entries = makeRandomEntries(rng, count);
        const worldMin = entries[0].x;
        const worldMax = entries.at(-1).x + entries.at(-1).width;
        const worldSpan = worldMax - worldMin;
        const cameraX = worldMin - worldSpan + rng() * worldSpan * 3;
        const cameraZoom = 0.5 + rng() * 4; // above threshold so baseScale=1
        const viewportWidthPx = 400 + rng() * 1600;
        const minimapWidth = 100 + rng() * 400;

        // baseScale=1 ⇒ all slotScales=1 ⇒ inflatedRanges == raw bounds.
        const ranges = entries.map(e => ({ x0: e.x, x1: e.x + e.width }));
        const legacy = computeMinimapViewportRect({
            sortedEntries: entries,
            cameraX, cameraZoom, viewportWidthPx, minimapWidth,
        });
        const inflated = computeMinimapViewportRect({
            sortedEntries: entries,
            cameraX, cameraZoom, viewportWidthPx, minimapWidth,
            inflatedRanges: ranges,
        });
        assert.deepEqual(inflated, legacy, `case ${i}: inflated path drifted from legacy`);
    }
});

test('property: when slot-scale inflates a neighbour into the camera window, rect crosses into that slot (200 cases)', () => {
    // For each random multi-page layout at low zoom:
    //   - compute baseScale, per-entry slotScale, and inflatedRanges
    //   - find any neighbour whose inflated range overlaps the camera window
    //     more than its raw range does (i.e., the bug scenario)
    //   - assert the rect's right edge crosses into that neighbour's slot.
    const rng = makeRng(0xFA1ED15E);
    let exercised = 0;
    for (let i = 0; i < 200; i++) {
        const count = 2 + Math.floor(rng() * 4);
        const entries = makeRandomEntries(rng, count);
        const minimapWidth = 200 + rng() * 200;
        const slots = computeMinimapSlots({ sortedEntries: entries, minimapWidth });
        const viewportWidthPx = 1000 + rng() * 400;
        const viewportHeightPx = 600 + rng() * 200;
        const ghostThreshold = 0.55;
        const zoom = 0.08 + rng() * 0.20; // below threshold → baseScale > 1
        const baseScale = computeBaseScale(zoom, ghostThreshold);
        const ranges = entries.map(entry => inflatedEntryRange(entry, computeSlotScale({
            entry,
            cameraX: 0, cameraY: 0,
            viewportW: viewportWidthPx, viewportH: viewportHeightPx,
            zoom, baseScale,
        })));
        // Camera window covers (only) the first entry: cam world width = vp/zoom
        const cameraX = entries[0].x;
        const camEnd = cameraX + viewportWidthPx / zoom;
        // Find a neighbour the camera window misses on raw, but inflated range overlaps.
        let neighbourIdx = -1;
        for (let j = 1; j < entries.length; j++) {
            const e = entries[j];
            const rawHits = cameraX < e.x + e.width && camEnd > e.x;
            const visHits = cameraX < ranges[j].x1 && camEnd > ranges[j].x0;
            if (visHits && !rawHits) { neighbourIdx = j; break; }
        }
        if (neighbourIdx < 0) continue;
        exercised++;
        const rect = computeMinimapViewportRect({
            sortedEntries: entries,
            cameraX, cameraZoom: zoom,
            viewportWidthPx, minimapWidth,
            inflatedRanges: ranges,
        });
        const neighbourSlot = slots[neighbourIdx];
        // The rect should reach at least into the neighbour's slot — even
        // a thin sliver. Otherwise the rect under-represents the camera
        // framing.
        assert.ok(
            rect.x + rect.width >= neighbourSlot.x - 1e-6,
            `case ${i}: rect end ${rect.x + rect.width} should reach neighbour slot at ${neighbourSlot.x}`,
        );
    }
    // Sanity: at least some cases triggered the bug scenario, otherwise the
    // test isn't covering anything.
    assert.ok(exercised > 0, `no cases exercised the slot-scale-inflation path; check zoom range`);
});

// --- drawViewportRect: pulse is plumbed through ----------------------------

test('drawViewportRect: passing a pulse increases stroke width above the baseline 2px', () => {
    const ctx = makeMockCtxWithPaths();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    let lastWidth = null;
    Object.defineProperty(ctx, 'lineWidth', {
        configurable: true,
        set(v) { lastWidth = v; },
        get() { return lastWidth; },
    });
    drawViewportRect(ctx, 200, 60, { x: 50, width: 40, y: 10, height: 40 },
        { progress: 0, alphaBoost: 0.2, blurExtra: 14, widthBoost: 1.5 });
    assert.ok(lastWidth > 2 + 1e-9, `lineWidth ${lastWidth} should exceed baseline 2 during pulse`);
});

test('drawViewportRect: with null pulse, stroke width stays at baseline 2px', () => {
    const ctx = makeMockCtxWithPaths();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    let lastWidth = null;
    Object.defineProperty(ctx, 'lineWidth', {
        configurable: true,
        set(v) { lastWidth = v; },
        get() { return lastWidth; },
    });
    drawViewportRect(ctx, 200, 60, { x: 50, width: 40, y: 10, height: 40 }, null);
    assert.ok(Math.abs(lastWidth - 2) < 1e-9, `lineWidth ${lastWidth} drifted from baseline 2 with null pulse`);
});

test('drawViewportRect: with omitted pulse arg, stroke width stays at baseline 2px (back-compat)', () => {
    const ctx = makeMockCtxWithPaths();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    let lastWidth = null;
    Object.defineProperty(ctx, 'lineWidth', {
        configurable: true,
        set(v) { lastWidth = v; },
        get() { return lastWidth; },
    });
    drawViewportRect(ctx, 200, 60, { x: 50, width: 40, y: 10, height: 40 });
    assert.ok(Math.abs(lastWidth - 2) < 1e-9, `lineWidth ${lastWidth} drifted with omitted pulse arg`);
});
