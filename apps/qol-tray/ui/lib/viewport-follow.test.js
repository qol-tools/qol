import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    keyboardTargetCenterDelta,
} from './viewport-follow.js';

test('keyboardTargetCenterDelta centers x and leaves y inside comfort band', () => {
    const viewportRect = { left: 0, top: 0, width: 1000, height: 1000 };
    const surfaceRect = { left: 700, top: 520, width: 100, height: 100 };
    const delta = keyboardTargetCenterDelta(viewportRect, surfaceRect);
    assert.equal(delta.dx, 250);
    assert.equal(delta.dy, 0);
});
