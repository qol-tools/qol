import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    PAGE_TOP_PAD_PX,
    boundsOfEntries,
    cameraTargetFor,
    maxEntryExtent,
    viewportPadding,
    withPadding,
} from './world-geometry.js';

function seededRng(seed) {
    let s = seed >>> 0;
    return () => {
        s = (s * 1664525 + 1013904223) >>> 0;
        return s / 0x100000000;
    };
}

test('viewportPadding: half-viewport in world units at zoom 1', () => {
    const entries = [{ x: 0, y: 0, width: 100, height: 100 }];
    const { padX, padY } = viewportPadding({ w: 800, h: 600 }, 1, entries);
    assert.equal(padX, 400);
    assert.equal(padY, 300);
});

test('viewportPadding: divides by zoom for world-unit conversion', () => {
    const entries = [{ x: 0, y: 0, width: 100, height: 100 }];
    const atTwoZoom = viewportPadding({ w: 800, h: 600 }, 2, entries);
    assert.equal(atTwoZoom.padX, 200);
    assert.equal(atTwoZoom.padY, 150);
    const atHalfZoom = viewportPadding({ w: 800, h: 600 }, 0.5, entries);
    assert.equal(atHalfZoom.padX, 800);
    assert.equal(atHalfZoom.padY, 600);
});

test('viewportPadding: allows any page center to reach viewport center', () => {
    const vp = { w: 800, h: 600 };
    const zoom = 1;
    const entries = [{ x: 0, y: 0, width: 1280, height: 900 }];
    const { padX, padY } = viewportPadding(vp, zoom, entries);
    const pageCenterX = 640;
    const pageCenterY = 450;
    const cameraX = pageCenterX - vp.w / (2 * zoom);
    const cameraY = pageCenterY - vp.h / (2 * zoom);
    assert.ok(cameraX >= -padX, 'camera.x reachable from left');
    assert.ok(cameraY >= -padY, 'camera.y reachable from top');
});

test('viewportPadding: falls back to maxEntryExtent when viewport is 0', () => {
    const entries = [{ x: 0, y: 0, width: 1280, height: 900 }];
    const { padX, padY } = viewportPadding({ w: 0, h: 0 }, 1, entries);
    assert.equal(padX, 1280);
    assert.equal(padY, 900);
});

test('viewportPadding: falls back when viewport is missing', () => {
    const entries = [{ x: 0, y: 0, width: 1280, height: 900 }];
    const { padX, padY } = viewportPadding(null, 1, entries);
    assert.equal(padX, 1280);
    assert.equal(padY, 900);
});

test('viewportPadding: falls back when zoom is 0', () => {
    const entries = [{ x: 0, y: 0, width: 1280, height: 900 }];
    const { padX, padY } = viewportPadding({ w: 800, h: 600 }, 0, entries);
    assert.equal(padX, 1280);
    assert.equal(padY, 900);
});

test('viewportPadding: falls back when zoom is negative', () => {
    const entries = [{ x: 0, y: 0, width: 1280, height: 900 }];
    const { padX, padY } = viewportPadding({ w: 800, h: 600 }, -1, entries);
    assert.equal(padX, 1280);
    assert.equal(padY, 900);
});

test('viewportPadding: falls back when viewport width alone is 0', () => {
    const entries = [{ x: 0, y: 0, width: 1280, height: 900 }];
    const { padX, padY } = viewportPadding({ w: 0, h: 600 }, 1, entries);
    assert.equal(padX, 1280);
    assert.equal(padY, 900);
});

test('viewportPadding: property — when viewport is valid, result is independent of entries', () => {
    const rng = seededRng(42);
    for (let i = 0; i < 200; i += 1) {
        const w = 100 + rng() * 2000;
        const h = 100 + rng() * 2000;
        const z = 0.1 + rng() * 7.9;
        const entries = [
            { x: rng() * 1000, y: rng() * 1000, width: rng() * 2000, height: rng() * 2000 },
            { x: rng() * 1000, y: rng() * 1000, width: rng() * 2000, height: rng() * 2000 },
        ];
        const a = viewportPadding({ w, h }, z, entries);
        const b = viewportPadding({ w, h }, z, []);
        assert.equal(a.padX, b.padX, `padX independent of entries (case ${i})`);
        assert.equal(a.padY, b.padY, `padY independent of entries (case ${i})`);
        assert.equal(a.padX, w / (2 * z), `padX matches formula (case ${i})`);
        assert.equal(a.padY, h / (2 * z), `padY matches formula (case ${i})`);
    }
});

test('viewportPadding: property — padding scales inversely with zoom', () => {
    const rng = seededRng(7);
    for (let i = 0; i < 200; i += 1) {
        const w = 100 + rng() * 2000;
        const h = 100 + rng() * 2000;
        const z1 = 0.1 + rng() * 4;
        const z2 = z1 * (0.5 + rng() * 3);
        const a = viewportPadding({ w, h }, z1, []);
        const b = viewportPadding({ w, h }, z2, []);
        const ratio = z1 / z2;
        const ax = a.padX * ratio;
        const ay = a.padY * ratio;
        assert.ok(Math.abs(ax - b.padX) < 1e-9, `padX scales with 1/zoom (case ${i})`);
        assert.ok(Math.abs(ay - b.padY) < 1e-9, `padY scales with 1/zoom (case ${i})`);
    }
});

test('maxEntryExtent: returns max width and height across entries', () => {
    const { padX, padY } = maxEntryExtent([
        { x: 0, y: 0, width: 100, height: 200 },
        { x: 0, y: 0, width: 300, height: 150 },
        { x: 0, y: 0, width: 50, height: 400 },
    ]);
    assert.equal(padX, 300);
    assert.equal(padY, 400);
});

test('maxEntryExtent: returns zero for empty entries', () => {
    const { padX, padY } = maxEntryExtent([]);
    assert.equal(padX, 0);
    assert.equal(padY, 0);
});

test('boundsOfEntries: computes axis-aligned bounding box', () => {
    const r = boundsOfEntries([
        { x: 10, y: 20, width: 100, height: 50 },
        { x: -30, y: 5, width: 40, height: 200 },
    ]);
    assert.equal(r.x, -30);
    assert.equal(r.y, 5);
    assert.equal(r.width, 140);
    assert.equal(r.height, 200);
});

test('boundsOfEntries: returns null for empty entries', () => {
    assert.equal(boundsOfEntries([]), null);
});

test('withPadding: expands a rect uniformly on both axes', () => {
    const r = withPadding({ x: 0, y: 0, width: 100, height: 100, layer: 0 }, 20, 30);
    assert.equal(r.x, -20);
    assert.equal(r.y, -30);
    assert.equal(r.width, 140);
    assert.equal(r.height, 160);
    assert.equal(r.layer, 0);
});

test('cameraTargetFor: x centers the entry, y top-aligns it under PAGE_TOP_PAD_PX', () => {
    const c = cameraTargetFor({ x: 100, y: 100, width: 200, height: 100 }, 800, 600, 1);
    assert.equal(c.x, 200 - 400);
    assert.equal(c.y, 100 - PAGE_TOP_PAD_PX);
});

test('cameraTargetFor: padding shrinks proportionally with zoom', () => {
    const c = cameraTargetFor({ x: 0, y: 0, width: 100, height: 100 }, 800, 600, 2);
    assert.equal(c.x, 50 - 200);
    assert.equal(c.y, 0 - PAGE_TOP_PAD_PX / 2);
});

test('cameraTargetFor: page top lands at the same screen-Y for any height (no vertical bounce)', () => {
    const rng = seededRng(0x9c0f3611);
    for (let i = 0; i < 200; i++) {
        const zoom = 0.25 + rng() * 3.75;
        const vpW = 320 + Math.floor(rng() * 1600);
        const vpH = 240 + Math.floor(rng() * 1200);
        const entryY = Math.floor(rng() * 5000);
        const entryH = 80 + Math.floor(rng() * 4000);
        const entry = { x: 0, y: entryY, width: 800, height: entryH };
        const cam = cameraTargetFor(entry, vpW, vpH, zoom);
        const screenTop = (entry.y - cam.y) * zoom;
        assert.ok(
            Math.abs(screenTop - PAGE_TOP_PAD_PX) < 1e-6,
            `screenTop=${screenTop} should equal PAGE_TOP_PAD_PX=${PAGE_TOP_PAD_PX} (case ${i})`,
        );
    }
});

test('cameraTargetFor: x centers the entry horizontally regardless of width (property)', () => {
    const rng = seededRng(0x123abc99);
    for (let i = 0; i < 200; i++) {
        const zoom = 0.25 + rng() * 3.75;
        const vpW = 320 + Math.floor(rng() * 1600);
        const vpH = 240 + Math.floor(rng() * 1200);
        const entryX = Math.floor(rng() * 5000);
        const entryW = 80 + Math.floor(rng() * 4000);
        const entry = { x: entryX, y: 0, width: entryW, height: 600 };
        const cam = cameraTargetFor(entry, vpW, vpH, zoom);
        const entryScreenCenter = (entry.x + entry.width / 2 - cam.x) * zoom;
        assert.ok(
            Math.abs(entryScreenCenter - vpW / 2) < 1e-6,
            `entryScreenCenter=${entryScreenCenter} should equal vpW/2=${vpW / 2} (case ${i})`,
        );
    }
});
