import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeMinimapViewportRect } from './minimap-geometry.js';

const THREE_ENTRIES = [
    { id: 'a', x: 0,     y: 0, width: 1280, height: 900 },
    { id: 'b', x: 10000, y: 0, width: 1280, height: 900 },
    { id: 'c', x: 20000, y: 0, width: 1280, height: 900 },
];

const THREE_SLOTS = [
    { x: 0,   w: 50 },
    { x: 60,  w: 100 },
    { x: 170, w: 50 },
];

test('rect spans all slots when camera view covers every entry', () => {
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        slots: THREE_SLOTS,
        cameraX: 0,
        cameraZoom: 0.03,
        viewportWidthPx: 800,
    });
    assert.equal(rect.x, 0);
    assert.equal(rect.width, 220);
});

test('rect covers only the active slot when camera view is limited to one entry', () => {
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        slots: THREE_SLOTS,
        cameraX: 10000,
        cameraZoom: 1,
        viewportWidthPx: 800,
    });
    assert.equal(rect.x, 60);
    assert.equal(rect.width, 100);
});

test('rect spans two adjacent slots when camera straddles two entries', () => {
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        slots: THREE_SLOTS,
        cameraX: 500,
        cameraZoom: 0.05,
        viewportWidthPx: 800,
    });
    assert.equal(rect.x, 0);
    assert.equal(rect.width, 160);
});

test('rect is empty when camera is panned outside any entry', () => {
    const rect = computeMinimapViewportRect({
        sortedEntries: THREE_ENTRIES,
        slots: THREE_SLOTS,
        cameraX: 5000,
        cameraZoom: 1,
        viewportWidthPx: 800,
    });
    assert.equal(rect.x, 0);
    assert.equal(rect.width, 0);
});

test('rect is empty when inputs are missing or empty', () => {
    const rect1 = computeMinimapViewportRect({ sortedEntries: [], slots: THREE_SLOTS, cameraX: 0, cameraZoom: 1, viewportWidthPx: 800 });
    assert.deepEqual(rect1, { x: 0, width: 0 });
    const rect2 = computeMinimapViewportRect({ sortedEntries: THREE_ENTRIES, slots: [], cameraX: 0, cameraZoom: 1, viewportWidthPx: 800 });
    assert.deepEqual(rect2, { x: 0, width: 0 });
});
