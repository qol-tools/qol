import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    computePeripheralSlots,
    computeSiblingCoverage,
    handleSlotClick,
    pickCenteredEntry,
    shouldHidePeripheralSide,
    NEIGHBOR_HARD_CAP,
} from './peripheral-geometry.js';

test('computePeripheralSlots returns empty when anchor is null', () => {
    assert.deepEqual(computePeripheralSlots(null, ['a', 'b', 'c'], 1), []);
});

test('computePeripheralSlots returns empty when anchor is not in siblings', () => {
    assert.deepEqual(computePeripheralSlots('z', ['a', 'b', 'c'], 1), []);
});

test('computePeripheralSlots returns empty when maxNeighbors <= 0', () => {
    assert.deepEqual(computePeripheralSlots('b', ['a', 'b', 'c'], 0), []);
    assert.deepEqual(computePeripheralSlots('b', ['a', 'b', 'c'], -1), []);
});

test('computePeripheralSlots returns prev and next for an interior anchor at maxNeighbors=1', () => {
    const slots = computePeripheralSlots('c', ['a', 'b', 'c', 'd', 'e'], 1);
    assert.deepEqual(slots, [
        { id: 'b', side: 'prev', distance: 1 },
        { id: 'd', side: 'next', distance: 1 },
    ]);
});

test('computePeripheralSlots returns interleaved prev/next up to maxNeighbors', () => {
    const slots = computePeripheralSlots('c', ['a', 'b', 'c', 'd', 'e'], 2);
    assert.deepEqual(slots, [
        { id: 'b', side: 'prev', distance: 1 },
        { id: 'd', side: 'next', distance: 1 },
        { id: 'a', side: 'prev', distance: 2 },
        { id: 'e', side: 'next', distance: 2 },
    ]);
});

test('computePeripheralSlots emits null id when anchor is at the start of the strip', () => {
    const slots = computePeripheralSlots('a', ['a', 'b'], 1);
    assert.deepEqual(slots, [
        { id: null, side: 'prev', distance: 1 },
        { id: 'b', side: 'next', distance: 1 },
    ]);
});

test('computePeripheralSlots emits null id when anchor is at the end of the strip', () => {
    const slots = computePeripheralSlots('b', ['a', 'b'], 1);
    assert.deepEqual(slots, [
        { id: 'a', side: 'prev', distance: 1 },
        { id: null, side: 'next', distance: 1 },
    ]);
});

test('computePeripheralSlots caps at NEIGHBOR_HARD_CAP', () => {
    const siblings = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k'];
    const slots = computePeripheralSlots('f', siblings, 99);
    assert.equal(slots.length, NEIGHBOR_HARD_CAP * 2);
});

test('computeSiblingCoverage returns 1 when sibling is entirely inside viewport', () => {
    const sibling = { x: 100, y: 100, width: 50, height: 50 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const viewport = { w: 800, h: 600 };
    assert.equal(computeSiblingCoverage(sibling, camera, viewport), 1);
});

test('computeSiblingCoverage returns 0 when sibling is entirely outside viewport', () => {
    const sibling = { x: 2000, y: 2000, width: 50, height: 50 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const viewport = { w: 800, h: 600 };
    assert.equal(computeSiblingCoverage(sibling, camera, viewport), 0);
});

test('computeSiblingCoverage returns 0.5 for half-visible sibling', () => {
    const sibling = { x: 750, y: 0, width: 100, height: 600 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const viewport = { w: 800, h: 600 };
    assert.equal(computeSiblingCoverage(sibling, camera, viewport), 0.5);
});

test('computeSiblingCoverage accounts for zoom reducing the visible world rect', () => {
    const sibling = { x: 400, y: 400, width: 100, height: 100 };
    const cameraZoomedOut = { x: 0, y: 0, zoom: 1 };
    const cameraZoomedIn = { x: 0, y: 0, zoom: 4 };
    const viewport = { w: 800, h: 600 };
    assert.equal(computeSiblingCoverage(sibling, cameraZoomedOut, viewport), 1);
    assert.equal(computeSiblingCoverage(sibling, cameraZoomedIn, viewport), 0);
});

test('handleSlotClick calls navigation.gotoAnchor with the slot pageId and resetZoom', () => {
    const calls = [];
    const navigation = { gotoAnchor: (anchor, opts) => calls.push({ anchor, opts }) };
    handleSlotClick({ id: 'page-x', side: 'next', distance: 1 }, navigation);
    assert.deepEqual(calls, [{ anchor: { pageId: 'page-x' }, opts: { resetZoom: 1 } }]);
});

test('handleSlotClick is a no-op when slot id is null', () => {
    const calls = [];
    const navigation = { gotoAnchor: (anchor) => calls.push(anchor) };
    handleSlotClick({ id: null, side: 'prev', distance: 1 }, navigation);
    assert.deepEqual(calls, []);
});

test('pickCenteredEntry returns null for an empty list', () => {
    assert.equal(pickCenteredEntry([], { x: 0, y: 0, zoom: 1 }, { w: 800, h: 600 }), null);
});

test('pickCenteredEntry returns the only entry when list has one', () => {
    const e = { id: 'only', x: 1000, y: 1000, width: 100, height: 100 };
    assert.equal(pickCenteredEntry([e], { x: 0, y: 0, zoom: 1 }, { w: 800, h: 600 }), e);
});

test('pickCenteredEntry picks the entry whose center is closest to the viewport center', () => {
    const entries = [
        { id: 'a', x: 0, y: 0, width: 100, height: 100 },
        { id: 'b', x: 900, y: 0, width: 100, height: 100 },
        { id: 'c', x: 2000, y: 0, width: 100, height: 100 },
    ];
    const camera = { x: 500, y: -250, zoom: 1 };
    const result = pickCenteredEntry(entries, camera, { w: 800, h: 600 });
    assert.equal(result.id, 'b');
});

test('pickCenteredEntry reacts to camera pan (different center pick at different camera.x)', () => {
    const entries = [
        { id: 'a', x: 0, y: 0, width: 100, height: 100 },
        { id: 'b', x: 10000, y: 0, width: 100, height: 100 },
    ];
    const vp = { w: 800, h: 600 };
    const nearA = pickCenteredEntry(entries, { x: 0, y: 0, zoom: 1 }, vp);
    const nearB = pickCenteredEntry(entries, { x: 9600, y: 0, zoom: 1 }, vp);
    assert.equal(nearA.id, 'a');
    assert.equal(nearB.id, 'b');
});

test('pickCenteredEntry accounts for zoom when computing viewport center', () => {
    const entries = [
        { id: 'a', x: 0, y: 0, width: 100, height: 100 },
        { id: 'b', x: 200, y: 0, width: 100, height: 100 },
    ];
    const vp = { w: 800, h: 600 };
    const atZoom1 = pickCenteredEntry(entries, { x: 0, y: 0, zoom: 1 }, vp);
    const atZoom4 = pickCenteredEntry(entries, { x: 0, y: 0, zoom: 4 }, vp);
    assert.equal(atZoom1.id, 'b');
    assert.equal(atZoom4.id, 'a');
});

test('shouldHidePeripheralSide hides next when active right edge is past viewport right edge', () => {
    const activeEntry = { x: 600, y: 0, width: 500, height: 800 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const viewport = { w: 1000, h: 800 };
    assert.equal(shouldHidePeripheralSide({ side: 'next', activeEntry, camera, viewport }), true);
});

test('shouldHidePeripheralSide shows next when active right edge is well inside viewport', () => {
    const activeEntry = { x: 0, y: 0, width: 400, height: 800 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const viewport = { w: 1000, h: 800 };
    assert.equal(shouldHidePeripheralSide({ side: 'next', activeEntry, camera, viewport }), false);
});

test('shouldHidePeripheralSide hides prev when active left edge is past viewport left edge', () => {
    const activeEntry = { x: -100, y: 0, width: 400, height: 800 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const viewport = { w: 1000, h: 800 };
    assert.equal(shouldHidePeripheralSide({ side: 'prev', activeEntry, camera, viewport }), true);
});

test('shouldHidePeripheralSide shows prev when active left edge is well inside viewport', () => {
    const activeEntry = { x: 200, y: 0, width: 400, height: 800 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const viewport = { w: 1000, h: 800 };
    assert.equal(shouldHidePeripheralSide({ side: 'prev', activeEntry, camera, viewport }), false);
});

test('shouldHidePeripheralSide boundary: hides next at exactly viewport.w - hysteresisPx', () => {
    const hysteresisPx = 16;
    const viewport = { w: 1000, h: 800 };
    const camera = { x: 0, y: 0, zoom: 1 };
    const activeEntry = { x: 0, y: 0, width: viewport.w - hysteresisPx, height: 800 };
    assert.equal(shouldHidePeripheralSide({ side: 'next', activeEntry, camera, viewport, hysteresisPx }), true);
});

test('shouldHidePeripheralSide zoom: higher zoom makes same entry span more screen pixels', () => {
    const activeEntry = { x: 0, y: 0, width: 600, height: 800 };
    const viewport = { w: 1000, h: 800 };
    const cameraZoom1 = { x: 0, y: 0, zoom: 1 };
    const cameraZoom2 = { x: 0, y: 0, zoom: 2 };
    assert.equal(shouldHidePeripheralSide({ side: 'next', activeEntry, camera: cameraZoom1, viewport }), false);
    assert.equal(shouldHidePeripheralSide({ side: 'next', activeEntry, camera: cameraZoom2, viewport }), true);
});
