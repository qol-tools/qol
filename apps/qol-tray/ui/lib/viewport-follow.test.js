import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    keyboardFollowDelta,
} from './viewport-follow.js';

const viewportRect = { left: 0, top: 0, width: 1000, height: 1000, right: 1000, bottom: 1000 };

test('keyboard follow centers the page, following the selection only when it leaves the viewport', () => {
    const cases = [
        ['centered page stays put when selection moves within it',
            { left: 100, top: 0, width: 800, height: 3000 },
            { left: 700, top: 520, width: 100, height: 100 }, 0],
        ['off-center page recenters on the page, not the selection',
            { left: 300, top: 0, width: 800, height: 3000 },
            { left: 350, top: 500, width: 100, height: 100 }, 200],
        ['selection past the right edge of a wide page pulls into view',
            { left: -1000, top: 0, width: 3000, height: 3000 },
            { left: 1400, top: 500, width: 100, height: 100 }, 540],
        ['selection past the left edge of a wide page pulls into view',
            { left: -1000, top: 0, width: 3000, height: 3000 },
            { left: -600, top: 500, width: 100, height: 100 }, -640],
        ['oversized selection centers instead of clipping',
            { left: -1000, top: 0, width: 3000, height: 3000 },
            { left: 325, top: 500, width: 950, height: 100 }, 300],
    ];
    for (const [name, pageRect, surfaceRect, wantDx] of cases) {
        const delta = keyboardFollowDelta(viewportRect, surfaceRect, pageRect);
        assert.equal(delta.dx, wantDx, name);
    }
});

test('keyboard follow keeps y inside the comfort band', () => {
    const pageRect = { left: 100, top: 0, width: 800, height: 3000 };
    const cases = [
        ['inside band', 520, 0],
        ['below band', 900, 210],
        ['above band', -100, -310],
    ];
    for (const [name, top, wantDy] of cases) {
        const delta = keyboardFollowDelta(viewportRect, { left: 450, top, width: 100, height: 100 }, pageRect);
        assert.equal(delta.dy, wantDy, name);
    }
});

test('keyboard follow without a page rect centers the selection', () => {
    const surfaceRect = { left: 700, top: 520, width: 100, height: 100 };
    const delta = keyboardFollowDelta(viewportRect, surfaceRect, null);
    assert.equal(delta.dx, 250);
    assert.equal(delta.dy, 0);
});
