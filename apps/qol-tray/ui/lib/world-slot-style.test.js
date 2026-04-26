import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isSlotVisible, slotStyle } from './world-slot-style.js';

const baseEntry = { id: 'plugins', x: 0, y: 0, width: 1280, height: 900, layer: 0 };

test('content-sized entry omits inline height', () => {
    const e = { ...baseEntry, contentSized: true };
    const style = slotStyle(e, true);
    assert.equal(style.includes('height:'), false, `style was: ${style}`);
    assert.match(style, /left:0px/);
    assert.match(style, /top:0px/);
    assert.match(style, /width:1280px/);
});

test('non-content-sized entry pins inline height to entry.height', () => {
    const style = slotStyle(baseEntry, true);
    assert.match(style, /height:900px/);
});

test('a tall content-sized entry still emits no height (proves clipping cannot return)', () => {
    const tall = { ...baseEntry, height: 4500, contentSized: true };
    const style = slotStyle(tall, true);
    assert.equal(style.includes('height:'), false);
});

test('hidden slot adds display:none regardless of contentSized', () => {
    for (const cs of [true, false]) {
        const style = slotStyle({ ...baseEntry, contentSized: cs }, false);
        assert.match(style, /display:none/, `contentSized=${cs}`);
    }
});

test('isSlotVisible matches camera layer for layer-0 entry', () => {
    assert.equal(isSlotVisible(baseEntry, 0, [], 0), true);
    assert.equal(isSlotVisible(baseEntry, -1, [], 1), false);
});

test('isSlotVisible suppresses sub-page when not diving', () => {
    const sub = { ...baseEntry, id: 'hotkeys-editor', layer: -1 };
    assert.equal(isSlotVisible(sub, 0, [], 0), false);
    assert.equal(isSlotVisible(sub, -1, ['hotkeys-editor'], 1), true);
});

test('isSlotVisible respects confinedPages allowlist', () => {
    const a = { ...baseEntry, id: 'a' };
    const b = { ...baseEntry, id: 'b' };
    assert.equal(isSlotVisible(a, 0, ['a'], 0), true);
    assert.equal(isSlotVisible(b, 0, ['a'], 0), false);
});

test('property: slotStyle output never contains overflow or scrollbar tokens', () => {
    // Lock down the "no scrolling" invariant — slot styles must never
    // re-introduce overflow / scroll directives via inline CSS regression.
    let rng = 1;
    const next = () => {
        rng = (rng * 1664525 + 1013904223) >>> 0;
        return rng;
    };
    const banned = ['overflow', 'scroll', 'overflow-y', 'overflow-x'];
    for (let i = 0; i < 200; i++) {
        const e = {
            id: `e${i}`,
            x: (next() % 100000) - 50000,
            y: (next() % 100000) - 50000,
            width: 100 + (next() % 4000),
            height: 100 + (next() % 4000),
            layer: (next() % 5) - 2,
            contentSized: (next() % 2) === 1,
        };
        const visible = (next() % 2) === 1;
        const style = slotStyle(e, visible);
        for (const word of banned) {
            assert.equal(
                style.toLowerCase().includes(word),
                false,
                `iteration ${i}: style=${style} contained ${word}`,
            );
        }
    }
});
