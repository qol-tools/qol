import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    edgeFollowDelta,
    surfaceCenterDelta,
} from './viewport-follow.js';

const viewportRect = { left: 0, top: 0, width: 1000, height: 1000, right: 1000, bottom: 1000 };

test('surfaceCenterDelta centers x and leaves y inside comfort band', () => {
    const cases = [
        ['off-center right', { left: 700, top: 520, width: 100, height: 100 }, 250, 0],
        ['below comfort band', { left: 450, top: 900, width: 100, height: 100 }, 0, 210],
        ['above comfort band', { left: 450, top: -100, width: 100, height: 100 }, 0, -310],
    ];
    for (const [name, surfaceRect, wantDx, wantDy] of cases) {
        const delta = surfaceCenterDelta(viewportRect, surfaceRect);
        assert.equal(delta.dx, wantDx, `${name}: dx`);
        assert.equal(delta.dy, wantDy, `${name}: dy`);
    }
});

test('edgeFollowDelta only reacts near viewport edges', () => {
    const cases = [
        ['inside pads', { left: 400, top: 400, right: 500, bottom: 500, width: 100, height: 100 }, 0, 0],
        ['past right pad', { left: 900, top: 400, right: 1000, bottom: 500, width: 100, height: 100 }, 40, 0],
        ['past bottom pad', { left: 400, top: 900, right: 500, bottom: 1000, width: 100, height: 100 }, 0, 40],
        ['past left pad', { left: -20, top: 400, right: 80, bottom: 500, width: 100, height: 100 }, -60, 0],
    ];
    for (const [name, surfaceRect, wantDx, wantDy] of cases) {
        const delta = edgeFollowDelta(viewportRect, surfaceRect);
        assert.equal(delta.dx, wantDx, `${name}: dx`);
        assert.equal(delta.dy, wantDy, `${name}: dy`);
    }
});
